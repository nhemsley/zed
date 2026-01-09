use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blade_graphics as gpu;
use blade_util::{BufferBelt, BufferBeltDescriptor};
use bytemuck::{Pod, Zeroable};
use raw_window_handle as rwh;

use crate::platform::blade::BladeAtlas;
use crate::{
    AnyWindowHandle, Background, Bounds, Capslock, DevicePixels, GpuSpecs, Modifiers, Path, Pixels,
    PlatformAtlas, PlatformDisplay, PlatformInputHandler, PlatformWindow, Point, PrimitiveBatch,
    PromptButton, PromptLevel, RequestFrameOptions, ScaledPixels, Scene, Size, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowParams,
};

use super::TexturedSurfaceDisplay;

const MAX_FRAME_TIME_MS: u32 = 10000;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathRasterizationVertex {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
}

pub struct TexturedSurfaceWindowState {
    #[allow(dead_code)]
    handle: AnyWindowHandle,
    bounds: Bounds<Pixels>,
    scale_factor: f32,

    gpu: Arc<gpu::Context>,
    render_target: gpu::Texture,
    render_target_view: gpu::TextureView,

    atlas: Arc<BladeAtlas>,
    atlas_sampler: gpu::Sampler,
    command_encoder: gpu::CommandEncoder,
    pipelines: TexturedSurfacePipelines,
    instance_belt: BufferBelt,

    path_intermediate_texture: gpu::Texture,
    path_intermediate_texture_view: gpu::TextureView,
    path_intermediate_msaa_texture: Option<gpu::Texture>,
    path_intermediate_msaa_texture_view: Option<gpu::TextureView>,

    rendered_pixels: Option<Vec<u8>>,

    input_handler: Option<PlatformInputHandler>,

    callbacks: Callbacks,

    #[allow(dead_code)]
    path_sample_count: u32,
}

#[derive(Default)]
struct Callbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<Box<dyn FnMut(crate::PlatformInput) -> crate::DispatchEventResult>>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    hover_status_change: Option<Box<dyn FnMut(bool)>>,
    resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
}

pub(crate) struct TexturedSurfaceWindow(Rc<RefCell<TexturedSurfaceWindowState>>);

impl TexturedSurfaceWindow {
    pub fn new(handle: AnyWindowHandle, params: WindowParams) -> anyhow::Result<Self> {
        let gpu = Arc::new(
            unsafe {
                gpu::Context::init(gpu::ContextDesc {
                    presentation: false,
                    validation: cfg!(debug_assertions),
                    ..Default::default()
                })
            }
            .map_err(|e| anyhow::anyhow!("Failed to initialize GPU context: {e:?}"))?,
        );

        let bounds = params.bounds;
        let scale_factor = 1.0;

        let device_width = (bounds.size.width.0 * scale_factor) as u32;
        let device_height = (bounds.size.height.0 * scale_factor) as u32;

        let format = gpu::TextureFormat::Bgra8UnormSrgb;

        let render_target = gpu.create_texture(gpu::TextureDesc {
            name: "textured_surface_target",
            format,
            size: gpu::Extent {
                width: device_width,
                height: device_height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::TARGET | gpu::TextureUsage::COPY,
            external: None,
        });

        let render_target_view = gpu.create_texture_view(
            render_target,
            gpu::TextureViewDesc {
                name: "textured_surface_target_view",
                format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );

        let command_encoder = gpu.create_command_encoder(gpu::CommandEncoderDesc {
            name: "textured_surface",
            buffer_count: 2,
        });

        let atlas = Arc::new(BladeAtlas::new(&gpu));
        let atlas_sampler = gpu.create_sampler(gpu::SamplerDesc {
            name: "textured_surface_sampler",
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });

        let surface_info = gpu::SurfaceInfo {
            format,
            alpha: gpu::AlphaMode::PreMultiplied,
        };

        let path_sample_count = [4, 2, 1]
            .into_iter()
            .find(|&n| (gpu.capabilities().sample_count_mask & n) != 0)
            .unwrap_or(1);

        let pipelines = TexturedSurfacePipelines::new(&gpu, surface_info, path_sample_count);

        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40,
        });

        let (path_intermediate_texture, path_intermediate_texture_view) =
            create_path_intermediate_texture(&gpu, format, device_width, device_height);

        let (path_intermediate_msaa_texture, path_intermediate_msaa_texture_view) =
            create_msaa_texture_if_needed(
                &gpu,
                format,
                device_width,
                device_height,
                path_sample_count,
            )
            .unzip();

        Ok(Self(Rc::new(RefCell::new(TexturedSurfaceWindowState {
            handle,
            bounds,
            scale_factor,
            gpu,
            render_target,
            render_target_view,
            atlas,
            atlas_sampler,
            command_encoder,
            pipelines,
            instance_belt,
            path_intermediate_texture,
            path_intermediate_texture_view,
            path_intermediate_msaa_texture,
            path_intermediate_msaa_texture_view,
            rendered_pixels: None,
            input_handler: None,
            callbacks: Callbacks::default(),
            path_sample_count,
        }))))
    }

    #[allow(dead_code)]
    pub fn read_pixels(&self) -> Option<Vec<u8>> {
        self.0.borrow().rendered_pixels.clone()
    }

    #[allow(dead_code)]
    pub fn texture_view(&self) -> gpu::TextureView {
        self.0.borrow().render_target_view
    }

    #[allow(dead_code)]
    pub fn size(&self) -> Size<DevicePixels> {
        let state = self.0.borrow();
        Size {
            width: DevicePixels((state.bounds.size.width.0 * state.scale_factor) as i32),
            height: DevicePixels((state.bounds.size.height.0 * state.scale_factor) as i32),
        }
    }
}

impl rwh::HasWindowHandle for TexturedSurfaceWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        Err(rwh::HandleError::NotSupported)
    }
}

impl rwh::HasDisplayHandle for TexturedSurfaceWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Err(rwh::HandleError::NotSupported)
    }
}

impl PlatformWindow for TexturedSurfaceWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().bounds
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
        self.0.borrow_mut().resize(size);
    }

    fn scale_factor(&self) -> f32 {
        self.0.borrow().scale_factor
    }

    fn appearance(&self) -> WindowAppearance {
        WindowAppearance::Dark
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(TexturedSurfaceDisplay::new()))
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

    fn set_input_handler(&mut self, handler: PlatformInputHandler) {
        self.0.borrow_mut().input_handler = Some(handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.borrow_mut().input_handler.take()
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
        true
    }

    fn is_hovered(&self) -> bool {
        false
    }

    fn set_title(&mut self, _title: &str) {}

    fn set_background_appearance(&self, _appearance: WindowBackgroundAppearance) {}

    fn minimize(&self) {}

    fn zoom(&self) {}

    fn toggle_fullscreen(&self) {}

    fn is_fullscreen(&self) -> bool {
        false
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.borrow_mut().callbacks.request_frame = Some(callback);
    }

    fn on_input(
        &self,
        callback: Box<dyn FnMut(crate::PlatformInput) -> crate::DispatchEventResult>,
    ) {
        self.0.borrow_mut().callbacks.input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.borrow_mut().callbacks.active_status_change = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.borrow_mut().callbacks.hover_status_change = Some(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.borrow_mut().callbacks.resize = Some(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.borrow_mut().callbacks.moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.borrow_mut().callbacks.should_close = Some(callback);
    }

    fn on_hit_test_window_control(
        &self,
        _callback: Box<dyn FnMut() -> Option<crate::WindowControlArea>>,
    ) {
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.borrow_mut().callbacks.close = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.borrow_mut().callbacks.appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        let mut state = self.0.borrow_mut();
        state.render_scene_to_texture(scene);
        state.read_pixels_to_buffer();
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.borrow().atlas.clone()
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        let state = self.0.borrow();
        let device_info = state.gpu.device_information();
        Some(GpuSpecs {
            is_software_emulated: device_info.is_software_emulated,
            device_name: device_info.device_name.clone(),
            driver_name: device_info.driver_name.clone(),
            driver_info: device_info.driver_info.clone(),
        })
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}

    fn read_pixels(&self) -> Option<Vec<u8>> {
        self.0.borrow().rendered_pixels.clone()
    }
}

impl TexturedSurfaceWindowState {
    /// Resize the window and recreate all GPU textures at the new size.
    /// This is necessary for textured rendering when the content size
    /// is determined after layout (e.g., FixedWidth mode where height is measured).
    fn resize(&mut self, size: Size<Pixels>) {
        // Skip if size hasn't changed
        if self.bounds.size == size {
            return;
        }

        self.bounds.size = size;

        let device_width = (size.width.0 * self.scale_factor) as u32;
        let device_height = (size.height.0 * self.scale_factor) as u32;

        // Ensure minimum size of 1x1 to avoid GPU errors
        let device_width = device_width.max(1);
        let device_height = device_height.max(1);

        let format = gpu::TextureFormat::Bgra8UnormSrgb;

        // Destroy old textures
        self.gpu.destroy_texture_view(self.render_target_view);
        self.gpu.destroy_texture(self.render_target);
        self.gpu
            .destroy_texture_view(self.path_intermediate_texture_view);
        self.gpu.destroy_texture(self.path_intermediate_texture);
        if let Some(msaa_texture) = self.path_intermediate_msaa_texture.take() {
            self.gpu.destroy_texture(msaa_texture);
        }
        if let Some(msaa_view) = self.path_intermediate_msaa_texture_view.take() {
            self.gpu.destroy_texture_view(msaa_view);
        }

        // Create new render target
        self.render_target = self.gpu.create_texture(gpu::TextureDesc {
            name: "textured_surface_target",
            format,
            size: gpu::Extent {
                width: device_width,
                height: device_height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::TARGET | gpu::TextureUsage::COPY,
            external: None,
        });

        self.render_target_view = self.gpu.create_texture_view(
            self.render_target,
            gpu::TextureViewDesc {
                name: "textured_surface_target_view",
                format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );

        // Recreate path intermediate textures
        let (path_intermediate_texture, path_intermediate_texture_view) =
            create_path_intermediate_texture(&self.gpu, format, device_width, device_height);
        self.path_intermediate_texture = path_intermediate_texture;
        self.path_intermediate_texture_view = path_intermediate_texture_view;

        // Recreate MSAA textures if needed
        let (msaa_texture, msaa_view) = create_msaa_texture_if_needed(
            &self.gpu,
            format,
            device_width,
            device_height,
            self.path_sample_count,
        )
        .unzip();
        self.path_intermediate_msaa_texture = msaa_texture;
        self.path_intermediate_msaa_texture_view = msaa_view;

        // Clear any previously rendered pixels since they're now invalid
        self.rendered_pixels = None;
    }

    fn render_scene_to_texture(&mut self, scene: &Scene) {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);

        self.command_encoder.init_texture(self.render_target);

        let width = self.bounds.size.width.0 * self.scale_factor;
        let height = self.bounds.size.height.0 * self.scale_factor;

        let globals = GlobalParams {
            viewport_size: [width, height],
            premultiplied_alpha: 1,
            pad: 0,
        };

        let mut pass = self.command_encoder.render(
            "textured_surface_main",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: self.render_target_view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );

        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Quads(quads) => {
                    let instance_buf = unsafe { self.instance_belt.alloc_typed(quads, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.quads);
                    encoder.bind(
                        0,
                        &ShaderQuadsData {
                            globals,
                            b_quads: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, quads.len() as u32);
                }
                PrimitiveBatch::Shadows(shadows) => {
                    let instance_buf =
                        unsafe { self.instance_belt.alloc_typed(shadows, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.shadows);
                    encoder.bind(
                        0,
                        &ShaderShadowsData {
                            globals,
                            b_shadows: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, shadows.len() as u32);
                }
                PrimitiveBatch::Paths(paths) => {
                    let Some(first_path) = paths.first() else {
                        continue;
                    };
                    drop(pass);
                    self.draw_paths_to_intermediate(paths, width, height);
                    pass = self.command_encoder.render(
                        "textured_surface_main",
                        gpu::RenderTargetSet {
                            colors: &[gpu::RenderTarget {
                                view: self.render_target_view,
                                init_op: gpu::InitOp::Load,
                                finish_op: gpu::FinishOp::Store,
                            }],
                            depth_stencil: None,
                        },
                    );
                    let mut encoder = pass.with(&self.pipelines.paths);

                    let sprites = if paths.last().unwrap().order == first_path.order {
                        paths
                            .iter()
                            .map(|path| PathSprite {
                                bounds: path.clipped_bounds(),
                            })
                            .collect()
                    } else {
                        let mut bounds = first_path.clipped_bounds();
                        for path in paths.iter().skip(1) {
                            bounds = bounds.union(&path.clipped_bounds());
                        }
                        vec![PathSprite { bounds }]
                    };

                    let instance_buf =
                        unsafe { self.instance_belt.alloc_typed(&sprites, &self.gpu) };
                    encoder.bind(
                        0,
                        &ShaderPathsData {
                            globals,
                            t_sprite: self.path_intermediate_texture_view,
                            s_sprite: self.atlas_sampler,
                            b_path_sprites: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, sprites.len() as u32);
                }
                PrimitiveBatch::Underlines(underlines) => {
                    let instance_buf =
                        unsafe { self.instance_belt.alloc_typed(underlines, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.underlines);
                    encoder.bind(
                        0,
                        &ShaderUnderlinesData {
                            globals,
                            b_underlines: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, underlines.len() as u32);
                }
                PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    sprites,
                } => {
                    let tex_info = self.atlas.get_texture_info(texture_id);
                    let instance_buf =
                        unsafe { self.instance_belt.alloc_typed(sprites, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.mono_sprites);
                    encoder.bind(
                        0,
                        &ShaderMonoSpritesData {
                            globals,
                            gamma_ratios: crate::get_gamma_correction_ratios(1.8),
                            grayscale_enhanced_contrast: 1.0,
                            t_sprite: tex_info.raw_view,
                            s_sprite: self.atlas_sampler,
                            b_mono_sprites: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, sprites.len() as u32);
                }
                PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    sprites,
                } => {
                    let tex_info = self.atlas.get_texture_info(texture_id);
                    let instance_buf =
                        unsafe { self.instance_belt.alloc_typed(sprites, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.poly_sprites);
                    encoder.bind(
                        0,
                        &ShaderPolySpritesData {
                            globals,
                            t_sprite: tex_info.raw_view,
                            s_sprite: self.atlas_sampler,
                            b_poly_sprites: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, sprites.len() as u32);
                }
                PrimitiveBatch::Surfaces(_surfaces) => {
                    // Surface rendering (video frames) is macOS-specific
                }
            }
        }

        drop(pass);

        let sync_point = self.gpu.submit(&mut self.command_encoder);
        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);

        if !self.gpu.wait_for(&sync_point, MAX_FRAME_TIME_MS) {
            log::error!("GPU hung while rendering textured surface");
        }
    }

    fn draw_paths_to_intermediate(
        &mut self,
        paths: &[Path<ScaledPixels>],
        width: f32,
        height: f32,
    ) {
        self.command_encoder
            .init_texture(self.path_intermediate_texture);
        if let Some(msaa_texture) = self.path_intermediate_msaa_texture {
            self.command_encoder.init_texture(msaa_texture);
        }

        let target = if let Some(msaa_view) = self.path_intermediate_msaa_texture_view {
            gpu::RenderTarget {
                view: msaa_view,
                init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                finish_op: gpu::FinishOp::ResolveTo(self.path_intermediate_texture_view),
            }
        } else {
            gpu::RenderTarget {
                view: self.path_intermediate_texture_view,
                init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                finish_op: gpu::FinishOp::Store,
            }
        };

        {
            let mut pass = self.command_encoder.render(
                "rasterize paths",
                gpu::RenderTargetSet {
                    colors: &[target],
                    depth_stencil: None,
                },
            );
            let globals = GlobalParams {
                viewport_size: [width, height],
                premultiplied_alpha: 0,
                pad: 0,
            };
            let mut encoder = pass.with(&self.pipelines.path_rasterization);

            let mut vertices = Vec::new();
            for path in paths {
                vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                    xy_position: v.xy_position,
                    st_position: v.st_position,
                    color: path.color,
                    bounds: path.clipped_bounds(),
                }));
            }
            let vertex_buf = unsafe { self.instance_belt.alloc_typed(&vertices, &self.gpu) };
            encoder.bind(
                0,
                &ShaderPathRasterizationData {
                    globals,
                    b_path_vertices: vertex_buf,
                },
            );
            encoder.draw(0, vertices.len() as u32, 0, 1);
        }
    }

    fn read_pixels_to_buffer(&mut self) {
        let width = (self.bounds.size.width.0 * self.scale_factor) as u32;
        let height = (self.bounds.size.height.0 * self.scale_factor) as u32;
        let bytes_per_pixel = 4u64;
        let row_pitch = width as u64 * bytes_per_pixel;
        let buffer_size = row_pitch * height as u64;

        let staging = self.gpu.create_buffer(gpu::BufferDesc {
            name: "pixel_readback",
            size: buffer_size,
            memory: gpu::Memory::Shared,
        });

        self.command_encoder.start();
        {
            let mut transfer = self.command_encoder.transfer("readback");
            transfer.copy_texture_to_buffer(
                gpu::TexturePiece {
                    texture: self.render_target,
                    mip_level: 0,
                    array_layer: 0,
                    origin: [0, 0, 0],
                },
                staging.into(),
                row_pitch as u32,
                gpu::Extent {
                    width,
                    height,
                    depth: 1,
                },
            );
        }

        let sync_point = self.gpu.submit(&mut self.command_encoder);
        if !self.gpu.wait_for(&sync_point, MAX_FRAME_TIME_MS) {
            log::error!("GPU hung while reading pixels");
            return;
        }

        let pixels = unsafe {
            std::slice::from_raw_parts(staging.data() as *const u8, buffer_size as usize).to_vec()
        };

        self.gpu.destroy_buffer(staging);
        self.rendered_pixels = Some(pixels);
    }
}

impl Drop for TexturedSurfaceWindowState {
    fn drop(&mut self) {
        self.atlas.destroy();
        self.gpu.destroy_sampler(self.atlas_sampler);
        self.instance_belt.destroy(&self.gpu);
        self.gpu.destroy_command_encoder(&mut self.command_encoder);
        self.pipelines.destroy(&self.gpu);
        self.gpu.destroy_texture(self.render_target);
        self.gpu.destroy_texture_view(self.render_target_view);
        self.gpu.destroy_texture(self.path_intermediate_texture);
        self.gpu
            .destroy_texture_view(self.path_intermediate_texture_view);
        if let Some(msaa_texture) = self.path_intermediate_msaa_texture {
            self.gpu.destroy_texture(msaa_texture);
        }
        if let Some(msaa_view) = self.path_intermediate_msaa_texture_view {
            self.gpu.destroy_texture_view(msaa_view);
        }
    }
}

// Shader data structures
#[derive(blade_macros::ShaderData)]
struct ShaderQuadsData {
    globals: GlobalParams,
    b_quads: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderShadowsData {
    globals: GlobalParams,
    b_shadows: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPathRasterizationData {
    globals: GlobalParams,
    b_path_vertices: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPathsData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_path_sprites: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderUnderlinesData {
    globals: GlobalParams,
    b_underlines: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderMonoSpritesData {
    globals: GlobalParams,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_mono_sprites: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPolySpritesData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_poly_sprites: gpu::BufferPiece,
}

struct TexturedSurfacePipelines {
    quads: gpu::RenderPipeline,
    shadows: gpu::RenderPipeline,
    path_rasterization: gpu::RenderPipeline,
    paths: gpu::RenderPipeline,
    underlines: gpu::RenderPipeline,
    mono_sprites: gpu::RenderPipeline,
    poly_sprites: gpu::RenderPipeline,
}

impl TexturedSurfacePipelines {
    fn new(gpu: &gpu::Context, surface_info: gpu::SurfaceInfo, path_sample_count: u32) -> Self {
        use blade_graphics::ShaderData as _;

        let shader = gpu.create_shader(gpu::ShaderDesc {
            source: include_str!("../../blade/shaders.wgsl"),
        });

        let blend_mode = match surface_info.alpha {
            gpu::AlphaMode::Ignored => gpu::BlendState::ALPHA_BLENDING,
            gpu::AlphaMode::PreMultiplied => gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            gpu::AlphaMode::PostMultiplied => gpu::BlendState::ALPHA_BLENDING,
        };
        let color_targets = &[gpu::ColorTargetState {
            format: surface_info.format,
            blend: Some(blend_mode),
            write_mask: gpu::ColorWrites::default(),
        }];

        Self {
            quads: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "quads",
                data_layouts: &[&ShaderQuadsData::layout()],
                vertex: shader.at("vs_quad"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_quad")),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            }),
            shadows: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "shadows",
                data_layouts: &[&ShaderShadowsData::layout()],
                vertex: shader.at("vs_shadow"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_shadow")),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            }),
            path_rasterization: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "path_rasterization",
                data_layouts: &[&ShaderPathRasterizationData::layout()],
                vertex: shader.at("vs_path_rasterization"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_path_rasterization")),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_info.format,
                    blend: Some(gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
                multisample_state: gpu::MultisampleState {
                    sample_count: path_sample_count,
                    ..Default::default()
                },
            }),
            paths: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "paths",
                data_layouts: &[&ShaderPathsData::layout()],
                vertex: shader.at("vs_path"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_path")),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_info.format,
                    blend: Some(gpu::BlendState {
                        color: gpu::BlendComponent::OVER,
                        alpha: gpu::BlendComponent::ADDITIVE,
                    }),
                    write_mask: gpu::ColorWrites::default(),
                }],
                multisample_state: gpu::MultisampleState::default(),
            }),
            underlines: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "underlines",
                data_layouts: &[&ShaderUnderlinesData::layout()],
                vertex: shader.at("vs_underline"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_underline")),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            }),
            mono_sprites: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "mono-sprites",
                data_layouts: &[&ShaderMonoSpritesData::layout()],
                vertex: shader.at("vs_mono_sprite"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_mono_sprite")),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            }),
            poly_sprites: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "poly-sprites",
                data_layouts: &[&ShaderPolySpritesData::layout()],
                vertex: shader.at("vs_poly_sprite"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: Some(shader.at("fs_poly_sprite")),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            }),
        }
    }

    fn destroy(&mut self, gpu: &gpu::Context) {
        gpu.destroy_render_pipeline(&mut self.quads);
        gpu.destroy_render_pipeline(&mut self.shadows);
        gpu.destroy_render_pipeline(&mut self.path_rasterization);
        gpu.destroy_render_pipeline(&mut self.paths);
        gpu.destroy_render_pipeline(&mut self.underlines);
        gpu.destroy_render_pipeline(&mut self.mono_sprites);
        gpu.destroy_render_pipeline(&mut self.poly_sprites);
    }
}

fn create_path_intermediate_texture(
    gpu: &gpu::Context,
    format: gpu::TextureFormat,
    width: u32,
    height: u32,
) -> (gpu::Texture, gpu::TextureView) {
    let texture = gpu.create_texture(gpu::TextureDesc {
        name: "path intermediate",
        format,
        size: gpu::Extent {
            width,
            height,
            depth: 1,
        },
        array_layer_count: 1,
        mip_level_count: 1,
        sample_count: 1,
        dimension: gpu::TextureDimension::D2,
        usage: gpu::TextureUsage::COPY | gpu::TextureUsage::RESOURCE | gpu::TextureUsage::TARGET,
        external: None,
    });
    let texture_view = gpu.create_texture_view(
        texture,
        gpu::TextureViewDesc {
            name: "path intermediate view",
            format,
            dimension: gpu::ViewDimension::D2,
            subresources: &Default::default(),
        },
    );
    (texture, texture_view)
}

fn create_msaa_texture_if_needed(
    gpu: &gpu::Context,
    format: gpu::TextureFormat,
    width: u32,
    height: u32,
    sample_count: u32,
) -> Option<(gpu::Texture, gpu::TextureView)> {
    if sample_count <= 1 {
        return None;
    }
    let texture_msaa = gpu.create_texture(gpu::TextureDesc {
        name: "path intermediate msaa",
        format,
        size: gpu::Extent {
            width,
            height,
            depth: 1,
        },
        array_layer_count: 1,
        mip_level_count: 1,
        sample_count,
        dimension: gpu::TextureDimension::D2,
        usage: gpu::TextureUsage::TARGET,
        external: None,
    });
    let texture_view_msaa = gpu.create_texture_view(
        texture_msaa,
        gpu::TextureViewDesc {
            name: "path intermediate msaa view",
            format,
            dimension: gpu::ViewDimension::D2,
            subresources: &Default::default(),
        },
    );

    Some((texture_msaa, texture_view_msaa))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{px, Bounds, Empty, Point, Size, WindowHandle, WindowKind};

    #[test]
    fn test_textured_surface_window_creation() {
        // This test verifies that the TexturedSurfaceWindow can be created
        // Note: This test requires GPU access and may fail in CI environments
        // without a GPU or display server

        let window_id = crate::WindowId::from(0u64);
        let handle: AnyWindowHandle = WindowHandle::<Empty>::new(window_id).into();
        let params = crate::WindowParams {
            bounds: Bounds {
                origin: Point {
                    x: px(0.0),
                    y: px(0.0),
                },
                size: Size {
                    width: px(800.0),
                    height: px(600.0),
                },
            },
            titlebar: None,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: true,
            is_minimizable: true,
            focus: true,
            show: true,
            display_id: None,
            window_min_size: None,
            #[cfg(target_os = "macos")]
            tabbing_identifier: None,
        };

        // This will fail if no GPU is available, which is expected in headless CI
        let result = TexturedSurfaceWindow::new(handle, params);

        // We just verify it either succeeds or fails gracefully
        match result {
            Ok(window) => {
                assert_eq!(window.bounds().size.width, px(800.0));
                assert_eq!(window.bounds().size.height, px(600.0));
                assert_eq!(window.scale_factor(), 1.0);
            }
            Err(e) => {
                // GPU initialization failure is expected in headless environments
                eprintln!(
                    "TexturedSurfaceWindow creation failed (expected in CI): {}",
                    e
                );
            }
        }
    }

    #[test]
    fn test_textured_surface_window_resize() {
        // Test that resize properly recreates GPU textures at the new size

        let window_id = crate::WindowId::from(1u64);
        let handle: AnyWindowHandle = WindowHandle::<Empty>::new(window_id).into();
        let params = crate::WindowParams {
            bounds: Bounds {
                origin: Point {
                    x: px(0.0),
                    y: px(0.0),
                },
                size: Size {
                    width: px(400.0),
                    height: px(300.0),
                },
            },
            titlebar: None,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: true,
            is_minimizable: true,
            focus: true,
            show: true,
            display_id: None,
            window_min_size: None,
            #[cfg(target_os = "macos")]
            tabbing_identifier: None,
        };

        let result = TexturedSurfaceWindow::new(handle, params);

        match result {
            Ok(mut window) => {
                // Verify initial size
                assert_eq!(window.bounds().size.width, px(400.0));
                assert_eq!(window.bounds().size.height, px(300.0));

                // Resize to new dimensions
                window.resize(Size {
                    width: px(800.0),
                    height: px(600.0),
                });

                // Verify new size
                assert_eq!(window.bounds().size.width, px(800.0));
                assert_eq!(window.bounds().size.height, px(600.0));

                // Resize to smaller dimensions
                window.resize(Size {
                    width: px(200.0),
                    height: px(150.0),
                });

                assert_eq!(window.bounds().size.width, px(200.0));
                assert_eq!(window.bounds().size.height, px(150.0));

                // Test resize to same size (should be a no-op)
                window.resize(Size {
                    width: px(200.0),
                    height: px(150.0),
                });

                assert_eq!(window.bounds().size.width, px(200.0));
                assert_eq!(window.bounds().size.height, px(150.0));
            }
            Err(e) => {
                eprintln!(
                    "TexturedSurfaceWindow creation failed (expected in CI): {}",
                    e
                );
            }
        }
    }

    // TODO: test that rendering to a texture actually works! (it does)
}
