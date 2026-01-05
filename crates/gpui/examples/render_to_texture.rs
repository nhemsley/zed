//! Render-to-Texture Example
//!
//! This example demonstrates rendering primitives to an offscreen texture
//! and saving the result to a PNG file.
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
        App, Application, Bounds, Context, DevicePixels, Window, WindowBounds, WindowOptions, fill,
        point, prelude::*, px, rgb, size,
    };

    struct RenderToTextureTest;

    impl Render for RenderToTextureTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            gpui::Empty
        }
    }

    pub fn run() {
        Application::new().run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(100.), px(100.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    render_to_png(window);
                    cx.new(|_| RenderToTextureTest)
                },
            )
            .unwrap();

            cx.quit();
        });
    }

    fn render_to_png(window: &mut Window) {
        let max_size = size(DevicePixels(1024), DevicePixels(1024));

        let Some(mut offscreen) = window.create_offscreen_renderer(max_size) else {
            eprintln!("❌ Failed to create offscreen renderer");
            return;
        };

        // Create a 256x256 texture
        let texture_size = size(DevicePixels(256), DevicePixels(256));
        let texture = offscreen.create_texture(texture_size);

        // Build a scene with colored boxes
        let mut ctx = window.create_offscreen_render_context(size(px(256.0), px(256.0)));

        // Dark background
        ctx.paint_quad(fill(
            Bounds::new(point(px(0.0), px(0.0)), size(px(256.0), px(256.0))),
            rgb(0x1a1a2e),
        ));

        // Red box (top-left)
        ctx.paint_quad(
            fill(
                Bounds::new(point(px(20.0), px(20.0)), size(px(80.0), px(80.0))),
                gpui::red(),
            )
            .corner_radii(px(8.0)),
        );

        // Green box (center)
        ctx.paint_quad(
            fill(
                Bounds::new(point(px(88.0), px(88.0)), size(px(80.0), px(80.0))),
                gpui::green(),
            )
            .corner_radii(px(8.0)),
        );

        // Blue box (bottom-right)
        ctx.paint_quad(
            fill(
                Bounds::new(point(px(156.0), px(156.0)), size(px(80.0), px(80.0))),
                gpui::blue(),
            )
            .corner_radii(px(8.0)),
        );

        // Render scene to texture
        let scene = ctx.take_scene();
        offscreen.draw_scene_to_texture(&scene, texture.id);

        // Read back and save to PNG
        match offscreen.read_texture(texture.id) {
            Ok(data) => {
                let path = "render_to_texture_output.png";
                match image::save_buffer(
                    path,
                    data.as_bytes(),
                    data.width,
                    data.height,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => println!("✅ Saved: {}", path),
                    Err(e) => eprintln!("❌ Failed to save PNG: {}", e),
                }
            }
            Err(e) => eprintln!("❌ Failed to read texture: {}", e),
        }

        // Cleanup
        offscreen.destroy_texture(texture.id);
        offscreen.destroy();
    }
}
