//! Sketch of how an application consumes the texture-window API proposed in
//! `restructure-texture.md`. Not compiled; names are proposals.
//!
//! Topology:
//!
//!   real window ──paints──► Canvas (texture window C)
//!                              ├──paints──► Counter "A" (texture window)
//!                              └──paints──► Counter "B" (texture window)
//!
//! Events flow the same way in reverse at every level: a host remaps the
//! event into the child's coordinate space and calls `dispatch_event` on it.
//! Frames flow root-first: the real window's frame hook draws dirty texture
//! children depth-first (A, B, then C) before drawing itself.

use gpui::*;

// ---------------------------------------------------------------------------
// Leaf content: an ordinary view. It has no idea it lives in a texture.
// ---------------------------------------------------------------------------

struct Counter {
    label: SharedString,
    count: i32,
    focus_handle: FocusHandle,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .bg(rgb(0x202030))
            .hover(|style| style.bg(rgb(0x303048)))
            .child(format!("{}: {}", self.label, self.count))
            .on_click(cx.listener(|this, _, _, cx| {
                this.count += 1;
                cx.notify(); // dirties this texture window only
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "backspace" {
                    this.count = 0;
                    cx.notify();
                }
            }))
    }
}

// ---------------------------------------------------------------------------
// Canvas: composites N texture windows with pan + zoom, and routes input.
// The canvas is itself opened as a texture window (see `main`), but nothing
// in here depends on that; it would work identically as a real window's root.
// ---------------------------------------------------------------------------

struct CanvasItem {
    window: AnyWindowHandle,
    /// Position in canvas space (unzoomed).
    origin: Point<Pixels>,
    /// Logical size of the child; the child window is kept at this size.
    size: Size<Pixels>,
}

struct Canvas {
    items: Vec<CanvasItem>,
    pan: Point<Pixels>,
    zoom: f32,
    hovered: Option<usize>,
    focused: Option<usize>,
    focus_handle: FocusHandle,
}

impl Canvas {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Result<Self> {
        let mut items = Vec::new();
        for (index, label) in ["A", "B"].into_iter().enumerate() {
            let size = size(px(320.), px(200.));
            // Proposed: `App::open_texture_window`. The child is a full GPUI
            // `Window` (own focus, hover, hit-test, dispatch tree, element
            // state). It has no host yet; painting it attaches it.
            let child = cx.open_texture_window(
                TextureWindowOptions {
                    size,
                    scale_factor: window.scale_factor(),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| Counter {
                        label: label.into(),
                        count: 0,
                        focus_handle: cx.focus_handle(),
                    })
                },
            )?;
            items.push(CanvasItem {
                window: child.into(),
                origin: point(px(40. + 400. * index as f32), px(40.)),
                size,
            });
        }
        Ok(Self {
            items,
            pan: point(px(0.), px(0.)),
            zoom: 1.0,
            hovered: None,
            focused: None,
            focus_handle: cx.focus_handle(),
        })
    }

    /// Host-space bounds of an item under the current pan/zoom.
    fn item_bounds(&self, item: &CanvasItem, canvas_origin: Point<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: canvas_origin + self.pan + item.origin * self.zoom,
            size: item.size * self.zoom,
        }
    }

    /// Host-space point → child-space point.
    fn to_child(&self, item_bounds: Bounds<Pixels>, position: Point<Pixels>) -> Point<Pixels> {
        (position - item_bounds.origin) / self.zoom
    }

    fn item_at(&self, canvas_origin: Point<Pixels>, position: Point<Pixels>) -> Option<usize> {
        // Topmost last; iterate in reverse so the last painted wins.
        self.items.iter().enumerate().rev().find_map(|(index, item)| {
            self.item_bounds(item, canvas_origin).contains(&position).then_some(index)
        })
    }

    fn forward(&self, index: usize, input: PlatformInput, cx: &mut App) -> DispatchEventResult {
        // `Window::dispatch_event` already exists and is public. Updating a
        // different window from inside the host's own dispatch is allowed;
        // updating the host again is not.
        self.items[index]
            .window
            .update(cx, |_, child_window, cx| child_window.dispatch_event(input, cx))
            .log_err()
            .unwrap_or_default()
    }

    fn set_focused(&mut self, index: Option<usize>, cx: &mut App) {
        if self.focused == index {
            return;
        }
        if let Some(previous) = self.focused {
            self.items[previous]
                .window
                .update(cx, |_, child_window, cx| child_window.set_texture_active(false, cx))
                .log_err();
        }
        if let Some(next) = index {
            self.items[next]
                .window
                .update(cx, |_, child_window, cx| child_window.set_texture_active(true, cx))
                .log_err();
        }
        self.focused = index;
    }
}

impl Render for Canvas {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(0x101010))
            .child(canvas(
                |_, _, _| (),
                cx.processor(|this, _, bounds: Bounds<Pixels>, window, cx| {
                    this.paint(bounds, window, cx)
                }),
            ).size_full())
            // Keyboard: whatever the canvas decides is focused gets the keys.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if let Some(index) = this.focused {
                    let result = this.forward(index, PlatformInput::KeyDown(event.clone()), cx);
                    if !result.propagate {
                        cx.stop_propagation();
                    }
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if let Some(index) = this.focused {
                    this.forward(index, PlatformInput::KeyUp(event.clone()), cx);
                }
            }))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                for index in 0..this.items.len() {
                    this.forward(index, PlatformInput::ModifiersChanged(event.clone()), cx);
                }
            }))
    }
}

impl Canvas {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();

        for item in &self.items {
            let item_bounds = self.item_bounds(item, bounds.origin);
            // Proposed: `Window::paint_texture_window`.
            //  * samples the child's texture into `item_bounds` (translate +
            //    scale; zoom is just bounds scaling of the sprite);
            //  * records `child ↔ this host` for this frame, which is what
            //    makes the host's frame hook draw the child and lets the
            //    child's `cx.notify()` wake this host;
            //  * does nothing about input — that's below.
            window.paint_texture_window(&item.window, item_bounds, cx);
        }

        // Mouse routing. Bubble phase so elements painted on top of the
        // canvas (toolbar, selection handles) can claim the event first.
        let canvas_origin = bounds.origin;

        window.on_mouse_event({
            let this = this.clone();
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                this.update(cx, |canvas, cx| {
                    let hit = canvas.item_at(canvas_origin, event.position);
                    if canvas.hovered != hit {
                        if let Some(previous) = canvas.hovered {
                            canvas.forward(
                                previous,
                                PlatformInput::MouseExited(MouseExitedEvent {
                                    position: event.position,
                                    pressed_button: event.pressed_button,
                                    modifiers: event.modifiers,
                                }),
                                cx,
                            );
                        }
                        canvas.hovered = hit;
                    }
                    if let Some(index) = hit {
                        let item_bounds = canvas.item_bounds(&canvas.items[index], canvas_origin);
                        let mut event = event.clone();
                        event.position = canvas.to_child(item_bounds, event.position);
                        canvas.forward(index, PlatformInput::MouseMove(event), cx);
                    }
                })
                .log_err();
            }
        });

        window.on_mouse_event({
            let this = this.clone();
            move |event: &MouseDownEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                this.update(cx, |canvas, cx| {
                    let hit = canvas.item_at(canvas_origin, event.position);
                    canvas.set_focused(hit, cx);
                    let Some(index) = hit else { return };
                    let item_bounds = canvas.item_bounds(&canvas.items[index], canvas_origin);
                    let mut event = event.clone();
                    event.position = canvas.to_child(item_bounds, event.position);
                    let result = canvas.forward(index, PlatformInput::MouseDown(event), cx);
                    if !result.propagate {
                        cx.stop_propagation();
                    }
                })
                .log_err();
            }
        });

        window.on_mouse_event({
            let this = this.clone();
            move |event: &MouseUpEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                this.update(cx, |canvas, cx| {
                    // Deliver mouse-up to the child that got the mouse-down,
                    // even if the cursor has left it (drag semantics).
                    let Some(index) = canvas.focused else { return };
                    let item_bounds = canvas.item_bounds(&canvas.items[index], canvas_origin);
                    let mut event = event.clone();
                    event.position = canvas.to_child(item_bounds, event.position);
                    canvas.forward(index, PlatformInput::MouseUp(event), cx);
                })
                .log_err();
            }
        });

        window.on_mouse_event({
            let this = this.clone();
            move |event: &ScrollWheelEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                this.update(cx, |canvas, cx| {
                    if event.modifiers.control {
                        // Zoom the canvas itself; children are untouched and
                        // stay cached — only the sprite bounds change.
                        canvas.zoom = (canvas.zoom * 1.1f32.powf(event.delta.pixel_delta(px(1.)).y.0 / 20.)).clamp(0.1, 8.);
                        cx.notify();
                        return;
                    }
                    if let Some(index) = canvas.item_at(canvas_origin, event.position) {
                        let item_bounds = canvas.item_bounds(&canvas.items[index], canvas_origin);
                        let mut event = event.clone();
                        event.position = canvas.to_child(item_bounds, event.position);
                        canvas.forward(index, PlatformInput::ScrollWheel(event), cx);
                    } else {
                        canvas.pan = canvas.pan + event.delta.pixel_delta(px(1.));
                        cx.notify();
                    }
                })
                .log_err();
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Real window: paints the canvas texture edge to edge and forwards input 1:1.
// Exists to show that the host side is the same code at every level.
// ---------------------------------------------------------------------------

struct Main {
    canvas: AnyWindowHandle,
}

impl Render for Main {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let canvas = self.canvas;
        div().size_full().child(
            canvas(
                |_, _, _| (),
                move |_, bounds: Bounds<Pixels>, window, cx| {
                    // Keep the canvas texture the size of the real window.
                    // `resize` on a texture window is synchronous and marks
                    // it dirty, so it re-lays-out before this frame's draw.
                    canvas
                        .update(cx, |_, canvas_window, _| {
                            if canvas_window.viewport_size() != bounds.size {
                                canvas_window.resize(bounds.size);
                            }
                        })
                        .log_err();

                    window.paint_texture_window(&canvas, bounds, cx);

                    // Identity remap: pass everything through.
                    for_each_mouse_event_kind(window, move |input, cx| {
                        canvas
                            .update(cx, |_, canvas_window, cx| canvas_window.dispatch_event(input, cx))
                            .log_err()
                    });
                },
            )
            .size_full(),
        )
        // key events would be forwarded the same way via on_key_down etc.
    }
}

fn main() {
    Application::new().run(|cx| {
        let main_window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let canvas = cx
                    .open_texture_window(
                        TextureWindowOptions {
                            size: window.viewport_size(),
                            scale_factor: window.scale_factor(),
                            ..Default::default()
                        },
                        |window, cx| cx.new(|cx| Canvas::new(window, cx).expect("gpu")),
                    )
                    .expect("texture windows unsupported on this platform");
                cx.new(|_| Main { canvas: canvas.into() })
            })
            .expect("window");
        main_window.update(cx, |_, window, _| window.activate_window()).ok();
    });
}
