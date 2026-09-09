# `gitui-render-to-texture` — structure

Single commit ("Render to texture, with example"), diffed against `origin/main`.
Smallest and simplest of the four render-to-texture spikes: adds a `RenderTarget`
texture abstraction directly to the Blade renderer and a standalone example that
renders a view to a PNG. No streaming, no window integration beyond drawing a
render target back into the normal frame.

## `crates/gpui/src/`

- **`gpui.rs`** — adds the public API surface: `RenderTargetId` (an opaque `u32`
  handle), `RenderTargetTexture` (owns a `blade_graphics::Texture` + view +
  sampler + size), and a `RenderTarget` trait with three methods
  (`create_render_target`, `render_to_texture`, `draw_render_target`) that a
  platform renderer can implement. Also wires the new `texture_renderer` module
  into the crate (`mod`/`pub use`).
- **`texture_renderer.rs`** *(new)* — a small `TextureRenderer` façade meant to
  be the entry point from example/application code: `TextureRenderer::new()`
  spins up a headless `Application`, and `render_to_png` is supposed to render
  a `Render` view of a given size to a PNG file. In this commit the body is a
  placeholder — it writes a synthetic gradient bitmap rather than actually
  invoking the view/scene/renderer pipeline — and `begin_session` explicitly
  returns "not yet implemented". Also adds a `WindowRenderExt::capture_to_png`
  extension trait on `Window`, similarly stubbed with gradient output instead
  of real pixel readback. This is the one piece of the branch that is scaffolding
  rather than working code.

## `crates/gpui/src/platform/`

- **`blade.rs`** — re-exports a free function `read_pixels_from_render_target`
  that forwards to `BladeRenderer::read_pixels_from_render_target`, so platform
  callers don't need to reach into the `blade_renderer` submodule directly.
- **`blade/blade_renderer.rs`** — the real implementation, all on `BladeRenderer`:
  - `render_targets: HashMap<RenderTargetId, RenderTargetTexture>` and a
    `next_render_target_id` counter added to the struct; targets are destroyed
    alongside other GPU resources in `destroy()`.
  - `create_render_target(size)` allocates a `TARGET | RESOURCE` texture (same
    format as the window surface) plus a view and a linear sampler, and stores
    it under a fresh id.
  - `render_to_texture(target_id, scene)` rasterizes paths, then calls the new
    `render_to_target_id` → `render_scene_to_target` helper, which re-implements
    the same per-`PrimitiveBatch` draw loop used for the main window (quads,
    shadows, paths, underlines, mono/poly sprites, macOS video surfaces) but
    targets the offscreen texture's `RenderTargetSet` instead of the swapchain.
    This duplicates rather than shares the window's draw loop.
  - `draw_render_target(target_id, bounds, content_mask)` draws a previously
    rendered target texture into the *real* swapchain frame, using a new
    `render_target` pipeline/shader pair so a render-target texture can be
    composited into an on-screen window like any other sprite.
  - `read_pixels_from_render_target_raw` / `read_pixels_from_render_target`
    copy the texture into a `Shared`-memory staging buffer, wait on the GPU,
    and read back raw RGBA bytes, resizing with `image::imageops::resize` if
    the caller asked for a different size than the target's native size.
  - `BladeRenderer` implements the crate-level `RenderTarget` trait by
    delegating to these same-named inherent methods.
  - New shader plumbing: a `ShaderRenderTargetData` bind-group struct and a
    `render_target` entry in `BladePipelines`, built from a new `fs_render_target`
    fragment shader appended to **`shaders.wgsl`** that samples the render
    target texture and alpha-blends it like the existing surface shader does.

## `crates/gpui/`

- **`Cargo.toml`** — turns on the `png` feature of the `image` crate (needed for
  `img.save(...)` in `texture_renderer.rs`) and registers the new
  `render_to_texture` example.
- **`examples/render_to_texture.rs`** *(new)* — a CLI example: takes an output
  path argument, builds a `TextureRenderer`, and calls `render_to_png` with a
  simple `ExampleView` (a `div()` tree with some text). Because
  `TextureRenderer::render_to_png` is stubbed, running this example currently
  produces a synthetic gradient PNG rather than an actual render of
  `ExampleView` — the renderer plumbing in `blade_renderer.rs` exists but isn't
  yet wired up to this entry point.

## Overall shape

The GPU-side mechanism (offscreen texture, duplicate draw loop, readback,
compositing back into a window) is real and self-contained inside
`blade_renderer.rs`/`blade.rs`/`shaders.wgsl`. The public-facing API
(`TextureRenderer`, `WindowRenderExt`) is a thin, unfinished shell on top of it
that doesn't yet call into that mechanism — the example runs but doesn't
exercise the new Blade code path at all.
