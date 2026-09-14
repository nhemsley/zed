//! Windows that render into an offscreen GPU texture instead of a compositor
//! surface.
//!
//! A texture window is a complete GPUI window (own focus, hit-testing, input
//! handlers), but the platform never drives it: it has no `wl_surface`, no
//! frame callback and receives no input. A host window forwards input to it,
//! draws it from its own frame loop, and composites the resulting texture.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gpui_util::ResultExt as _;
use gpui_wgpu::{GpuContext, WgpuHeadlessRenderer};

use gpui::{
    Bounds, Capslock, DispatchEventResult, ExternalTexture, GpuSpecs, Modifiers, Pixels,
    PlatformAtlas, PlatformDisplay, PlatformHeadlessRenderer as _, PlatformInput,
    PlatformInputHandler, PlatformWindow, Point, PromptButton, PromptLevel, RequestFrameOptions,
    Scene, Size, TextureWindowOptions, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea,
};

struct TextureWindowState {
    renderer: WgpuHeadlessRenderer,
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    appearance: WindowAppearance,
    background_appearance: WindowBackgroundAppearance,
    input_handler: Option<PlatformInputHandler>,
    title: String,
}

pub(crate) struct TextureWindow {
    state: RefCell<TextureWindowState>,
    resize_callback: RefCell<Option<Box<dyn FnMut(Size<Pixels>, f32)>>>,
}

impl raw_window_handle::HasWindowHandle for TextureWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::NotSupported)
    }
}

impl raw_window_handle::HasDisplayHandle for TextureWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::NotSupported)
    }
}

impl TextureWindow {
    /// `instance` is used only if `gpu_context` has not been initialized yet;
    /// it must be bound to the display server when real windows will share
    /// the context, since their surfaces are created through it.
    pub(crate) fn new(
        gpu_context: GpuContext,
        instance: Option<gpui_wgpu::wgpu::Instance>,
        options: TextureWindowOptions,
        appearance: WindowAppearance,
    ) -> Result<Self> {
        anyhow::ensure!(
            options.scale_factor.is_finite() && options.scale_factor > 0.0,
            "texture window scale factor must be positive, got {}",
            options.scale_factor
        );
        let renderer = WgpuHeadlessRenderer::new_with_instance(gpu_context, instance)?;
        Ok(Self {
            state: RefCell::new(TextureWindowState {
                renderer,
                bounds: Bounds {
                    origin: Point::default(),
                    size: options.size,
                },
                scale_factor: options.scale_factor,
                appearance,
                background_appearance: options.window_background,
                input_handler: None,
                title: String::new(),
            }),
            resize_callback: RefCell::new(None),
        })
    }

    /// The most recently drawn frame as straight-alpha RGBA.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn read_frame(&self) -> Result<image::RgbaImage> {
        self.state.borrow().renderer.read_frame()
    }
}

impl PlatformWindow for TextureWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.state.borrow().bounds
    }

    fn is_maximized(&self) -> bool {
        false
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Windowed(self.bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.bounds().size
    }

    fn resize(&mut self, size: Size<Pixels>) {
        let scale_factor = {
            let mut state = self.state.borrow_mut();
            if state.bounds.size == size {
                return;
            }
            state.bounds.size = size;
            state.scale_factor
        };

        // Unlike compositor windows, nothing else will ever report the new
        // size, so the callback runs synchronously.
        let callback = self.resize_callback.borrow_mut().take();
        if let Some(mut callback) = callback {
            callback(size, scale_factor);
            *self.resize_callback.borrow_mut() = Some(callback);
        }
    }

    fn scale_factor(&self) -> f32 {
        self.state.borrow().scale_factor
    }

    fn appearance(&self) -> WindowAppearance {
        self.state.borrow().appearance
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        None
    }

    fn mouse_position(&self) -> Point<Pixels> {
        Point::default()
    }

    fn modifiers(&self) -> Modifiers {
        Modifiers::default()
    }

    fn capslock(&self) -> Capslock {
        Capslock::default()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.state.borrow_mut().input_handler = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.state.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {}

    fn is_active(&self) -> bool {
        false
    }

    fn is_hovered(&self) -> bool {
        false
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.state.borrow().background_appearance
    }

    fn set_title(&mut self, title: &str) {
        self.state.borrow_mut().title = title.to_owned();
    }

    fn get_title(&self) -> String {
        self.state.borrow().title.clone()
    }

    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        self.state.borrow_mut().background_appearance = background_appearance;
    }

    fn minimize(&self) {}

    fn zoom(&self) {}

    fn toggle_fullscreen(&self) {}

    fn is_fullscreen(&self) -> bool {
        false
    }

    // No compositor delivers frames, input or status changes to a texture
    // window; the host drives the GPUI `Window` directly.
    fn on_request_frame(&self, _callback: Box<dyn FnMut(RequestFrameOptions)>) {}

    fn on_input(&self, _callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {}

    fn on_active_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}

    fn on_hover_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        *self.resize_callback.borrow_mut() = Some(callback);
    }

    fn on_moved(&self, _callback: Box<dyn FnMut()>) {}

    fn on_should_close(&self, _callback: Box<dyn FnMut() -> bool>) {}

    fn on_close(&self, _callback: Box<dyn FnOnce()>) {}

    fn on_hit_test_window_control(&self, _callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
    }

    fn on_appearance_changed(&self, _callback: Box<dyn FnMut()>) {}

    fn draw(&self, scene: &Scene) {
        let mut state = self.state.borrow_mut();
        let size = state.bounds.size.to_device_pixels(state.scale_factor);
        state.renderer.render_scene(scene, size).log_err();
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.state.borrow().renderer.sprite_atlas()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        self.state.borrow().renderer.supports_dual_source_blending()
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        Some(self.state.borrow().renderer.gpu_specs())
    }

    #[cfg(any(test, feature = "test-support"))]
    fn render_to_image(&self, scene: &Scene) -> Result<image::RgbaImage> {
        self.draw(scene);
        self.read_frame()
    }

    fn external_texture(&self) -> Option<ExternalTexture> {
        self.state.borrow().renderer.external_texture()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Background, BorderStyle, ContentMask, Corners, Edges, Hsla, Quad, ScaledPixels, point, px,
        size,
    };
    use std::cell::Cell;

    fn params(width: f32, height: f32) -> TextureWindowOptions {
        TextureWindowOptions {
            size: size(px(width), px(height)),
            ..TextureWindowOptions::default()
        }
    }

    #[test]
    fn resizes_synchronously_and_draws_into_texture() {
        let gpu_context: GpuContext = Rc::new(RefCell::new(None));
        let mut window =
            match TextureWindow::new(gpu_context, None, params(8., 8.), WindowAppearance::Dark) {
                Ok(window) => window,
                Err(error) => {
                    eprintln!("skipping: no usable GPU adapter ({error:#})");
                    return;
                }
            };

        let resized = Rc::new(Cell::new(None));
        window.on_resize(Box::new({
            let resized = resized.clone();
            move |size, scale_factor| resized.set(Some((size, scale_factor)))
        }));
        window.resize(size(px(16.), px(12.)));
        assert_eq!(resized.get(), Some((size(px(16.), px(12.)), 1.0)));

        let full = Bounds {
            origin: point(ScaledPixels(0.), ScaledPixels(0.)),
            size: size(ScaledPixels(16.), ScaledPixels(12.)),
        };
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            order: 0,
            border_style: BorderStyle::default(),
            bounds: Bounds {
                origin: point(ScaledPixels(0.), ScaledPixels(0.)),
                size: size(ScaledPixels(8.), ScaledPixels(6.)),
            },
            content_mask: ContentMask { bounds: full },
            background: Background::from(Hsla {
                h: 0.,
                s: 1.,
                l: 0.5,
                a: 1.,
            }),
            border_color: Hsla::transparent_black(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
        });
        scene.finish();

        window.draw(&scene);
        let image = window.read_frame().expect("frame readback should succeed");
        assert_eq!(image.dimensions(), (16, 12));
        let inside = image.get_pixel(2, 2).0;
        assert!(
            inside[0] > 200 && inside[1] < 40 && inside[2] < 40 && inside[3] > 200,
            "expected an opaque red pixel, got {inside:?}"
        );
        let outside = image.get_pixel(13, 10).0;
        assert_eq!(
            outside[3], 0,
            "expected a transparent pixel, got {outside:?}"
        );

        if let Ok(directory) = std::env::var("GPUI_TEXTURE_WINDOW_PNG_DIR") {
            let path = std::path::Path::new(&directory).join("texture_window.png");
            image.save(&path).expect("png should be written");
        }
    }
}
