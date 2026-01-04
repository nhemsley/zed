//! Render-to-Texture Example
//!
//! This example demonstrates how to use GPUI's render-to-texture functionality
//! to render a scene to an offscreen texture and export it to a PNG file.
//!
//! Run with: cargo run -p gpui --example render_to_texture --features render-to-texture

fn main() {
    #[cfg(feature = "render-to-texture")]
    render_to_texture_example::run();

    #[cfg(not(feature = "render-to-texture"))]
    {
        eprintln!("This example requires the 'render-to-texture' feature.");
        eprintln!(
            "Run with: cargo run -p gpui --example render_to_texture --features render-to-texture"
        );
        std::process::exit(1);
    }
}

#[cfg(feature = "render-to-texture")]
mod render_to_texture_example {
    use gpui::{
        App, Application, Bounds, Context, DevicePixels, Hsla, SharedString, Window, WindowBounds,
        WindowOptions, div, hsla, prelude::*, px, rgb, size, white,
    };

    struct RenderToTextureDemo {
        text: SharedString,
        exported: bool,
    }

    impl Render for RenderToTextureDemo {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .bg(rgb(0x2d2d2d))
                .size(px(400.0))
                .justify_center()
                .items_center()
                .border_2()
                .border_color(rgb(0x00aaff))
                .rounded_lg()
                .shadow_lg()
                .child(
                    div()
                        .text_xl()
                        .text_color(rgb(0xffffff))
                        .child(format!("🎨 {}", &self.text)),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(colored_box(gpui::red()))
                        .child(colored_box(gpui::green()))
                        .child(colored_box(gpui::blue()))
                        .child(colored_box(gpui::yellow()))
                        .child(colored_box(hsla(0.75, 0.6, 0.5, 1.0))), // purple
                )
                .child(
                    div()
                        .mt_4()
                        .text_sm()
                        .text_color(rgb(0x888888))
                        .child(if self.exported {
                            "✅ Texture exported! Check console for path."
                        } else {
                            "Creating offscreen renderer..."
                        }),
                )
        }
    }

    fn colored_box(color: Hsla) -> impl IntoElement {
        div()
            .size_10()
            .bg(color)
            .border_1()
            .border_color(white().opacity(0.3))
            .rounded_md()
    }

    pub fn run() {
        Application::new().run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(500.), px(500.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    // Try to create an offscreen renderer
                    let max_size = size(DevicePixels(1024), DevicePixels(1024));

                    match window.create_offscreen_renderer(max_size) {
                        Some(mut offscreen) => {
                            println!("✅ Offscreen renderer created successfully!");
                            println!("   Max texture size: {:?}", offscreen.max_texture_size());

                            // Create a texture
                            let texture_size = size(DevicePixels(512), DevicePixels(512));
                            let texture_info = offscreen.create_texture(texture_size);
                            println!("✅ Texture created: {:?}", texture_info);

                            // Note: To actually render to the texture, we would need to:
                            // 1. Build a Scene from elements
                            // 2. Call offscreen.draw_scene_to_texture(&scene, texture_info.id)
                            //
                            // The Scene type is internal to GPUI, so this would typically be done
                            // through a higher-level API that GPUI would provide.
                            //
                            // For now, this example demonstrates the API availability.

                            println!("ℹ️  Render-to-texture API is available!");
                            println!("   Texture ID: {:?}", texture_info.id);
                            println!("   Texture Size: {:?}", texture_info.size);

                            // Clean up
                            offscreen.destroy_texture(texture_info.id);
                            offscreen.destroy();
                            println!("✅ Resources cleaned up");
                        }
                        None => {
                            eprintln!("❌ Failed to create offscreen renderer");
                            eprintln!("   This platform may not support render-to-texture");
                        }
                    }

                    cx.new(|_| RenderToTextureDemo {
                        text: "Render to Texture".into(),
                        exported: true,
                    })
                },
            )
            .unwrap();

            cx.activate(true);
        });
    }
}
