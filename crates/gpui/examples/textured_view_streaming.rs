//! Example: TexturedView Streaming with FPS Control
//!
//! This example demonstrates streaming mode with a slider to adjust frame rate.
//! The animation runs in a background thread and streams frames to the main UI.
//!
//! Run with: cargo run -p gpui --example textured_view_streaming

use gpui::{
    App, Application, Bounds, Context, Entity, ParentElement, Render, Styled, TexturedView, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use std::sync::atomic::{AtomicU32, Ordering};

// Shared frame counter for the animation
static FRAME_COUNTER: AtomicU32 = AtomicU32::new(0);

struct StreamingDemo {
    /// Current FPS setting
    fps: u32,
    /// The streaming TexturedView
    textured_view: Option<Entity<TexturedView<fn() -> gpui::Div>>>,
    /// Whether we need to recreate the view (after FPS change)
    needs_recreate: bool,
}

impl StreamingDemo {
    fn new() -> Self {
        Self {
            fps: 30,
            textured_view: None,
            needs_recreate: true,
        }
    }

    fn create_textured_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Reset frame counter when recreating
        FRAME_COUNTER.store(0, Ordering::Relaxed);

        let render_fn: fn() -> gpui::Div = render_animation;
        self.textured_view = Some(cx.new(|cx| {
            TexturedView::streaming(size(px(400.), px(300.)), self.fps, window, cx, render_fn)
        }));
        self.needs_recreate = false;
    }

    fn set_fps(&mut self, fps: u32, cx: &mut Context<Self>) {
        if fps != self.fps && fps >= 1 && fps <= 120 {
            self.fps = fps;
            // Drop the old view and mark for recreation
            self.textured_view = None;
            self.needs_recreate = true;
            cx.notify();
        }
    }
}

/// Render the animated content (runs in background thread)
fn render_animation() -> gpui::Div {
    let frame = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Time-based animation (assuming ~30fps base, scales with actual fps)
    let t = frame as f32 / 30.0;

    // Bouncing ball
    let ball_x = ((t * 2.5).sin() * 0.5 + 0.5) * 340.0; // 0-340 range (400 - 60 ball size)
    let ball_y = ((t * 1.8).sin() * 0.5 + 0.5) * 240.0; // 0-240 range (300 - 60 ball size)

    // Color cycling
    let hue = (frame % 360) as f32 / 360.0;
    let (r, g, b) = hsl_to_rgb(hue, 0.7, 0.6);
    let ball_color = rgb((r as u32) << 16 | (g as u32) << 8 | (b as u32));

    // Secondary spinning element
    let spin_x = 200.0 + (t * 3.0).cos() * 80.0;
    let spin_y = 150.0 + (t * 3.0).sin() * 80.0;

    // Pulsing size
    let pulse = ((t * 4.0).sin() * 0.3 + 0.7) * 30.0;

    div()
        .size_full()
        .bg(rgb(0x1a1a2e))
        .relative()
        // Bouncing ball
        .child(
            div()
                .absolute()
                .left(px(ball_x))
                .top(px(ball_y))
                .w(px(60.))
                .h(px(60.))
                .bg(ball_color)
                .rounded_full(),
        )
        // Spinning square
        .child(
            div()
                .absolute()
                .left(px(spin_x - pulse / 2.0))
                .top(px(spin_y - pulse / 2.0))
                .w(px(pulse))
                .h(px(pulse))
                .bg(rgb(0x00d4ff))
                .rounded_md(),
        )
        // Trail effect (fading squares)
        .children((0..5).map(|i| {
            let offset = i as f32 * 0.1;
            let trail_x = ((t - offset) * 2.5).sin() * 0.5 + 0.5;
            let trail_y = ((t - offset) * 1.8).sin() * 0.5 + 0.5;
            let alpha = 1.0 - (i as f32 * 0.2);
            let size = 60.0 - (i as f32 * 8.0);

            div()
                .absolute()
                .left(px(trail_x * 340.0))
                .top(px(trail_y * 240.0))
                .w(px(size))
                .h(px(size))
                .rounded_full()
                .bg(rgb(0x4a4a6a))
                .opacity(alpha * 0.3)
        }))
        // Frame counter
        .child(
            div()
                .absolute()
                .right(px(10.))
                .top(px(10.))
                .text_color(rgb(0x888888))
                .text_sm()
                .child(format!("Frame: {}", frame)),
        )
        // Label
        .child(
            div()
                .absolute()
                .left(px(10.))
                .bottom(px(10.))
                .text_color(rgb(0xcccccc))
                .text_sm()
                .child("Background Thread Animation"),
        )
}

/// Convert HSL to RGB
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = match (h * 6.0) as u32 {
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

impl Render for StreamingDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Create or recreate the textured view if needed
        if self.needs_recreate {
            self.create_textured_view(window, cx);
        }

        let current_fps = self.fps;

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
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(0x89b4fa))
                            .child("TexturedView Streaming Demo"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .child(format!("Target: {} FPS", current_fps)),
                    ),
            )
            // Description
            .child(div().text_color(rgb(0x6c7086)).text_sm().child(
                "Animation runs in a background thread using Application::textured(). \
                     Adjust FPS with the buttons below.",
            ))
            // Animation container
            .child(
                div().flex().justify_center().child(
                    div()
                        .border_2()
                        .border_color(rgb(0x45475a))
                        .rounded_lg()
                        .overflow_hidden()
                        .children(self.textured_view.clone()),
                ),
            )
            // FPS Controls
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap_4()
                    .items_center()
                    .child(div().text_color(rgb(0xa6adc8)).child("Frame Rate:"))
                    // Preset buttons
                    .child(fps_button("10", 10, current_fps, cx))
                    .child(fps_button("15", 15, current_fps, cx))
                    .child(fps_button("24", 24, current_fps, cx))
                    .child(fps_button("30", 30, current_fps, cx))
                    .child(fps_button("60", 60, current_fps, cx))
                    // Increment/decrement
                    .child(
                        div()
                            .ml_4()
                            .flex()
                            .gap_2()
                            .child(fps_adjust_button("-5", -5, current_fps, cx))
                            .child(fps_adjust_button("-1", -1, current_fps, cx))
                            .child(fps_adjust_button("+1", 1, current_fps, cx))
                            .child(fps_adjust_button("+5", 5, current_fps, cx)),
                    ),
            )
            // Info
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x6c7086))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child("• Changing FPS recreates the background render thread")
                    .child("• Frame counter resets when FPS changes")
                    .child("• Animation timing is frame-based, so speed varies with FPS"),
            )
    }
}

/// Create an FPS preset button
fn fps_button(
    label: &str,
    fps: u32,
    current_fps: u32,
    cx: &mut Context<StreamingDemo>,
) -> impl IntoElement {
    let is_active = fps == current_fps;

    div()
        .id(format!("fps-btn-{}", fps))
        .px(px(12.))
        .py(px(6.))
        .rounded_md()
        .cursor_pointer()
        .bg(if is_active {
            rgb(0x89b4fa)
        } else {
            rgb(0x45475a)
        })
        .text_color(if is_active {
            rgb(0x1e1e2e)
        } else {
            rgb(0xcdd6f4)
        })
        .hover(|s| {
            s.bg(if is_active {
                rgb(0x89b4fa)
            } else {
                rgb(0x585b70)
            })
        })
        .on_click(cx.listener(move |this, _event, _window, cx| {
            this.set_fps(fps, cx);
        }))
        .child(label.to_string())
}

/// Create an FPS adjustment button (+/- increment)
fn fps_adjust_button(
    label: &str,
    delta: i32,
    current_fps: u32,
    cx: &mut Context<StreamingDemo>,
) -> impl IntoElement {
    let new_fps = (current_fps as i32 + delta).clamp(1, 120) as u32;
    let is_disabled = new_fps == current_fps;

    div()
        .id(format!("fps-adj-{}", delta))
        .px(px(8.))
        .py(px(6.))
        .rounded_md()
        .cursor_pointer()
        .bg(if is_disabled {
            rgb(0x313244)
        } else {
            rgb(0x45475a)
        })
        .text_color(if is_disabled {
            rgb(0x6c7086)
        } else {
            rgb(0xcdd6f4)
        })
        .hover(|s| if is_disabled { s } else { s.bg(rgb(0x585b70)) })
        .on_click(cx.listener(move |this, _event, _window, cx| {
            if !is_disabled {
                this.set_fps(new_fps, cx);
            }
        }))
        .child(label.to_string())
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(600.), px(550.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| StreamingDemo::new()),
        )
        .unwrap();

        cx.activate(true);
    });
}
