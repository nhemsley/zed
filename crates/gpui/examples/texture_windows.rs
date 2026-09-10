//! Texture windows: GPUI windows that render into an offscreen texture and
//! are composited, and driven, by another window.
//!
//! Topology:
//!
//!   real window ──paints──► Canvas (texture window)
//!                              ├──paints──► Counter "A" (texture window)
//!                              └──paints──► Counter "B" (texture window)
//!
//! Events flow the same way at every level: a host remaps the event into the
//! child's coordinate space and calls `Window::dispatch_event` on it. Frames
//! flow root-first: the real window's frame loop draws dirty texture children
//! depth-first (A, B, then the canvas) before drawing itself.
//!
//! Wayland only for now; other platforms fail at `open_texture_window`.

#[path = "example_support/fonts.rs"]
mod example_support;

use gpui::{
    AnyWindowHandle, App, Bounds, Context, DispatchEventResult, DispatchPhase, FocusHandle,
    Focusable, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent, MouseDownEvent, MouseExitEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, ScrollWheelEvent, SharedString,
    Size, TextureWindowOptions, Window, WindowBounds, WindowOptions, canvas, div, point,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;
use gpui_util::ResultExt as _;

/// Leaf content: an ordinary view that has no idea it lives in a texture.
struct Counter {
    label: SharedString,
    count: i32,
    focus_handle: FocusHandle,
}

impl Render for Counter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = window.is_window_active();
        div()
            .id("counter")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .text_color(rgb(0xe0e0e0))
            .bg(if active { rgb(0x2a2a48) } else { rgb(0x202030) })
            .hover(|style| style.bg(rgb(0x303058)))
            .child(format!("{}: {}", self.label, self.count))
            .child(div().text_sm().child("click: +1, backspace: reset"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.count += 1;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "backspace" {
                    this.count = 0;
                    cx.notify();
                }
            }))
    }
}

struct CanvasItem {
    window: AnyWindowHandle,
    /// Position in canvas space, before zoom.
    origin: Point<Pixels>,
    /// Logical size of the child; the child window is kept at this size.
    size: Size<Pixels>,
}

/// Composites N texture windows with pan and zoom and routes input to them.
/// The canvas is itself opened as a texture window (see `main`), but nothing
/// in here depends on that; it would work identically as a real window's root.
struct Canvas {
    items: Vec<CanvasItem>,
    pan: Point<Pixels>,
    zoom: f32,
    hovered: Option<usize>,
    focused: Option<usize>,
    focus_handle: FocusHandle,
}

impl Canvas {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> anyhow::Result<Self> {
        let mut items = Vec::new();
        for (index, label) in ["A", "B"].into_iter().enumerate() {
            let size = size(px(320.), px(200.));
            let child = cx.open_texture_window(
                TextureWindowOptions {
                    size,
                    scale_factor: window.scale_factor(),
                    ..TextureWindowOptions::default()
                },
                |_, cx| {
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

    /// Host-space bounds of an item under the current pan and zoom.
    fn item_bounds(&self, item: &CanvasItem, canvas_origin: Point<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: canvas_origin + self.pan + item.origin * self.zoom,
            size: item.size.map(|length| length * self.zoom),
        }
    }

    fn to_child(&self, item_bounds: Bounds<Pixels>, position: Point<Pixels>) -> Point<Pixels> {
        (position - item_bounds.origin) / self.zoom
    }

    fn item_at(&self, canvas_origin: Point<Pixels>, position: Point<Pixels>) -> Option<usize> {
        // Topmost last; iterate in reverse so the last painted wins.
        self.items
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, item)| {
                self.item_bounds(item, canvas_origin)
                    .contains(&position)
                    .then_some(index)
            })
    }

    fn forward(&self, index: usize, input: PlatformInput, cx: &mut App) -> DispatchEventResult {
        self.items[index]
            .window
            .update(cx, |_, child_window, cx| {
                child_window.dispatch_event(input, cx)
            })
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
                .update(cx, |_, child_window, cx| {
                    child_window.set_texture_active(false, cx)
                })
                .log_err();
        }
        if let Some(next) = index {
            self.items[next]
                .window
                .update(cx, |_, child_window, cx| {
                    child_window.set_texture_active(true, cx)
                })
                .log_err();
        }
        self.focused = index;
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        for item in &self.items {
            let item_bounds = self.item_bounds(item, bounds.origin);
            window.paint_texture_window(&item.window, item_bounds, cx);
        }

        let this = cx.entity().downgrade();
        let canvas_origin = bounds.origin;

        // Mouse routing lives in user code: hover transitions, delivering
        // mouse-up to the child that got mouse-down, and remapping positions.
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
                                PlatformInput::MouseExited(MouseExitEvent {
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
            move |event: &ScrollWheelEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                this.update(cx, |canvas, cx| {
                    let delta = event.delta.pixel_delta(px(20.));
                    if event.modifiers.control {
                        // Zoom only changes sprite bounds; children stay cached.
                        canvas.zoom =
                            (canvas.zoom * 1.1f32.powf(delta.y.as_f32() / 20.)).clamp(0.1, 8.);
                        cx.notify();
                        return;
                    }
                    if let Some(index) = canvas.item_at(canvas_origin, event.position) {
                        let item_bounds = canvas.item_bounds(&canvas.items[index], canvas_origin);
                        let mut event = event.clone();
                        event.position = canvas.to_child(item_bounds, event.position);
                        canvas.forward(index, PlatformInput::ScrollWheel(event), cx);
                    } else {
                        canvas.pan = canvas.pan + delta;
                        cx.notify();
                    }
                })
                .log_err();
            }
        });
    }
}

impl Focusable for Canvas {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Canvas {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(0x101010))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        this.update(cx, |canvas, cx| canvas.paint(bounds, window, cx))
                            .log_err();
                    },
                )
                .size_full(),
            )
            // Keyboard: whatever the canvas decided is focused gets the keys.
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

/// The real window: paints the canvas texture edge to edge and forwards
/// input one to one. The host side is the same code at every level.
struct Main {
    canvas: AnyWindowHandle,
    focus_handle: FocusHandle,
}

impl Main {
    fn forward(&self, input: PlatformInput, cx: &mut App) -> DispatchEventResult {
        self.canvas
            .update(cx, |_, canvas_window, cx| {
                canvas_window.dispatch_event(input, cx)
            })
            .log_err()
            .unwrap_or_default()
    }
}

impl Render for Main {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let canvas_handle = self.canvas;
        let this = cx.entity().downgrade();

        // The canvas owns the keyboard while this window is active.
        let active = window.is_window_active();
        canvas_handle
            .update(cx, |_, canvas_window, cx| {
                canvas_window.set_texture_active(active, cx)
            })
            .log_err();

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        // Keep the canvas texture the size of this window.
                        // `resize` is synchronous for texture windows and
                        // dirties the child, so it re-lays-out at the new
                        // size before the next frame's draw.
                        canvas_handle
                            .update(cx, |_, canvas_window, _| {
                                if canvas_window.viewport_size() != bounds.size {
                                    canvas_window.resize(bounds.size);
                                }
                            })
                            .log_err();

                        window.paint_texture_window(&canvas_handle, bounds, cx);

                        let origin = bounds.origin;
                        window.on_mouse_event({
                            let this = this.clone();
                            move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let mut event = event.clone();
                                event.position = event.position - origin;
                                this.update(cx, |main, cx| {
                                    main.forward(PlatformInput::MouseMove(event), cx);
                                })
                                .log_err();
                            }
                        });
                        window.on_mouse_event({
                            let this = this.clone();
                            move |event: &MouseDownEvent, phase, _, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let mut event = event.clone();
                                event.position = event.position - origin;
                                this.update(cx, |main, cx| {
                                    main.forward(PlatformInput::MouseDown(event), cx);
                                })
                                .log_err();
                            }
                        });
                        window.on_mouse_event({
                            let this = this.clone();
                            move |event: &MouseUpEvent, phase, _, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let mut event = event.clone();
                                event.position = event.position - origin;
                                this.update(cx, |main, cx| {
                                    main.forward(PlatformInput::MouseUp(event), cx);
                                })
                                .log_err();
                            }
                        });
                        window.on_mouse_event({
                            let this = this.clone();
                            move |event: &ScrollWheelEvent, phase, _, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let mut event = event.clone();
                                event.position = event.position - origin;
                                this.update(cx, |main, cx| {
                                    main.forward(PlatformInput::ScrollWheel(event), cx);
                                })
                                .log_err();
                            }
                        });
                    },
                )
                .size_full(),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let result = this.forward(PlatformInput::KeyDown(event.clone()), cx);
                if !result.propagate {
                    cx.stop_propagation();
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                this.forward(PlatformInput::KeyUp(event.clone()), cx);
            }))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.forward(PlatformInput::ModifiersChanged(event.clone()), cx);
            }))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }
        let bounds = Bounds::centered(None, size(px(900.), px(500.)), cx);
        let main_window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            },
            |window, cx| {
                let canvas = cx
                    .open_texture_window(
                        TextureWindowOptions {
                            size: window.viewport_size(),
                            scale_factor: window.scale_factor(),
                            ..TextureWindowOptions::default()
                        },
                        |window, cx| {
                            cx.new(|cx| {
                                Canvas::new(window, cx).expect("child texture windows should open")
                            })
                        },
                    )
                    .expect("texture windows are not supported on this platform");
                cx.new(|cx| Main {
                    canvas: canvas.into(),
                    focus_handle: cx.focus_handle(),
                })
            },
        );
        match main_window {
            Ok(main_window) => {
                main_window
                    .update(cx, |main, window, cx| {
                        window.focus(&main.focus_handle, cx);
                        window.activate_window();
                    })
                    .log_err();
            }
            Err(error) => {
                eprintln!("failed to open window: {error:#}");
                cx.quit();
            }
        }
    });
}
