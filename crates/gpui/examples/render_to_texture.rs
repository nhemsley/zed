use gpui::{
    div, rgb, Context, DevicePixels, ParentElement, Render, Size, TextureRenderer, SharedString, Styled
};
use std::env;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // Get the output file path from the command line argument
    let output_path = env::args().nth(1).unwrap_or_else(|| "render_output.png".to_string());
    let output_path = PathBuf::from(output_path);
    
    println!("Rendering to {}", output_path.display());
    
    // Create a texture renderer
    let mut renderer = TextureRenderer::new()?;
    
    // Define the size for the rendered image
    let width = 800;
    let height = 600;
    let size = Size::new(DevicePixels(width), DevicePixels(height));
    
    // Render our view to a PNG file
    renderer.render_to_png(size, |cx| {
        // Create a custom view for rendering
        ExampleView::new("Hello, render-to-texture!", cx)
    }, output_path)?;
    
    println!("Rendering completed successfully");
    Ok(())
}

// A simple view to demonstrate rendering to texture
struct ExampleView {
    message: String,
}

impl ExampleView {
    fn new(message: impl Into<String>, _cx: &mut Context<Self>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Render for ExampleView {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        // Create a simple UI with a colored background and styled text
        div()
            .h_full()
            .w_full()
            .bg(rgb(0x2B2E33))
            .child(
                div()
                    .bg(rgb(0x1E2227))
                    .child(
                        div()
                            .child(SharedString::from(self.message.clone()))
                    )
                    .child(
                        div()
                            .child(SharedString::from("This image was rendered with GPUI's render-to-texture feature"))
                    )
                    .child(
                        div()
                            .child(SharedString::from("Generated from render_to_texture example"))
                    )
            )
    }
}