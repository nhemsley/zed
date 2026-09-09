# `textured-view` — structure

16 commits, diffed against `origin/main`. Shares its base with `gpui-oneshot-render`
(the first 7 commits and every file in
[`diffs/gpui-oneshot-render/structure.md`](../gpui-oneshot-render/structure.md) are
identical between the two branches) and adds 9 more commits on top that turn
the `textured_surface` platform from example-driven scaffolding into a reusable
public `TexturedView` component, plus a batch of fixes discovered while making
it actually work as a `View`. This section covers only what's new *beyond*
`gpui-oneshot-render`; see that branch's `structure.md` for the shared base
(the `TexturedSurfaceClient`/`Window`/`Display`, `Application::textured()`,
`Window::read_pixels`, and the original examples/research).

## `crates/gpui/src/textured_view.rs` *(new, 811 lines — the centerpiece)*

Promotes the pattern demonstrated ad hoc in `gpui-oneshot-render`'s
`multi_app_textured.rs` example (spawn a background thread running
`Application::textured()`, stream frames over a channel, display via `img()`)
into a proper, reusable `View`:

- **`TexturedView<F>`** — the view itself; owns the `render_fn` closure, the
  background `JoinHandle`, sizing/render-mode config, and the latest decoded
  frame as an `Arc<RenderImage>`. Constructors: `fixed(size, ...)` (exact
  dimensions, most performant), `measured(width, ...)` / `measured_with_estimate`
  (fixed width, height measured from content via `layout_as_root` before the
  real render), and `streaming(size, fps, ...)` (continuous re-render loop).
  `with_options` is the shared underlying constructor that spawns the
  background thread.
- **`ItemSizing`** enum (`Fixed`, `FixedWidth { width, estimated_height }`,
  `Explicit`) and **`RenderMode`** enum (`Once` default, `Streaming { target_fps }`
  — doc-commented as experimental, "for now use `RenderMode::Once`").
- **`TextureError`** enum (`UnsupportedPlatform`, `GpuInitFailed`, `ThreadDied`,
  `RenderPanic`) surfaced via `TexturedView::error()`, and an `is_ready()` /
  `texture()` accessor pair for callers that want the raw `RenderImage`.
- **`BackgroundRenderer<F>`**, a private helper type driving the actual
  background-thread loop that owns a `TexturedSurfaceWindow`, calls the
  caller's `render_fn`, and pushes `RenderedFrame { pixels, width, height }`
  values (BGRA, matching the GPU/atlas format — no conversion) over a `flume`
  channel back to the view.
- Explicitly Linux/FreeBSD-only (matching `Application::textured()`'s
  availability); on other platforms constructing a `TexturedView` renders an
  error placeholder instead of failing to compile.

## Fixes to the shared `gpui-oneshot-render` base

These are changes to files `textured-view` inherited from `gpui-oneshot-render`,
made while turning the prototype into something a `View` could drive
repeatedly and concurrently:

- **`platform/linux/textured_surface/mod.rs`** — adds a process-global
  `GPU_CONTEXT_MUTEX` and `acquire_gpu_lock()`, taken around both
  `TexturedSurfaceWindow::new` and `draw()`
  ([window.rs](worktree://bdda8758-c8bc-4b73-a2dc-aa856829eb4f/crates/gpui/src/platform/linux/textured_surface/window.rs)).
  The commit message accompanying this (`00ffa33337`) is candid about it being
  a workaround, not a fix: "acquire lock to serialize gpu access, fixes
  getting segfaults in the vulkan driver. we are obviously doing some
  questionable things." This matters because `TexturedView` is designed to be
  used many-at-once (see the stress-test example below), each with its own
  background thread and its own `gpu::Context` — concurrent Vulkan context
  creation/use across those threads was crashing the NVIDIA driver.
- **`platform/linux/textured_surface/window.rs`** — `resize()` now explicitly
  invokes the `on_resize` callback after resizing GPU textures (previously it
  only resized the textures), and **`window.rs`**'s `Window::resize` now
  synchronously updates `viewport_size` from the platform window's
  `content_size()` instead of waiting for an async resize callback. Both are
  fixing the same root cause: for a real window the compositor eventually
  fires a resize event that updates `viewport_size`, but a `TexturedSurfaceWindow`
  has no compositor, so without these changes `draw_roots()` would lay out
  content at a stale size after `ItemSizing::FixedWidth`'s measure-then-resize
  step.
- **`platform/linux/textured_surface/client.rs`** — minor `futures::channel::oneshot`
  vs. bare `oneshot` crate fix for `screen_capture_sources`'s return type.
- **`gpui.rs`** — registers the new module and re-exports `ItemSizing`,
  `RenderMode`, `TextureError`, `TexturedView`.

## `crates/gpui/examples/` (new, on top of `gpui-oneshot-render`'s three)

- **`textured_view.rs`** — demonstrates the three sizing/render modes
  (`fixed`, `measured`, `streaming`) directly through the new `TexturedView`
  API, replacing the manual thread/channel wiring the earlier examples had to
  do by hand.
- **`textured_view_streaming.rs`** — streaming mode with a live FPS slider,
  used to validate/exercise `RenderMode::Streaming`.
- **`textured_view_stress_test.rs`** (671 lines) — deliberately hammers the
  system: many simultaneous streaming `TexturedView`s, rapid create/destroy
  cycles, varying frame rates, to surface race conditions and the driver
  crashes that motivated the `GPU_CONTEXT_MUTEX` workaround above. Its module
  doc explicitly warns each view spawns its own thread + GPU context and that
  creating too many too fast can exhaust system resources.

## `crates/gpui/research/*.md` (new, on top of `gpui-oneshot-render`'s five)

A second wave of design/investigation docs specific to making `TexturedView`
work as a live, repeatedly-updating `View`, roughly in the order problems were
found and fixed:

- **`textured_view_design.md`** — the design doc matching `textured_view.rs`
  as built (View vs. Element rationale, the three constructors).
- **`gpui_async_rendering_investigation.md`** — diagnoses the bug where
  `cx.notify()` from a background-thread frame doesn't repaint the UI unless
  the user is already interacting (panning/scrolling) — the event loop simply
  isn't woken by a channel send.
- **`texture_rendering_lifecycle_analysis.md`** — records that investigation's
  resolution: replace timer polling with an async receiver task that wakes the
  executor, fix an unintended BGRA↔RGBA swap, and replace a
  compositor-frame-callback assumption (which doesn't exist for
  `TexturedSurface`) with a real continuous render loop for streaming mode.
  Matches the commit history's "Replace timer polling with async receiver",
  "dont munge the texture", and "Fix TexturedView color channels and streaming
  mode" commits.
- **`texture-rendering-roadmap-todo.md`** — a short status/TODO snapshot taken
  once those fixes landed.
- **`textured_canvas_roadmap.md`** — roadmap for a downstream consumer (an
  "infinite canvas" crate), noting `RenderMode::Streaming` was still broken as
  of writing and recommending `RenderMode::Once` until "Phase 4.4".
- **`scaling-images.md`** — a more self-contained note on downscaling
  syntax-highlighted text/code textures without the colors washing out —
  relevant to any canvas/thumbnail consumer of `TexturedView` output, but not
  wired into the code in this branch.

## Overall shape

`textured-view` is `gpui-oneshot-render` matured from "a new platform backend
plus hand-written thread/channel example code" into a packaged, documented
`View` type with real bug fixes discovered under load (resize/layout staleness,
repaint wake-up, color channel swap, driver crashes under concurrent GPU
context use). The GPU-context mutex remains an acknowledged workaround rather
than a resolved concurrency design, and `RenderMode::Streaming` is documented
in multiple places as not fully reliable — both flagged directly in the
commits/docs themselves, not something this summary is inferring.
