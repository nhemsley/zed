//! Example demonstrating TexturedSurface rendering with text output to PNG.
//!
//! This example shows how to use `Application::textured()` to render GPUI content
//! to a GPU texture and read back the pixels, which can then be saved as an image.
//!
//! Run with: cargo run --example textured_surface --features wayland
//!
//! Note: This requires a GPU and is Linux-specific.

use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};

struct TexturedView {
    message: String,
}

impl Render for TexturedView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .bg(gpui::white())
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_4()
            .child(
                div()
                    .text_3xl()
                    .text_color(gpui::black())
                    .child("GPUI TexturedSurface"),
            )
            .child(
                div()
                    .text_xl()
                    .text_color(gpui::rgb(0x666666))
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .bg(gpui::red())
                            .size(px(50.0))
                            .rounded_md()
                            .flex()
                            .justify_center()
                            .items_center()
                            .text_color(gpui::white())
                            .child("R"),
                    )
                    .child(
                        div()
                            .bg(gpui::green())
                            .size(px(50.0))
                            .rounded_md()
                            .flex()
                            .justify_center()
                            .items_center()
                            .text_color(gpui::white())
                            .child("G"),
                    )
                    .child(
                        div()
                            .bg(gpui::blue())
                            .size(px(50.0))
                            .rounded_md()
                            .flex()
                            .justify_center()
                            .items_center()
                            .text_color(gpui::white())
                            .child("B"),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .border_1()
                    .border_color(gpui::rgb(0xcccccc))
                    .rounded_lg()
                    .child("Rendered to texture without a display window!"),
            )
    }
}

fn main() {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        run_textured_example();
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        eprintln!("TexturedSurface is only available on Linux/FreeBSD");
        std::process::exit(1);
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn run_textured_example() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let output_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "textured_output.png".to_string());

    let pixels_result: Rc<RefCell<Option<(Vec<u8>, u32, u32)>>> = Rc::new(RefCell::new(None));
    let pixels_clone = pixels_result.clone();

    let app = Application::textured();

    app.run(move |cx: &mut App| {
        let width = 800.0_f32;
        let height = 600.0_f32;

        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);

        let window_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| TexturedView {
                    message: "Hello from TexturedSurface!".to_string(),
                })
            },
        );

        match window_result {
            Ok(window) => {
                // Convert to AnyWindowHandle for use in spawn
                let window_handle: gpui::AnyWindowHandle = window.into();
                let pixels_for_callback = pixels_clone.clone();
                let width_copy = width;
                let height_copy = height;

                // Schedule the draw and pixel read for the next frame
                // This avoids the nested update issue
                cx.spawn(async move |cx| {
                    // Force a complete draw and present cycle to render to the texture
                    let draw_result = cx.update_window(window_handle, |_, window, cx| {
                        window.draw_and_present(cx)
                    });

                    match draw_result {
                        Ok(true) => {
                            println!("Draw and present completed successfully");
                        }
                        Ok(false) => {
                            eprintln!("Window is not in a valid state for drawing");
                            return;
                        }
                        Err(e) => {
                            eprintln!("Failed to draw: {}", e);
                            return;
                        }
                    }

                    // Now read the pixels after the draw and present
                    let read_result = cx.update_window(window_handle, |_, window, _cx| {
                        window.read_pixels()
                    });

                    match read_result {
                        Ok(Some(pixels)) => {
                            println!(
                                "Successfully rendered {}x{} image",
                                width_copy as u32, height_copy as u32
                            );
                            println!("Pixel data size: {} bytes", pixels.len());
                            *pixels_for_callback.borrow_mut() =
                                Some((pixels, width_copy as u32, height_copy as u32));
                        }
                        Ok(None) => {
                            eprintln!(
                                "Window does not support pixel readback (read_pixels returned None)"
                            );
                            eprintln!("This may mean the platform window hasn't rendered yet.");
                        }
                        Err(e) => {
                            eprintln!("Failed to read pixels: {}", e);
                        }
                    }

                    // Quit after we're done
                    cx.update(|cx| cx.quit()).ok();
                })
                .detach();

                // Return early - the spawn will handle quitting
                return;
            }
            Err(e) => {
                eprintln!("Failed to create window: {}", e);
                eprintln!("This may be expected if no GPU is available.");
            }
        }

    });

    // After the app has run, save the pixels to PNG
    if let Some((pixels, width, height)) = pixels_result.borrow_mut().take() {
        match save_to_png(&pixels, width, height, &output_path) {
            Ok(()) => {
                println!("Saved PNG to: {}", output_path);
            }
            Err(e) => {
                eprintln!("Failed to save PNG: {}", e);
            }
        }
    } else {
        eprintln!("No pixels were captured");
    }

    println!("TexturedSurface example completed!");
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn save_to_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // The pixels are in BGRA format from the GPU, we need to convert to RGBA for PNG
    let mut rgba_pixels = Vec::with_capacity(pixels.len());
    for chunk in pixels.chunks(4) {
        if chunk.len() == 4 {
            // BGRA -> RGBA
            rgba_pixels.push(chunk[2]); // R
            rgba_pixels.push(chunk[1]); // G
            rgba_pixels.push(chunk[0]); // B
            rgba_pixels.push(chunk[3]); // A
        }
    }

    let img = image::RgbaImage::from_raw(width, height, rgba_pixels)
        .ok_or("Failed to create image from pixel data")?;

    img.save(path)?;

    Ok(())
}
