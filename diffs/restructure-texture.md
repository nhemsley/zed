# Texture Windows — Implementation Plan

> Status: milestones 1–3 landed on branch `textured` (see "Steps / milestones"
> for what each delivered and where it deviates from the text below).
> Supersedes the preliminary notes. The four branch summaries in `diffs/` are
> Blade-era and kept **for reference only**; none of their code applies.

## Scope

Wayland only for the first round. X11 / macOS / Windows get a
`Platform::open_texture_window` default that returns an error, nothing more.
Everything below the `gpui` core is written so X11 can be enabled later by
one small addition in `gpui_linux/src/linux/platform.rs`.

## Goal

An application can open an arbitrary number of **texture windows** alongside
its real windows. Each texture window:

- renders through the normal GPUI layout/prepaint/paint pipeline into an
  offscreen GPU texture instead of a compositor surface;
- is an independent input target: it has its own focus, hover, hit-testing,
  IME, actions, and receives `PlatformInput` addressed to it;
- is displayed inside a host (real) window by a `TextureView` element, which
  is also the thing that forwards input to it;
- is only re-laid-out / re-rendered when dirty (the "render + cache" mode),
  or drawn once and parked (the "oneshot" mode).

## What the survey found

### The repo moved from under the old diffs

| Old (Blade branches)                          | Now                                                |
|-----------------------------------------------|----------------------------------------------------|
| `crates/gpui/src/platform/blade/*`            | `crates/gpui_wgpu/src/wgpu_renderer.rs`, `wgpu_atlas.rs`, `wgpu_context.rs` |
| `crates/gpui/src/platform/linux/*`            | `crates/gpui_linux/src/linux/{wayland,x11,headless}/*` |
| `blade_graphics::Context` per window          | `GpuContext = Rc<RefCell<Option<WgpuContext>>>` shared per client (`gpui_linux/src/linux/wayland/client.rs:316`) |

### The hard part already landed

- `WgpuRenderer::record_frame(&mut self, scene, frame_view: &wgpu::TextureView)`
  (`gpui_wgpu/src/wgpu_renderer.rs:1405`) is the target-agnostic batch loop that
  `gpui-render-to-texture` extracted as `render_batches`. The only remaining
  swapchain coupling is in `draw()` (`:1265`): `surface.get_current_texture()`,
  and `GlobalParams.viewport_size` / `premultiplied_alpha` derived from
  `surface_config`.
- `PlatformHeadlessRenderer` trait exists (`gpui/src/platform.rs:1023`) with
  `render_scene`, `render_scene_to_image`, `sprite_atlas`. Only Metal
  implements it. `TestWindow` (`gpui/src/platform/test/window.rs:430`) shows a
  `PlatformWindow` whose `draw` just calls `renderer.render_scene(scene, size)`.
- `PlatformWindow::render_to_image` / `Window::render_to_image` exist but are
  `cfg(test-support)`.
- `PrimitiveBatch::Surfaces` is a no-op in wgpu (`wgpu_renderer.rs` ~`:1530`);
  `PaintSurface` carries a macOS `CVPixelBuffer` only (`gpui/src/scene.rs:768`).

### `Window` is already the addressable input unit

- Input enters via `PlatformWindow::on_input(PlatformInput) -> DispatchEventResult`
  (`gpui/src/window.rs:1893`) → `Window::dispatch_event` (`:5203`), which
  updates mouse position/modifiers and runs hit-test, hover, focus, key/action
  dispatch and IME against **that window's** `Frame` (`hitboxes`,
  `mouse_listeners`, `dispatch_tree`, `focus`, `input_handlers`).
- `App.windows: SlotMap<WindowId, Option<Box<Window>>>` (`gpui/src/app.rs:694`).
  Nothing requires a `Window` to own an OS surface (`HeadlessWindow`,
  `TestWindow`).
- `Window::draw` runs only when `invalidator.is_dirty() || force_render`
  (`window.rs:1779`); otherwise the frame is re-presented as-is. That *is* the
  "render + cache, skip layout when clean" mode.

Conclusion: do not fan input out inside one `Window`. **Make each texture a
real GPUI `Window` backed by a texture-backed `PlatformWindow`.** Everything
in "Open questions §1/§4" of the old notes is then already solved by existing
code.

### Compositor interaction is unaffected

Wayland is strictly 1 `wl_surface` ↔ 1 `WaylandWindow` ↔ 1 `WgpuRenderer`
owning a `wgpu::Surface` ↔ 1 frame callback (`wayland/window.rs:919-1020`,
`:1901`). Texture windows have no `wl_surface`, no frame callback, no
`xdg_toplevel`. They are paced by their host window's frame. `client.rs` is
untouched.

## Design

```
host Window (WaylandWindow)                  texture Window (TextureWindow)
┌───────────────────────────────┐            ┌─────────────────────────────┐
│ on_request_frame              │            │ Frame: hitboxes, focus, …   │
│   ├─ draw dirty texture_children ──────────► Window::draw → present     │
│   │                           │            │   └─ TextureWindow::draw    │
│   └─ Window::draw (host)      │            │        └─ render_to_view    │
│        TextureView element    │            │             into wgpu::Texture
│          paint: PolychromeSprite ◄─── atlas external texture id ───┘    │
│          hitbox + listeners   │            │                             │
│            └─ dispatch_event ─────────────►│ Window::dispatch_event      │
└───────────────────────────────┘            └─────────────────────────────┘
```

### 1. `gpui_wgpu`: target-agnostic core + headless renderer

Files: `wgpu_renderer.rs`, `wgpu_atlas.rs`, `gpui_wgpu.rs`.

- Factor `draw()` so the surface-independent part is callable on its own:

  ```rust
  fn render_to_view(
      &mut self,
      scene: &Scene,
      target: &wgpu::TextureView,
      size: Size<DevicePixels>,
      premultiplied_alpha: bool,
  ) -> Result<()>          // writes GlobalParams/GammaParams, ensure_intermediate_textures, record_frame
  ```

  `draw()` becomes: acquire swapchain texture → `render_to_view` → `present`.
- Make `WgpuResources.surface` an `Option<wgpu::Surface<'static>>` (or split
  into `SurfaceTarget` / `OffscreenTarget` enum). `unconfigure_surface`,
  `replace_surface`, `recover` guard on it. Intermediate/MSAA textures are
  sized from a `target_size` field rather than `surface_config`.
- New `WgpuHeadlessRenderer` (same file, or `wgpu_headless_renderer.rs` if it
  grows): owns a `WgpuRenderer` with no surface plus an
  `OffscreenTarget { texture: wgpu::Texture, view, size, format }`. Format =
  `WgpuContext::color_texture_format()`, usage
  `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`.

  ```rust
  impl PlatformHeadlessRenderer for WgpuHeadlessRenderer {
      fn render_scene(&mut self, scene, size) -> Result<()>;          // resize target if needed, render_to_view
      fn render_scene_to_image(&mut self, scene, size) -> Result<RgbaImage>; // render_scene + copy_texture_to_buffer + map (row pitch 256-aligned)
      fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas>;
  }
  impl WgpuHeadlessRenderer {
      pub fn new(gpu_context: GpuContext, size: Size<DevicePixels>) -> Result<Self>;
      pub fn target_view(&self) -> &wgpu::TextureView;
      pub fn target_size(&self) -> Size<DevicePixels>;
  }
  ```

  Shares `device`/`queue`/`WgpuAtlas` with the host via `GpuContext`
  (`WgpuAtlas::from_context`). No threads, no GPU mutex.
- `PlatformHeadlessRenderer` is currently `cfg(test|test-support|bench-support)`
  in `gpui/src/platform.rs:1022`. Lift the cfg (trait stays `#[doc(hidden)]`).
- **Atlas external textures** (for zero-copy compositing, step 4):

  ```rust
  impl WgpuAtlas {
      pub fn register_external_texture(&self, view: wgpu::TextureView, size: Size<DevicePixels>) -> AtlasTile; // kind: Polychrome, index from a separate range
      pub fn update_external_texture(&self, tile: &AtlasTile, view: wgpu::TextureView, size: Size<DevicePixels>);
      pub fn unregister_external_texture(&self, tile: &AtlasTile);
  }
  ```

  `WgpuAtlasStorage` gains `external: Vec<Option<WgpuAtlasTexture-like>>`;
  `get_texture_info` (`wgpu_atlas.rs:77`) resolves either range. `clear()` /
  `handle_device_lost()` must drop external entries too (owners re-register
  after recovery). Sampler is the shared linear `atlas_sampler`, fine for 1:1;
  downscaling quality is out of scope.

This step is independently useful: it gives Linux visual tests and benches a
real headless renderer, so it can be its own PR.

### 2. `gpui_linux`: `TextureWindow: PlatformWindow`

File: `gpui_linux/src/linux/texture_window.rs` (backend-agnostic; not under
`wayland/`). Registered from `linux.rs`.

- State: `callbacks` (same shape as `WaylandWindow`'s `Callbacks`:
  `request_frame`, `input`, `active_status_change`, `hover_status_change`,
  `resize`, `close`, …), `bounds`, `scale_factor`, `appearance`, `active`,
  `input_handler: Option<PlatformInputHandler>`, `renderer: WgpuHeadlessRenderer`,
  `atlas_tile: Option<AtlasTile>`, `host: Option<Weak<…>>` / a
  `Rc<dyn Fn()>` waker supplied by the host.
- `draw(scene)`: `renderer.render_scene(scene, device_size)`, then
  `update_external_texture` if the target was recreated by a resize.
- `resize(size)`: update bounds, recreate the target on next draw, and call
  the `resize` callback **synchronously** (the `textured-view` branch hit stale
  `viewport_size` because nothing else will ever fire it).
- `frame_waker()` / `schedule_frame()`: call the host waker (see step 3).
- `sprite_atlas()`: the shared `WgpuAtlas`.
- Everything compositor-shaped (`activate`, `minimize`, `zoom`,
  `toggle_fullscreen`, `set_title`, `prompt`, decorations, IME position) is a
  no-op or stored-only, as in `HeadlessWindow`
  (`gpui_linux/src/linux/headless/window.rs`).
- Public, non-trait accessors for the `TextureView` element:
  `handle_input(PlatformInput) -> DispatchEventResult` (mirrors
  `wayland/window.rs:1514` minus the shift-keychar fallback),
  `set_focused(bool)`, `set_hovered(bool)`, `atlas_tile()`, `take_input_handler`.
- `Platform` (`gpui_linux/src/linux/platform.rs`) and `LinuxClient` gain
  `open_texture_window(handle, params) -> Result<Box<dyn PlatformWindow>>`;
  Wayland client constructs it with `state.gpu_context.clone()`, X11 client
  returns `Err` for now.

### 3. `gpui` core: opening, driving, and hosting texture windows

Files: `gpui/src/platform.rs`, `app.rs`, `window.rs`, new
`gpui/src/elements/texture_view.rs`.

**Opening**

```rust
// platform.rs
fn open_texture_window(&self, handle: AnyWindowHandle, options: WindowParams)
    -> Result<Box<dyn PlatformWindow>> { bail!("texture windows are not supported on this platform") }

// app.rs
pub fn open_texture_window<V: Render>(
    &mut self,
    options: TextureWindowOptions { size: Size<Pixels>, scale_factor: f32, appearance, … },
    build_root: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<WindowHandle<V>>;
```

`Window::new` takes a `WindowKind`-style flag (or `Window::new_texture`) so it
calls `platform.open_texture_window` instead of `open_window` and skips
`map_window`, app-id, and tab-controller wiring. The rest of `Window::new` —
`on_request_frame`, `on_input`, `on_resize`, invalidator, focus map — is
reused unchanged; that reuse is the whole point.

**Driving frames**

`Window` gets `pub(crate) texture_children: Vec<AnyWindowHandle>` (added by
`TextureView` when it first paints, removed on drop). In the host's
`on_request_frame` closure (`window.rs:1779`), before `window.draw(cx)`:

```rust
for child in handle.read(&cx, |window| window.texture_children.clone()) {
    child.update(&mut cx, |_, window, cx| {
        if window.invalidator.is_dirty() || force_render {
            let clear = window.draw(cx);
            window.present();
            clear.clear(cx);
        }
    }).log_err();
}
```

Children must be drawn *before* the host draw, never from inside the host's
paint: `Window::draw` returns `ArenaClearNeeded` and nested draws would share
the element arena. The child's `frame_waker` is the host's
`invalidator.wake_platform()`, so a child `cx.notify()` schedules a host frame.
Texture windows never call `platform_window.schedule_frame()` on themselves
beyond that.

**Hosting: user-facing primitives**

User code (an infinite canvas, a dock, a 3D panel) is the compositor. It gets
four primitives and owns the transform, the z-order and the input policy. See
[`texture-window-example.rs`](texture-window-example.rs) for a two-child
canvas that is itself a texture painted by a real window.

```rust
// Open. Child is a full Window: own root view, focus, hover, hit-test,
// dispatch tree, element state. Unattached until something paints it.
App::open_texture_window<V: Render>(&mut self, TextureWindowOptions, build_root) -> Result<WindowHandle<V>>

// Composite. Samples the child's texture into `bounds` (translate + scale),
// and records `child ↔ this host` for the current frame. Painting IS
// attachment: the frame hook draws children painted last frame, and a
// child's cx.notify() wakes the hosts that painted it last frame.
Window::paint_texture_window(&mut self, child: &AnyWindowHandle, bounds: Bounds<Pixels>, cx: &mut App)

// Route input. Already public; host remaps `position` into child space first.
Window::dispatch_event(&mut self, PlatformInput, cx: &mut App) -> DispatchEventResult

// Focus. Drives the child's `active` flag / focus-lost path, since no
// compositor will ever tell a texture window it was (de)activated.
Window::set_texture_active(&mut self, active: bool, cx: &mut App)

// Size. Already public. For texture windows this is synchronous: updates
// viewport_size, invokes the resize callback, marks the window dirty.
Window::resize(&mut self, Size<Pixels>)
```

Host repaint after a child changes needs no subscription: when the frame hook
draws a dirty child it marks every host that painted it dirty, so the host
re-paints and re-samples in the same frame. An optional
`cx.observe_texture_window(child, cb)` can be added later for consumers that
want the pixels (thumbnails, readback) rather than a repaint.

A `TextureView` element wrapping these for the "docked rectangle" case is a
convenience for later, not part of the core (see appendix).

**What the example forces on the implementation**

- *Attachment by painting.* `paint_texture_window` appends to
  `Window.texture_children` for the frame being painted; the child keeps a
  `hosts: SmallVec<AnyWindowHandle>` refreshed the same way. A child painted
  by no host in a frame is simply not drawn (parked), and is closed only when
  the owner closes it.
- *Depth-first frame order.* In the example the real window paints the canvas
  (a texture window), which paints A and B. The real window's frame hook must
  draw A and B before the canvas, and the canvas before itself:
  `draw_texture_children(handle)` recurses on each child's own
  `texture_children` first. Cycles are a programming error; guard with a
  visited set and `debug_assert`.
- *Wake-up chain.* A's `cx.notify()` → A dirty → A's `frame_waker` wakes its
  hosts (canvas) → canvas has no real waker, so it forwards to *its* hosts →
  real window's `invalidator.wake_platform()`. `TextureWindow::frame_waker`
  therefore has to resolve to "wake all current hosts", not a fixed pointer.
- *Uniform forwarding.* The same `dispatch_event` call is used by the real
  window (identity remap) and the canvas (pan/zoom remap). No special
  "top-level" path.
- *Mouse policy lives in user code:* `MouseExited` when the hovered child
  changes, mouse-up delivered to the child that received mouse-down even if
  the cursor left it, modifier changes broadcast to all children. GPUI does
  none of this automatically for texture windows; document it.
- *Focus is explicit.* `set_texture_active(true)` on mouse-down inside a child,
  `false` on the previous one. Keyboard events go only to the active child.
  The real window's own `FocusHandle` on the canvas element is what makes key
  events arrive at all.
- *Zoom is free.* Sprite bounds scaling; children stay cached. Downscale
  quality is a later concern (mip/linear sampler).
- *Resize before draw.* `Main` resizes the canvas texture window in its paint;
  because `resize` is synchronous and dirties the child, the child is
  re-laid-out at the new size on the *next* frame hook pass. Accept one frame
  of stale size, or have the frame hook run a second child pass if any child
  was resized during host paint (decide in milestone 3).
- *`AnyWindowHandle::update` from inside another window's mouse listener* is
  the only access path. Fine as long as the host is never re-entered.

### 4. Rendering modes

- **Render + cache** — default. Child is only re-drawn when its invalidator
  is dirty (`cx.notify()`, focus change, resize, force_render after device
  recovery). Otherwise the host just re-samples the existing texture. No new
  invalidation machinery is needed.
- **Oneshot** — open a texture window, run one `draw`+`present` (or
  `Window::render_to_image` once its `cfg(test-support)` gate is lifted for
  this path), then either keep it parked as a static texture or close it.
  Expose `Window::draw_once(cx)` as a thin public wrapper if callers outside
  the frame loop need it.

Input to a parked/cached window works unchanged: `dispatch_event` runs against
its `rendered_frame` and any `cx.notify()` it triggers dirties it for the next
host frame.

## Steps / milestones

1. **`gpui_wgpu`** — *done.* `render_to_view` factoring, optional surface,
   `WgpuHeadlessRenderer` implementing `PlatformHeadlessRenderer`, trait cfg
   lifted, wired into `gpui_platform::current_headless_renderer` on Linux
   with a red-box visual test. Atlas external textures deferred to 4.
2. **`gpui_linux`** — *done.* `TextureWindow`, `Platform::open_texture_window`
   (Wayland; X11 errors). Smoke test draws, reads back, and writes a PNG when
   `GPUI_TEXTURE_WINDOW_PNG_DIR` is set. Deviation: the platform takes
   `TextureWindowOptions` (size, scale factor, background) rather than
   `WindowParams`, so the child's scale factor is set at open time.
3. **`gpui` core, milestone A** — *done.* `App::open_texture_window`,
   `Window::paint_texture_window` (readback → `RenderImage` → `paint_image`),
   `Window::set_texture_active`, synchronous `Window::resize`, depth-first
   frame driving from the host's `on_request_frame`, and
   `examples/texture_windows.rs`. Deviations from the design text above:
   - Attachment persists once painted (until the child closes) instead of
     being re-derived every frame: cached views replay their paint without
     re-running it, so "painted last frame" is not observable.
   - A dirty child marks its hosts dirty directly (through the child's
     invalidator waker) rather than only waking their frame source, and the
     host marks the views that painted the child dirty so the new frame is
     re-uploaded instead of replayed from the view cache.
   - Children that still want a frame after being drawn (animations via
     `on_next_frame`) make the host re-arm its own frame source.
   - `TestPlatform` opens texture windows through its headless renderer, and
     `HeadlessAppContext::simulate_frame` drives a host's frame loop, so the
     mechanics are tested without a GPU (`gpui`) and the pixels with one
     (`gpui_platform` Linux tests).
4. **`gpui` core, milestone B**: atlas external textures; swap readback for
   the zero-copy `PolychromeSprite` path. IME proxy and cursor-style relay
   helpers for hosts that want them.
5. Cleanup: `render_to_image` gating, docs, `Application::headless()` parity,
   X11 `open_texture_window` if trivial. Known gap: headless adapter selection
   does not validate pipeline creation, so a GL adapter without vertex storage
   buffers panics at first draw (`LIBGL_ALWAYS_SOFTWARE=1` works around it).

## Alternatives considered

- **Fan-out inside one `Window`** (a texture as a child render target with its
  own hit-test/focus scope). Requires a second `Frame`, dispatch tree and
  focus stack per texture — a reimplementation of `Window`. Rejected.
- **Separate `Application` per texture on background threads**
  (`textured-view`). Needs the main-thread assert disabled, hit Vulkan driver
  crashes that were papered over with a global mutex, no shared atlas, no
  input path. Rejected.
- **Readback + `img()` for compositing.** Simple and portable; one GPU→CPU→GPU
  copy per changed frame. Kept as milestone A, superseded by B.
- **Linux `PaintSurface` variant holding a wgpu texture.** Semantically the
  cleanest "external texture" primitive, but touches `Scene` batching, adds a
  pipeline, and `PaintSurface` is `cfg(macos)`-shaped today. Revisit if atlas
  aliasing turns out to be awkward (e.g. sampler/format mismatches).
- **`wl_subsurface` per texture.** Compositor does input routing and frame
  pacing for free and each child is a plain `WaylandWindow`. But it is a
  surface, not a texture: no scaling, rotation, canvas or 3D use, and
  Wayland-only by construction. Only appropriate if the real use case is
  docked rectangles.

## Risks / things to verify while implementing

- Re-entrancy: `TextureView` mouse listeners run inside the host
  `Window::dispatch_event` (host `Window` is taken out of `App.windows`);
  updating a *different* window there is fine, updating the host again is a
  panic. Keep all child access going through `child.update`.
- `Window::new` currently unconditionally calls `map_window()` and registers
  tab-controller callbacks; the texture path must skip those without forking
  the function.
- `WgpuAtlas::clear()` on GPU error / recovery (`wgpu_renderer.rs:1300`)
  drops tiles; external entries need re-registration on the child's next draw.
- Coordinates: child and host share `scale_factor` by construction
  (`TextureWindowOptions.scale_factor` should default to the host's). Mixed
  scale factors are unsupported in round one.
- `premultiplied_alpha`: render the child target premultiplied and make the
  polychrome sprite path expect it (check `fs_polychrome_sprite` blending).
- `is_active` / `is_hovered` on the child are driven purely by the
  `TextureView`; nothing else will ever call them.

## Appendix: `TextureView` convenience element (later)

For the common "one child, docked, 1:1" case, an element that owns a
`FocusHandle`, paints the child into its own bounds, forwards mouse/key/IME,
relays cursor style and emits `MouseExited` — i.e. the policy the example's
`Canvas` implements by hand. Built entirely on the primitives above; no core
changes.

## Out of scope

X11, macOS, Windows, web; mixed scale factors; downscaled-texture quality;
gpuix (the `/tmp/gpuix` checkout referenced earlier was empty — re-point when
available).

## Reference (historical, Blade-era, do not port)

- [`gitui-render-to-texture/structure.md`](gitui-render-to-texture/structure.md)
- [`gpui-render-to-texture/structure.md`](gpui-render-to-texture/structure.md) — its `render_batches` idea is what `record_frame` now is.
- [`gpui-oneshot-render/structure.md`](gpui-oneshot-render/structure.md)
- [`textured-view/structure.md`](textured-view/structure.md) — its resize/`viewport_size` and repaint-wakeup fixes map onto `TextureWindow::resize` and `frame_waker` above.
