//! Example: Multi-App with Textured Rendering
//!
//! This example demonstrates:
//! 1. Running a GPUI application in "textured" mode on a background thread
//! 2. Rendering to a GPU texture instead of a display surface
//! 3. Reading back the rendered pixels
//! 4. Sending those pixels via flume channel to the main app
//! 5. Displaying the rendered pixels in the main app's window using img()
//!
//! This pattern is useful for:
//! - Offscreen rendering
//! - Generating thumbnails
//! - Embedding GPUI content in other contexts
//!
//! Note: This requires the textured platform (Linux/FreeBSD only) and
//! the main thread assertion to be disabled in App::new_app().

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use flume::{Receiver, Sender};
use gpui::{
    AnyWindowHandle, App, Application, Bounds, Context, RenderImage, SharedString, Timer, Window,
    WindowBounds, WindowOptions, div, img, prelude::*, px, rgb, size,
};
use image::{Frame, RgbaImage};
use smallvec::smallvec;

/// Message sent from the background renderer to the main app
struct RenderedFrame {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

struct MainApp {
    frame_receiver: Receiver<RenderedFrame>,
    current_image: Option<Arc<RenderImage>>,
    frame_count: u32,
    status: SharedString,
}

impl MainApp {
    fn new(frame_receiver: Receiver<RenderedFrame>) -> Self {
        Self {
            frame_receiver,
            current_image: None,
            frame_count: 0,
            status: "Waiting for frames...".into(),
        }
    }

    fn poll_for_frame(&mut self) {
        // Try to receive a new frame without blocking
        while let Ok(frame) = self.frame_receiver.try_recv() {
            self.frame_count += 1;
            self.status = format!("Received frame #{}", self.frame_count).into();

            // Convert raw RGBA pixels to RenderImage
            // The textured surface renders in BGRA format, so we need to convert
            let mut rgba_pixels = frame.pixels.clone();
            for chunk in rgba_pixels.chunks_exact_mut(4) {
                chunk.swap(0, 2); // Swap B and R
            }

            if let Some(image_buffer) = RgbaImage::from_raw(frame.width, frame.height, rgba_pixels)
            {
                let image_frame = Frame::new(image_buffer);
                self.current_image = Some(Arc::new(RenderImage::new(smallvec![image_frame])));
            }
        }
    }
}

impl Render for MainApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Poll for new frames each render
        self.poll_for_frame();

        // Schedule another frame to keep polling
        window
            .spawn(cx, async move |cx| {
                Timer::after(Duration::from_millis(16)).await;
                cx.update(|window, _cx| {
                    window.refresh();
                })
                .ok();
            })
            .detach();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x1e1e2e))
            .size_full()
            .p_4()
            .child(
                div()
                    .text_2xl()
                    .text_color(rgb(0x89b4fa))
                    .child("Main App - Displaying Textured Renderer Output"),
            )
            .child(
                div()
                    .text_base()
                    .text_color(rgb(0xa6adc8))
                    .child(self.status.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x313244))
                    .rounded_lg()
                    .child(if let Some(image) = &self.current_image {
                        div()
                            .border_2()
                            .border_color(rgb(0x89b4fa))
                            .rounded_md()
                            .overflow_hidden()
                            .child(img(image.clone()).w(px(400.)).h(px(300.)))
                            .into_any_element()
                    } else {
                        div()
                            .text_color(rgb(0x6c7086))
                            .child("No frames received yet...")
                            .into_any_element()
                    }),
            )
            .child(div().text_sm().text_color(rgb(0x6c7086)).child(
                "The image above is rendered by a background thread using GPUI's textured mode",
            ))
    }
}

/// The view rendered in the background thread's textured app
struct BackgroundRenderer {
    hue: f32,
    frame_sender: Sender<RenderedFrame>,
    window_handle: Option<AnyWindowHandle>,
}

impl BackgroundRenderer {
    fn new(frame_sender: Sender<RenderedFrame>) -> Self {
        Self {
            hue: 0.0,
            frame_sender,
            window_handle: None,
        }
    }

    fn hue_to_rgb(hue: f32) -> (u8, u8, u8) {
        let h = hue * 6.0;
        let c = 1.0f32;
        let x = c * (1.0 - ((h % 2.0) - 1.0).abs());

        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };

        ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }
}

impl Render for BackgroundRenderer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Store the window handle for pixel reading
        if self.window_handle.is_none() {
            self.window_handle = Some(window.window_handle());
        }

        // Animate the hue
        self.hue = (self.hue + 0.005) % 1.0;
        let (r, g, b) = Self::hue_to_rgb(self.hue);
        let bg_color = rgb((r as u32) << 16 | (g as u32) << 8 | (b as u32));

        // Schedule reading pixels after this frame renders
        let sender = self.frame_sender.clone();
        let window_handle = self.window_handle.unwrap();

        window
            .spawn(cx, async move |cx| {
                // Small delay to ensure rendering is complete
                Timer::after(Duration::from_millis(5)).await;

                cx.update_window(window_handle, |_, window, cx| {
                    // Force a draw and present cycle to render the scene to texture
                    window.draw_and_present(cx);

                    if let Some(pixels) = window.read_pixels() {
                        let bounds = window.bounds();
                        let width: u32 = bounds.size.width.into();
                        let height: u32 = bounds.size.height.into();

                        sender
                            .send(RenderedFrame {
                                pixels,
                                width,
                                height,
                            })
                            .ok();
                    }
                })
                .ok();
            })
            .detach();

        // Schedule next frame
        window
            .spawn(cx, async move |cx| {
                Timer::after(Duration::from_millis(33)).await; // ~30 FPS
                cx.update(|window, _cx| {
                    window.refresh();
                })
                .ok();
            })
            .detach();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .bg(bg_color)
            .size_full()
            .justify_center()
            .items_center()
            .child(
                div()
                    .text_3xl()
                    .text_color(rgb(0xffffff))
                    .child("Textured Renderer"),
            )
            .child(
                div()
                    .text_xl()
                    .text_color(rgb(0xffffff))
                    .child("Running in background thread"),
            )
            .child(
                div()
                    .mt_8()
                    .size_24()
                    .bg(rgb(0xffffff))
                    .rounded_full()
                    .shadow_xl(),
            )
            .child(
                div()
                    .mt_4()
                    .flex()
                    .gap_2()
                    .child(div().size_8().bg(rgb(0xff0000)).rounded_md())
                    .child(div().size_8().bg(rgb(0x00ff00)).rounded_md())
                    .child(div().size_8().bg(rgb(0x0000ff)).rounded_md()),
            )
    }
}

fn main() {
    // Create a channel for sending rendered frames from background to main
    let (frame_sender, frame_receiver): (Sender<RenderedFrame>, Receiver<RenderedFrame>) =
        flume::bounded(4);

    // Spawn the background renderer thread
    let background_handle = thread::spawn(move || {
        // Give the main thread a moment to start
        thread::sleep(Duration::from_millis(500));

        println!("[Background Thread] Starting textured GPUI application...");

        // Use the textured platform for offscreen rendering
        Application::textured().run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(400.), px(300.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| BackgroundRenderer::new(frame_sender)),
            )
            .unwrap();

            println!("[Background Thread] Textured renderer started!");
        });

        println!("[Background Thread] Textured application finished.");
    });

    println!("[Main Thread] Starting main GPUI application...");

    // Run the main application on the main thread
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(600.), px(500.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MainApp::new(frame_receiver)),
        )
        .unwrap();

        cx.activate(true);
        println!("[Main Thread] Main application window opened!");
    });

    println!("[Main Thread] Main application finished.");

    // Wait for the background thread to complete
    if let Err(e) = background_handle.join() {
        eprintln!("Background thread panicked: {:?}", e);
    }

    println!("All applications have finished.");
}
