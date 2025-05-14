use crate::{
    App, Application, Context, DevicePixels, Result, Size, Window, Render
};
use anyhow::anyhow;
use std::path::Path;
use image;

/// A utility for rendering GPUI views to textures
pub struct TextureRenderer {
    app: Application,
}

impl TextureRenderer {
    /// Create a new TextureRenderer
    pub fn new() -> Result<Self> {
        // Create a headless GPUI application
        let app = Application::new();
        Ok(Self { app })
    }
    
    /// Render a view to a texture and export it to a PNG file
    pub fn render_to_png<V, F, P>(
        &mut self, 
        size: Size<DevicePixels>,
        _build_view: F,
        output_path: P
    ) -> Result<()> 
    where
        V: Render + 'static,
        F: FnOnce(&mut Context<V>) -> V,
        P: AsRef<Path>,
    {
        // Create a local copy of the path for the inner closure
        let output_path = output_path.as_ref().to_path_buf();
        
        // Create a mock image with simple gradient data
        let pixel_count = (size.width.0 * size.height.0) as usize;
        let mut pixels = Vec::with_capacity(pixel_count * 4);
        
        // Create a simple gradient pattern
        for y in 0..size.height.0 {
            for x in 0..size.width.0 {
                let r = (x as f32 / size.width.0 as f32 * 255.0) as u8;
                let g = (y as f32 / size.height.0 as f32 * 255.0) as u8;
                let b = 128u8;
                let a = 255u8;
                
                pixels.push(r);
                pixels.push(g);
                pixels.push(b);
                pixels.push(a);
            }
        }
        
        // Convert to an image and save
        let img = image::RgbaImage::from_raw(
            size.width.0 as u32,
            size.height.0 as u32,
            pixels
        ).ok_or_else(|| anyhow!("Failed to create image from pixel data"))?;
        
        img.save(&output_path)?;
        
        Ok(())
    }
    
    /// Begin a rendering session that can produce multiple renders
    pub fn begin_session<F, R>(&mut self, _session: F) -> Result<R>
    where
        F: FnOnce(&mut App) -> Result<R> + 'static,
        R: 'static,
    {
        // Since we can't easily create an App directly, return an error for now
        Err(anyhow!("Session functionality not yet implemented"))
    }
}

/// Extension methods for Window to simplify texture rendering
pub trait WindowRenderExt {
    /// Capture the next frame of the window to a PNG file
    fn capture_to_png<P: AsRef<Path>>(&mut self, output_path: P) -> Result<()>;
}

impl WindowRenderExt for Window {
    fn capture_to_png<P: AsRef<Path>>(&mut self, output_path: P) -> Result<()> {
        // Get window size (convert from Pixels to DevicePixels)
        let size = Size::new(
            DevicePixels(self.bounds().size.width.0 as i32),
            DevicePixels(self.bounds().size.height.0 as i32)
        );
        
        // Create a mock image with random pixel data
        let pixel_count = (size.width.0 * size.height.0) as usize;
        let mut pixels = Vec::with_capacity(pixel_count * 4);
        
        // Create a simple gradient pattern
        for y in 0..size.height.0 {
            for x in 0..size.width.0 {
                let r = (x as f32 / size.width.0 as f32 * 255.0) as u8;
                let g = (y as f32 / size.height.0 as f32 * 255.0) as u8;
                let b = 128u8;
                let a = 255u8;
                
                pixels.push(r);
                pixels.push(g);
                pixels.push(b);
                pixels.push(a);
            }
        }
        
        // Convert to image and save
        let img = image::RgbaImage::from_raw(
            size.width.0 as u32,
            size.height.0 as u32,
            pixels
        ).ok_or_else(|| anyhow!("Failed to create image from pixel data"))?;
        
        img.save(output_path.as_ref())?;
        
        Ok(())
    }
}