//! Example: TexturedView Usage
//!
//! This example demonstrates the three modes of TexturedView:
//! 1. Fixed size - render at exact dimensions
//! 2. Measured height - fixed width, content determines height
//! 3. Streaming - continuous frame updates with animation
//!
//! Run with: cargo run -p gpui --example textured_view

use gpui::{
    App, Application, Bounds, Context, Entity, ParentElement, Styled, TexturedView, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use std::sync::atomic::{AtomicU32, Ordering};

// Global frame counter for the streaming animation
static FRAME_COUNTER: AtomicU32 = AtomicU32::new(0);

struct ExampleApp {
    // Store TexturedViews so they're only created once
    fixed_view: Option<Entity<TexturedView<fn() -> gpui::Div>>>,
    measured_view: Option<Entity<TexturedView<fn() -> gpui::Div>>>,
    streaming_view: Option<Entity<TexturedView<fn() -> gpui::Div>>>,
    initialized: bool,
}

impl ExampleApp {
    fn new() -> Self {
        Self {
            fixed_view: None,
            measured_view: None,
            streaming_view: None,
            initialized: false,
        }
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.initialized {
            return;
        }
        self.initialized = true;

        // Fixed size view - exact 200x150 dimensions
        // Cast fn items to fn pointers
        let fixed_fn: fn() -> gpui::Div = render_fixed_content;
        self.fixed_view =
            Some(cx.new(|cx| TexturedView::fixed(size(px(200.), px(150.)), window, cx, fixed_fn)));

        // Measured height view - width 250, height determined by content
        let measured_fn: fn() -> gpui::Div = render_measured_content;
        self.measured_view =
            Some(cx.new(|cx| TexturedView::measured(px(250.), window, cx, measured_fn)));

        // Streaming view - continuously updates at 30 FPS with animation
        let streaming_fn: fn() -> gpui::Div = render_streaming_content;
        self.streaming_view = Some(cx.new(|cx| {
            TexturedView::streaming(size(px(200.), px(150.)), 30, window, cx, streaming_fn)
        }));
    }
}

// Render functions defined as fn pointers so they're Clone + Send
fn render_fixed_content() -> gpui::Div {
    div()
        .size_full()
        .bg(rgb(0x3498db))
        .flex()
        .items_center()
        .justify_center()
        .child(div().text_color(rgb(0xffffff)).child("Fixed 200x150"))
}

fn render_measured_content() -> gpui::Div {
    div()
        .w_full()
        .bg(rgb(0x2ecc71))
        .p(px(16.))
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_color(rgb(0xffffff)).child("Measured Height"))
        .child(
            div()
                .text_color(rgb(0xdddddd))
                .child("Width is fixed at 250px"),
        )
        .child(
            div()
                .text_color(rgb(0xdddddd))
                .child("Height is determined by content"),
        )
        .child(
            div()
                .text_color(rgb(0xdddddd))
                .child("This is useful for text blocks"),
        )
}

fn render_streaming_content() -> gpui::Div {
    // Increment frame counter
    let frame = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Bouncing box animation
    let t = (frame as f32) / 30.0; // Time in seconds at 30 FPS
    let bounce_x = ((t * 2.0).sin() * 0.5 + 0.5) * 140.0; // 0-140 range
    let bounce_y = ((t * 3.0).sin() * 0.5 + 0.5) * 90.0; // 0-90 range

    // Color cycling
    let hue = (frame % 180) as f32 / 180.0;
    let (r, g, b) = hue_to_rgb(hue);
    let box_color = rgb((r as u32) << 16 | (g as u32) << 8 | (b as u32));

    div()
        .size_full()
        .bg(rgb(0x1a1a2e))
        .relative()
        // Bouncing box
        .child(
            div()
                .absolute()
                .left(px(bounce_x + 10.0))
                .top(px(bounce_y + 10.0))
                .w(px(40.))
                .h(px(40.))
                .bg(box_color)
                .rounded_md(),
        )
        // Frame counter display
        .child(
            div()
                .absolute()
                .right(px(8.))
                .top(px(8.))
                .text_color(rgb(0x888888))
                .text_xs()
                .child(format!("Frame: {}", frame)),
        )
        // Label
        .child(
            div()
                .absolute()
                .left(px(8.))
                .bottom(px(8.))
                .text_color(rgb(0xffffff))
                .text_xs()
                .child("30 FPS Animation"),
        )
}

/// Convert HSV hue (0-1) to RGB
fn hue_to_rgb(hue: f32) -> (u8, u8, u8) {
    let h = hue * 6.0;
    let c = 0.8f32;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = 0.2f32;

    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

impl Render for ExampleApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Initialize views once (now with window parameter)
        self.initialize(window, cx);

        // Main UI layout
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap_6()
            // Title
            .child(
                div()
                    .text_2xl()
                    .text_color(rgb(0x89b4fa))
                    .child("TexturedView Examples"),
            )
            // Description
            .child(div().text_color(rgb(0xa6adc8)).child(
                "Each box below is rendered in a background thread using Application::textured()",
            ))
            // Views container
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_6()
                    // Fixed size view
                    .child(view_card(
                        "Fixed Size",
                        "Exact 200x150 pixels",
                        self.fixed_view.clone(),
                    ))
                    // Measured height view
                    .child(view_card(
                        "Measured Height",
                        "Width 250px, height from content",
                        self.measured_view.clone(),
                    ))
                    // Streaming view
                    .child(view_card(
                        "Streaming",
                        "Bouncing box at 30 FPS",
                        self.streaming_view.clone(),
                    )),
            )
            // Footer
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x6c7086))
                    .child("Note: Textured rendering only works on Linux/FreeBSD"),
            )
    }
}

/// Create a card wrapping a TexturedView with a label
fn view_card(
    title: &str,
    description: &str,
    view: Option<Entity<TexturedView<fn() -> gpui::Div>>>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_base()
                .text_color(rgb(0xcdd6f4))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x6c7086))
                .child(description.to_string()),
        )
        .child(
            div()
                .border_2()
                .border_color(rgb(0x45475a))
                .rounded_lg()
                .overflow_hidden()
                .children(view),
        )
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ExampleApp::new()),
        )
        .unwrap();

        cx.activate(true);
    });
}
