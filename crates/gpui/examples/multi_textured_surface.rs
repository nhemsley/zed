//! Example demonstrating TexturedSurface rendering with multiple states combined into one PNG.
//!
//! This example shows how to use `Application::textured()` to render GPUI content
//! to a GPU texture, update the view state, render again, and combine both images
//! into a single PNG output.
//!
//! Run with: cargo run --example multi_textured_surface --features wayland
//!
//! Note: This requires a GPU and is Linux-specific.

use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::cell::RefCell;
use std::rc::Rc;

struct DemoView {
    title: String,
    message: String,
    color: u32,
    frame_number: usize,
}

impl DemoView {
    fn new(frame_number: usize) -> Self {
        match frame_number {
            1 => Self {
                title: "Frame 1: Initial State".into(),
                message: "This is the first render".into(),
                color: 0x3498db, // Blue
                frame_number: 1,
            },
            2 => Self {
                title: "Frame 2: Updated State".into(),
                message: "The view has been updated!".into(),
                color: 0xe74c3c, // Red
                frame_number: 2,
            },
            _ => Self {
                title: format!("Frame {}", frame_number),
                message: "Additional frame".into(),
                color: 0x2ecc71, // Green
                frame_number,
            },
        }
    }
}

impl Render for DemoView {
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
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_xl()
                    .text_color(rgb(0x666666))
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .mt_4()
                    .bg(rgb(self.color))
                    .px_8()
                    .py_4()
                    .rounded_lg()
                    .text_color(gpui::white())
                    .text_xl()
                    .child(format!("Frame #{}", self.frame_number)),
            )
            .child(
                div()
                    .mt_8()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .bg(rgb(0xe74c3c))
                            .size(px(40.0))
                            .rounded_full()
                            .when(self.frame_number == 1, |d| d.opacity(1.0))
                            .when(self.frame_number != 1, |d| d.opacity(0.3)),
                    )
                    .child(
                        div()
                            .bg(rgb(0xf39c12))
                            .size(px(40.0))
                            .rounded_full()
                            .opacity(0.3),
                    )
                    .child(
                        div()
                            .bg(rgb(0x2ecc71))
                            .size(px(40.0))
                            .rounded_full()
                            .when(self.frame_number == 2, |d| d.opacity(1.0))
                            .when(self.frame_number != 2, |d| d.opacity(0.3)),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .border_1()
                    .border_color(rgb(0xcccccc))
                    .rounded_lg()
                    .text_color(rgb(0x888888))
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
    let output_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "multi_textured_output.png".to_string());

    // Store captured frames
    let frames: Rc<RefCell<Vec<(Vec<u8>, u32, u32)>>> = Rc::new(RefCell::new(Vec::new()));
    let frames_clone = frames.clone();

    let app = Application::textured();

    app.run(move |cx: &mut App| {
        let width = 400.0_f32;
        let height = 300.0_f32;

        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);

        let window_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| DemoView::new(1)),
        );

        match window_result {
            Ok(window) => {
                let window_handle: gpui::AnyWindowHandle = window.into();
                let frames_for_callback = frames_clone.clone();
                let width_u32 = width as u32;
                let height_u32 = height as u32;

                cx.spawn(async move |cx| {
                    // === Frame 1: Render initial state ===
                    println!("Rendering Frame 1...");

                    let draw_result = cx
                        .update_window(window_handle, |_, window, cx| window.draw_and_present(cx));

                    if let Err(e) = draw_result {
                        eprintln!("Failed to draw frame 1: {}", e);
                        cx.update(|cx| cx.quit()).ok();
                        return;
                    }

                    let pixels1 = cx
                        .update_window(window_handle, |_, window, _| window.read_pixels())
                        .ok()
                        .flatten();

                    if let Some(pixels) = pixels1 {
                        println!("Frame 1 captured: {} bytes", pixels.len());
                        frames_for_callback
                            .borrow_mut()
                            .push((pixels, width_u32, height_u32));
                    } else {
                        eprintln!("Failed to capture frame 1");
                        cx.update(|cx| cx.quit()).ok();
                        return;
                    }

                    // === Frame 2: Update the view and render again ===
                    println!("Updating view and rendering Frame 2...");

                    // Update the view to a new state
                    // We need to get the root view and update it through the entity system
                    let update_result =
                        cx.update_window(window_handle, |root_view, _window, cx| {
                            if let Ok(view) = root_view.downcast::<DemoView>() {
                                view.update(cx, |view, _cx| {
                                    *view = DemoView::new(2);
                                });
                            }
                        });

                    if let Err(e) = update_result {
                        eprintln!("Failed to update view: {}", e);
                        cx.update(|cx| cx.quit()).ok();
                        return;
                    }

                    // Draw the updated state
                    let draw_result = cx
                        .update_window(window_handle, |_, window, cx| window.draw_and_present(cx));

                    if let Err(e) = draw_result {
                        eprintln!("Failed to draw frame 2: {}", e);
                        cx.update(|cx| cx.quit()).ok();
                        return;
                    }

                    let pixels2 = cx
                        .update_window(window_handle, |_, window, _| window.read_pixels())
                        .ok()
                        .flatten();

                    if let Some(pixels) = pixels2 {
                        println!("Frame 2 captured: {} bytes", pixels.len());
                        frames_for_callback
                            .borrow_mut()
                            .push((pixels, width_u32, height_u32));
                    } else {
                        eprintln!("Failed to capture frame 2");
                    }

                    println!("All frames captured, quitting...");
                    cx.update(|cx| cx.quit()).ok();
                })
                .detach();
            }
            Err(e) => {
                eprintln!("Failed to create window: {}", e);
                eprintln!("This may be expected if no GPU is available.");
                cx.quit();
            }
        }
    });

    // After the app has run, combine and save the frames
    let captured_frames = frames.borrow();
    if captured_frames.len() >= 2 {
        println!(
            "\nCombining {} frames into single image...",
            captured_frames.len()
        );

        match combine_frames_horizontally(&captured_frames, &output_path) {
            Ok(()) => {
                println!("Saved combined PNG to: {}", output_path);
            }
            Err(e) => {
                eprintln!("Failed to save PNG: {}", e);
            }
        }
    } else {
        eprintln!(
            "Not enough frames captured (got {}, need 2)",
            captured_frames.len()
        );
    }

    println!("MultiTexturedSurface example completed!");
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn combine_frames_horizontally(
    frames: &[(Vec<u8>, u32, u32)],
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if frames.is_empty() {
        return Err("No frames to combine".into());
    }

    // Calculate combined dimensions
    let total_width: u32 = frames.iter().map(|(_, w, _)| w).sum();
    let max_height: u32 = frames.iter().map(|(_, _, h)| *h).max().unwrap_or(0);

    // Add a small gap between frames
    let gap = 4u32;
    let total_width_with_gaps = total_width + gap * (frames.len() as u32 - 1);

    // Create the combined image
    let mut combined = image::RgbaImage::new(total_width_with_gaps, max_height);

    // Fill with a light gray background
    for pixel in combined.pixels_mut() {
        *pixel = image::Rgba([240, 240, 240, 255]);
    }

    // Copy each frame into the combined image
    let mut x_offset = 0u32;
    for (i, (pixels, width, height)) in frames.iter().enumerate() {
        // Convert BGRA to RGBA
        let rgba_pixels: Vec<u8> = pixels
            .chunks(4)
            .flat_map(|chunk| {
                if chunk.len() == 4 {
                    vec![chunk[2], chunk[1], chunk[0], chunk[3]] // BGRA -> RGBA
                } else {
                    vec![0, 0, 0, 255]
                }
            })
            .collect();

        // Create an image from the frame
        if let Some(frame_img) = image::RgbaImage::from_raw(*width, *height, rgba_pixels) {
            // Copy pixels to combined image
            for y in 0..*height {
                for x in 0..*width {
                    let pixel = frame_img.get_pixel(x, y);
                    if x_offset + x < total_width_with_gaps && y < max_height {
                        combined.put_pixel(x_offset + x, y, *pixel);
                    }
                }
            }
        }

        x_offset += width + if i < frames.len() - 1 { gap } else { 0 };
    }

    // Save the combined image
    combined.save(path)?;

    println!(
        "Combined image size: {}x{} pixels",
        total_width_with_gaps, max_height
    );

    Ok(())
}
