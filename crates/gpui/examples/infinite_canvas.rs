//! An infinite canvas of cards, each card a texture window.
//!
//! This is the first consumer of texture windows with a real workload: a grid
//! of cards (standing in for a git history viewer's per-file diffs) that is
//! panned and zoomed as a whole while each card stays a complete GPUI window
//! with its own layout, hover and focus.
//!
//! It exercises the two things a canvas needs beyond `texture_windows.rs`:
//!
//! - **Content-sized cards.** A card is opened at a guessed height, measured
//!   with `Window::measure_root` at the column width, and resized to fit. The
//!   grid's row heights come from those measurements. Pressing `r` on the
//!   focused card regenerates its content, which re-measures and resizes it.
//! - **Culling.** Cards outside the viewport (plus a margin) are closed, which
//!   frees their textures, and reopened when they scroll back into view.
//!   Cards inside the viewport but not painted this frame are simply parked.
//!
//! Controls: scroll to pan, ctrl+scroll to zoom, click a card to focus it,
//! `r` to regenerate the focused card.
//!
//! Wayland only for now; other platforms fail at `open_texture_window`.

#[path = "example_support/fonts.rs"]
mod example_support;

use gpui::{
    AnyWindowHandle, App, AvailableSpace, Bounds, Context, DispatchEventResult, DispatchPhase,
    FocusHandle, Focusable, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent, MouseDownEvent,
    MouseExitEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, ScrollWheelEvent,
    SharedString, Size, TextureWindowOptions, Window, WindowBounds, WindowHandle, WindowOptions,
    canvas, div, point, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use gpui_util::ResultExt as _;

const COLUMNS: usize = 4;
const CARD_COUNT: usize = 40;
const CARD_WIDTH: Pixels = px(260.);
const GAP: Pixels = px(16.);
/// Cards this far outside the viewport (in canvas space) are closed.
const CULL_MARGIN: Pixels = px(200.);

/// One line of a fake diff.
#[derive(Clone)]
struct DiffLine {
    added: bool,
    text: SharedString,
}

fn fake_content(index: usize, revision: usize) -> (SharedString, Vec<DiffLine>) {
    let seed = index * 31 + revision * 7;
    let title: SharedString = format!(
        "commit {:07x} · src/module_{}.rs",
        seed * 2654435761 % 0xfffffff,
        index
    )
    .into();
    let line_count = 2 + (seed * 13) % 11;
    let lines = (0..line_count)
        .map(|line| {
            let added = !(seed + line).is_multiple_of(3);
            let words = 3 + (seed + line * 5) % 9;
            let text = (0..words)
                .map(|word| {
                    [
                        "let", "value", "=", "compute(", "item", ")", ";", "if", "ready",
                    ][(seed + line + word) % 9]
                })
                .collect::<Vec<_>>()
                .join(" ");
            DiffLine {
                added,
                text: format!("{} {text}", if added { "+" } else { "-" }).into(),
            }
        })
        .collect();
    (title, lines)
}

/// Card content: an ordinary view that has no idea it lives in a texture.
struct Card {
    title: SharedString,
    lines: Vec<DiffLine>,
    focus_handle: FocusHandle,
}

impl Render for Card {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let active = window.is_window_active();
        div()
            .id("card")
            .track_focus(&self.focus_handle)
            // Width follows the window; height follows the content so the
            // canvas can measure it.
            .w_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .rounded_md()
            .bg(if active { rgb(0x2a2a48) } else { rgb(0x1e1e2e) })
            .hover(|style| style.bg(rgb(0x30304a)))
            .text_color(rgb(0xcdd6f4))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9399b2))
                    .child(self.title.clone()),
            )
            .children(self.lines.iter().map(|line| {
                div()
                    .text_xs()
                    .text_color(if line.added {
                        rgb(0xa6e3a1)
                    } else {
                        rgb(0xf38ba8)
                    })
                    .child(line.text.clone())
            }))
    }
}

struct CardSlot {
    revision: usize,
    /// Canvas-space origin; rows are laid out from measured heights.
    origin: Point<Pixels>,
    /// Measured size, or the initial guess until the card has been opened.
    size: Size<Pixels>,
    /// Open while the card is near the viewport.
    window: Option<WindowHandle<Card>>,
}

struct InfiniteCanvas {
    cards: Vec<CardSlot>,
    pan: Point<Pixels>,
    zoom: f32,
    hovered: Option<usize>,
    focused: Option<usize>,
    focus_handle: FocusHandle,
}

impl InfiniteCanvas {
    fn new(cx: &mut Context<Self>) -> Self {
        let cards = (0..CARD_COUNT)
            .map(|_| CardSlot {
                revision: 0,
                origin: Point::default(),
                size: size(CARD_WIDTH, px(120.)),
                window: None,
            })
            .collect();
        let mut this = Self {
            cards,
            pan: point(GAP, GAP),
            zoom: 1.0,
            hovered: None,
            focused: None,
            focus_handle: cx.focus_handle(),
        };
        this.layout();
        this
    }

    /// Grid layout: fixed columns, row height from the tallest card in the row.
    fn layout(&mut self) {
        let mut y = px(0.);
        for row in self.cards.chunks_mut(COLUMNS) {
            let row_height = row
                .iter()
                .map(|card| card.size.height)
                .fold(px(0.), Pixels::max);
            for (column, card) in row.iter_mut().enumerate() {
                card.origin = point((CARD_WIDTH + GAP) * column as f32, y);
            }
            y += row_height + GAP;
        }
    }

    fn card_bounds(&self, index: usize, canvas_origin: Point<Pixels>) -> Bounds<Pixels> {
        let card = &self.cards[index];
        Bounds {
            origin: canvas_origin + self.pan + card.origin * self.zoom,
            size: card.size.map(|length| length * self.zoom),
        }
    }

    fn to_card(&self, card_bounds: Bounds<Pixels>, position: Point<Pixels>) -> Point<Pixels> {
        (position - card_bounds.origin) / self.zoom
    }

    fn card_at(&self, canvas_origin: Point<Pixels>, position: Point<Pixels>) -> Option<usize> {
        (0..self.cards.len()).rev().find(|&index| {
            self.cards[index].window.is_some()
                && self.card_bounds(index, canvas_origin).contains(&position)
        })
    }

    /// Opens cards near the viewport and closes cards far from it.
    fn sync_visibility(
        &mut self,
        viewport: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keep = Bounds {
            origin: point(-CULL_MARGIN, -CULL_MARGIN),
            size: size(
                viewport.width + CULL_MARGIN * 2.,
                viewport.height + CULL_MARGIN * 2.,
            ),
        };
        let mut relayout = false;
        for index in 0..self.cards.len() {
            let visible = self.card_bounds(index, Point::default()).intersects(&keep);
            match (visible, self.cards[index].window.is_some()) {
                (true, false) => {
                    if self.open_card(index, window, cx) {
                        relayout = true;
                    }
                }
                (false, true) => self.close_card(index, cx),
                _ => {}
            }
        }
        if relayout {
            self.layout();
        }
    }

    /// Opens the card's texture window and sizes it to its content. Returns
    /// whether the measured size differs from the slot's current size.
    fn open_card(&mut self, index: usize, window: &Window, cx: &mut Context<Self>) -> bool {
        let (title, lines) = fake_content(index, self.cards[index].revision);
        let guess = self.cards[index].size;
        let opened = cx.open_texture_window(
            TextureWindowOptions {
                size: guess,
                scale_factor: window.scale_factor(),
                ..TextureWindowOptions::default()
            },
            |_, cx| {
                cx.new(|cx| Card {
                    title,
                    lines,
                    focus_handle: cx.focus_handle(),
                })
            },
        );
        let Some(handle) = opened.log_err() else {
            return false;
        };
        self.cards[index].window = Some(handle);
        self.fit_card(index, cx)
    }

    /// Measures the card at the column width and resizes its window to fit.
    fn fit_card(&mut self, index: usize, cx: &mut App) -> bool {
        let Some(handle) = self.cards[index].window else {
            return false;
        };
        let handle: AnyWindowHandle = handle.into();
        let measured = handle
            .update(cx, |_, card_window, cx| {
                let measured = card_window.measure_root(
                    size(
                        AvailableSpace::Definite(CARD_WIDTH),
                        AvailableSpace::MinContent,
                    ),
                    cx,
                );
                card_window.resize(measured);
                measured
            })
            .log_err();
        let Some(measured) = measured else {
            return false;
        };
        let changed = self.cards[index].size != measured;
        self.cards[index].size = measured;
        changed
    }

    fn close_card(&mut self, index: usize, cx: &mut App) {
        if let Some(handle) = self.cards[index].window.take() {
            let handle: AnyWindowHandle = handle.into();
            handle
                .update(cx, |_, card_window, _| card_window.remove_window())
                .log_err();
        }
        if self.hovered == Some(index) {
            self.hovered = None;
        }
        if self.focused == Some(index) {
            self.focused = None;
        }
    }

    /// Replaces the focused card's content, then re-measures and resizes it.
    fn regenerate_focused(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.focused else { return };
        let Some(handle) = self.cards[index].window else {
            return;
        };
        self.cards[index].revision += 1;
        let (title, lines) = fake_content(index, self.cards[index].revision);
        handle
            .update(cx, |card, _, cx| {
                card.title = title;
                card.lines = lines;
                cx.notify();
            })
            .log_err();
        if self.fit_card(index, cx) {
            self.layout();
        }
        cx.notify();
    }

    fn forward(&self, index: usize, input: PlatformInput, cx: &mut App) -> DispatchEventResult {
        let Some(handle) = self.cards[index].window else {
            return DispatchEventResult::default();
        };
        let handle: AnyWindowHandle = handle.into();
        handle
            .update(cx, |_, card_window, cx| {
                card_window.dispatch_event(input, cx)
            })
            .log_err()
            .unwrap_or_default()
    }

    fn set_focused(&mut self, index: Option<usize>, cx: &mut App) {
        if self.focused == index {
            return;
        }
        for (target, active) in [(self.focused, false), (index, true)] {
            let Some(target) = target else { continue };
            let Some(handle) = self.cards[target].window else {
                continue;
            };
            let handle: AnyWindowHandle = handle.into();
            handle
                .update(cx, |_, card_window, cx| {
                    card_window.set_texture_active(active, cx)
                })
                .log_err();
        }
        self.focused = index;
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        for index in 0..self.cards.len() {
            let Some(handle) = self.cards[index].window else {
                continue;
            };
            let card_bounds = self.card_bounds(index, bounds.origin);
            if card_bounds.intersects(&bounds) {
                window.paint_texture_window(&handle.into(), card_bounds, cx);
            }
        }

        let this = cx.entity().downgrade();
        let canvas_origin = bounds.origin;

        window.on_mouse_event({
            let this = this.clone();
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                this.update(cx, |canvas, cx| {
                    let hit = canvas.card_at(canvas_origin, event.position);
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
                        let card_bounds = canvas.card_bounds(index, canvas_origin);
                        let mut event = event.clone();
                        event.position = canvas.to_card(card_bounds, event.position);
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
                    let hit = canvas.card_at(canvas_origin, event.position);
                    canvas.set_focused(hit, cx);
                    let Some(index) = hit else { return };
                    let card_bounds = canvas.card_bounds(index, canvas_origin);
                    let mut event = event.clone();
                    event.position = canvas.to_card(card_bounds, event.position);
                    if !canvas
                        .forward(index, PlatformInput::MouseDown(event), cx)
                        .propagate
                    {
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
                    let card_bounds = canvas.card_bounds(index, canvas_origin);
                    let mut event = event.clone();
                    event.position = canvas.to_card(card_bounds, event.position);
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
                        // Zoom around the cursor so the point under it stays put.
                        let old_zoom = canvas.zoom;
                        canvas.zoom =
                            (old_zoom * 1.1f32.powf(delta.y.as_f32() / 20.)).clamp(0.1, 6.);
                        let cursor = event.position - canvas_origin;
                        let canvas_point = (cursor - canvas.pan) / old_zoom;
                        canvas.pan = cursor - canvas_point * canvas.zoom;
                    } else if let Some(index) = canvas.card_at(canvas_origin, event.position) {
                        let card_bounds = canvas.card_bounds(index, canvas_origin);
                        let mut event = event.clone();
                        event.position = canvas.to_card(card_bounds, event.position);
                        canvas.forward(index, PlatformInput::ScrollWheel(event), cx);
                        return;
                    } else {
                        canvas.pan = canvas.pan + delta;
                    }
                    cx.notify();
                })
                .log_err();
            }
        });
    }
}

impl Focusable for InfiniteCanvas {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InfiniteCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_visibility(window.viewport_size(), window, cx);
        let open = self
            .cards
            .iter()
            .filter(|card| card.window.is_some())
            .count();
        let this = cx.entity().downgrade();

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(0x11111b))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        this.update(cx, |canvas, cx| canvas.paint(bounds, window, cx))
                            .log_err();
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(0x313244))
                    .text_xs()
                    .text_color(rgb(0xcdd6f4))
                    .child(format!(
                        "{open}/{CARD_COUNT} cards open · zoom {:.2} · scroll: pan · ctrl+scroll: zoom · click: focus · r: regenerate",
                        self.zoom
                    )),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "r" && event.keystroke.modifiers == Default::default() {
                    this.regenerate_focused(cx);
                    cx.stop_propagation();
                    return;
                }
                if let Some(index) = this.focused
                    && !this.forward(index, PlatformInput::KeyDown(event.clone()), cx).propagate
                {
                    cx.stop_propagation();
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if let Some(index) = this.focused {
                    this.forward(index, PlatformInput::KeyUp(event.clone()), cx);
                }
            }))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                for index in 0..this.cards.len() {
                    this.forward(index, PlatformInput::ModifiersChanged(event.clone()), cx);
                }
            }))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }
        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            },
            |_, cx| cx.new(InfiniteCanvas::new),
        );
        match window {
            Ok(window) => {
                window
                    .update(cx, |canvas, window, cx| {
                        window.focus(&canvas.focus_handle, cx);
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
