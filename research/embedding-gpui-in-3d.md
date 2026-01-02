# Embedding GPUI in 3D Environments

This document describes the architecture and considerations for rendering GPUI interfaces as textures that can be displayed within 3D environments.

## Overview

Embedding GPUI in 3D allows 2D user interfaces to be rendered onto surfaces within a 3D scene. The GPUI content is rendered to a texture, which is then sampled by the 3D renderer when drawing geometry.

### Use Cases

1. **VR/AR Interfaces**: Floating UI panels in virtual/augmented reality
2. **In-Game UI**: Computer screens, control panels, terminals in games
3. **Digital Twins**: Interactive dashboards on 3D equipment models
4. **Spatial Computing**: Mixed reality productivity applications
5. **CAD/Design Tools**: Property panels, tool palettes on 3D objects
6. **Training Simulations**: Interactive elements in 3D training environments
7. **Data Visualization**: Interactive 2D charts/controls in 3D data spaces

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         3D Application                               │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                      3D Rendering Engine                        │ │
│  │                  (Bevy, wgpu, Unity, Unreal)                   │ │
│  │                                                                  │ │
│  │  ┌──────────────────────────────────────────────────────────┐  │ │
│  │  │                GPUI Surface Manager                       │  │ │
│  │  │                                                            │  │ │
│  │  │   ┌────────────┐   ┌────────────┐   ┌────────────┐       │  │ │
│  │  │   │  Surface A │   │  Surface B │   │  Surface C │       │  │ │
│  │  │   │  (Menu)    │   │  (Console) │   │  (HUD)     │       │  │ │
│  │  │   │            │   │            │   │            │       │  │ │
│  │  │   │ ┌────────┐ │   │ ┌────────┐ │   │ ┌────────┐ │       │  │ │
│  │  │   │ │Texture │ │   │ │Texture │ │   │ │Texture │ │       │  │ │
│  │  │   │ └────────┘ │   │ └────────┘ │   │ └────────┘ │       │  │ │
│  │  │   └─────┬──────┘   └─────┬──────┘   └─────┬──────┘       │  │ │
│  │  │         │                │                │              │  │ │
│  │  └─────────┼────────────────┼────────────────┼──────────────┘  │ │
│  │            │                │                │                 │ │
│  │            ▼                ▼                ▼                 │ │
│  │  ┌──────────────────────────────────────────────────────────┐  │ │
│  │  │                     3D Scene Graph                        │  │ │
│  │  │                                                            │  │ │
│  │  │   ┌─────────────┐  ┌─────────────┐  ┌─────────────┐      │  │ │
│  │  │   │ World-Space │  │Curved Panel │  │Screen-Space │      │  │ │
│  │  │   │    Quad     │  │  (curved)   │  │  Overlay    │      │  │ │
│  │  │   └─────────────┘  └─────────────┘  └─────────────┘      │  │ │
│  │  │                                                            │  │ │
│  │  └──────────────────────────────────────────────────────────┘  │ │
│  │                              │                                  │ │
│  │                              ▼                                  │ │
│  │  ┌──────────────────────────────────────────────────────────┐  │ │
│  │  │                     Final Render                          │  │ │
│  │  │             (3D scene with embedded UI)                   │  │ │
│  │  └──────────────────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Core Components

### EmbeddedGpuiSurface

Represents a single GPUI interface that can be embedded in 3D:

```rust
/// A GPUI surface that renders to a texture for 3D embedding
pub struct EmbeddedGpuiSurface {
    /// Unique identifier for this surface
    id: SurfaceId,
    
    /// The root view being rendered
    root: AnyView,
    
    /// Size in device pixels
    size: Size<DevicePixels>,
    
    /// Scale factor for rendering quality
    scale_factor: f32,
    
    /// GPU texture containing rendered content
    texture: GpuTexture,
    
    /// Whether content needs re-rendering
    dirty: bool,
    
    /// Current input state
    input_state: SurfaceInputState,
    
    /// Render scheduling mode
    render_mode: RenderMode,
    
    /// Time since last render (for throttling)
    time_since_render: Duration,
    
    /// GPUI rendering context for this surface
    context: EmbeddedContext,
}
```

### GpuiSurfaceManager

Manages multiple GPUI surfaces and coordinates rendering:

```rust
/// Manages multiple GPUI surfaces for 3D embedding
pub struct GpuiSurfaceManager {
    /// All registered surfaces
    surfaces: HashMap<SurfaceId, EmbeddedGpuiSurface>,
    
    /// Currently focused surface (receives keyboard input)
    focused_surface: Option<SurfaceId>,
    
    /// Shared text system across all surfaces
    text_system: Arc<TextSystem>,
    
    /// Shared GPU resources
    gpu_resources: SharedGpuResources,
    
    /// Renderer for GPUI content
    renderer: EmbeddedGpuiRenderer,
}

impl GpuiSurfaceManager {
    /// Create a new surface manager
    pub fn new(gpu: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self>;
    
    /// Create a new surface with a root view
    pub fn create_surface<V: Render>(
        &mut self,
        size: Size<DevicePixels>,
        build_root: impl FnOnce(&mut Context<V>) -> V,
    ) -> Result<SurfaceId>;
    
    /// Remove a surface
    pub fn destroy_surface(&mut self, id: SurfaceId);
    
    /// Get a surface by ID
    pub fn get_surface(&self, id: SurfaceId) -> Option<&EmbeddedGpuiSurface>;
    
    /// Get a surface mutably
    pub fn get_surface_mut(&mut self, id: SurfaceId) -> Option<&mut EmbeddedGpuiSurface>;
    
    /// Update all surfaces (call each frame)
    pub fn update(&mut self, dt: Duration);
    
    /// Render dirty surfaces
    pub fn render_dirty_surfaces(&mut self);
    
    /// Focus a surface for keyboard input
    pub fn focus_surface(&mut self, id: SurfaceId);
    
    /// Get the currently focused surface
    pub fn focused_surface(&self) -> Option<SurfaceId>;
    
    /// Dispatch a keyboard event to the focused surface
    pub fn dispatch_keyboard(&mut self, event: KeyEvent) -> bool;
}
```

---

## Texture Sharing Strategies

The 3D engine and GPUI both need GPU access. Several strategies exist for sharing the rendered texture:

### Strategy 1: CPU Roundtrip (Simplest)

```
GPUI GPU ──render──► GPUI Texture ──readback──► CPU Memory ──upload──► 3D Engine Texture
```

**Pros:**
- Simple to implement
- No GPU API compatibility concerns
- Works across different GPU backends

**Cons:**
- Slow (GPU → CPU → GPU copy)
- High latency
- Uses CPU memory bandwidth

```rust
impl EmbeddedGpuiSurface {
    /// Update the 3D engine's texture via CPU roundtrip
    pub fn update_texture_cpu_roundtrip(&mut self, engine_texture: &mut EngineTexture) {
        if !self.dirty {
            return;
        }
        
        // Render GPUI to internal texture
        self.render_internal();
        
        // Read pixels back to CPU
        let pixels = self.renderer.read_pixels(&self.texture);
        
        // Upload to engine's texture
        engine_texture.upload_rgba(&pixels, self.size);
        
        self.dirty = false;
    }
}
```

### Strategy 2: Shared GPU Context (Optimal)

```
Shared GPU Device
       │
       ├───► GPUI Renderer ───► Shared Texture ◄─── 3D Engine Renderer
       │                              │
       └──────────────────────────────┘
```

**Pros:**
- Zero-copy, maximum performance
- No latency from data transfer
- Minimal memory usage

**Cons:**
- Requires both systems to use same GPU API
- Deep integration needed
- May require architectural changes

```rust
/// Shared GPU resources between GPUI and 3D engine
pub struct SharedGpuResources {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl EmbeddedGpuiSurface {
    /// Get the texture view for direct sampling by 3D engine
    pub fn texture_view(&self) -> &wgpu::TextureView {
        &self.texture.view
    }
    
    /// Get bind group for shader sampling
    pub fn bind_group(&self, layout: &wgpu::BindGroupLayout) -> wgpu::BindGroup {
        self.gpu_resources.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui_surface_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(self.texture_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}
```

### Strategy 3: Platform Interop APIs (Advanced)

Use platform-specific APIs to share textures between different GPU contexts:

| Platform | API |
|----------|-----|
| Vulkan | External Memory (VK_KHR_external_memory) |
| DirectX 12 | Shared Handles |
| Metal | MTLSharedEvent, IOSurface |
| OpenGL | EGL/GLX external objects |

**Pros:**
- Efficient cross-context sharing
- Works with different GPU backends

**Cons:**
- Platform-specific code
- Complex implementation
- Not all combinations supported

---

## Input Handling

### 3D to 2D Input Transformation

Mouse/pointer events in 3D space must be transformed to 2D GPUI coordinates:

```rust
/// Represents a GPUI surface positioned in 3D space
pub struct GpuiSurface3D {
    /// The embedded GPUI surface
    surface: EmbeddedGpuiSurface,
    
    /// World transform of the surface
    transform: Matrix4<f32>,
    
    /// Size in world units
    world_size: Vector2<f32>,
    
    /// Surface geometry (for raycasting)
    geometry: SurfaceGeometry,
}

/// Different surface geometries
pub enum SurfaceGeometry {
    /// Flat rectangular quad
    Quad { normal: Vector3<f32> },
    
    /// Cylindrical curve (like curved monitor)
    Cylinder { radius: f32, arc_angle: f32 },
    
    /// Spherical section
    Sphere { radius: f32 },
    
    /// Custom mesh with UV mapping
    Mesh { mesh: Arc<Mesh>, uv_channel: u32 },
}

impl GpuiSurface3D {
    /// Convert a 3D ray to GPUI surface coordinates
    pub fn ray_to_surface_coords(&self, ray: Ray3D) -> Option<Point<Pixels>> {
        // 1. Find intersection with surface geometry
        let hit = self.geometry.intersect(&ray, &self.transform)?;
        
        // 2. Get UV coordinates at intersection point
        let uv = self.geometry.point_to_uv(&hit, &self.transform)?;
        
        // 3. Convert UV to pixel coordinates
        let pixel_x = uv.x * self.surface.size.width.0 as f32;
        let pixel_y = uv.y * self.surface.size.height.0 as f32;
        
        Some(Point::new(px(pixel_x), px(pixel_y)))
    }
    
    /// Handle 3D pointer input
    pub fn handle_pointer_input(&mut self, event: Pointer3DEvent) -> bool {
        match event {
            Pointer3DEvent::Move { ray } => {
                if let Some(pos) = self.ray_to_surface_coords(ray) {
                    self.surface.inject_mouse_move(pos);
                    true
                } else {
                    // Ray doesn't hit surface, trigger mouse leave
                    self.surface.inject_mouse_leave();
                    false
                }
            }
            
            Pointer3DEvent::Press { ray, button } => {
                if let Some(pos) = self.ray_to_surface_coords(ray) {
                    self.surface.inject_mouse_down(pos, button);
                    true
                } else {
                    false
                }
            }
            
            Pointer3DEvent::Release { ray, button } => {
                if let Some(pos) = self.ray_to_surface_coords(ray) {
                    self.surface.inject_mouse_up(pos, button);
                    true
                } else {
                    false
                }
            }
            
            Pointer3DEvent::Scroll { ray, delta } => {
                if let Some(pos) = self.ray_to_surface_coords(ray) {
                    self.surface.inject_scroll(pos, delta);
                    true
                } else {
                    false
                }
            }
        }
    }
}
```

### Input State Management

```rust
/// Input state for an embedded surface
pub struct SurfaceInputState {
    /// Current mouse position (if hovering)
    mouse_position: Option<Point<Pixels>>,
    
    /// Currently pressed mouse buttons
    pressed_buttons: HashSet<MouseButton>,
    
    /// Currently pressed modifier keys
    modifiers: Modifiers,
    
    /// Whether this surface has pointer focus
    pointer_focus: bool,
    
    /// Whether this surface has keyboard focus
    keyboard_focus: bool,
}

impl EmbeddedGpuiSurface {
    /// Inject a mouse move event
    pub fn inject_mouse_move(&mut self, position: Point<Pixels>) {
        let previous = self.input_state.mouse_position;
        self.input_state.mouse_position = Some(position);
        
        // Generate GPUI mouse move event
        let event = MouseMoveEvent {
            position,
            pressed_button: self.input_state.pressed_buttons.iter().next().copied(),
            modifiers: self.input_state.modifiers,
        };
        
        self.dispatch_mouse_event(MouseEvent::Move(event));
    }
    
    /// Inject a mouse button press
    pub fn inject_mouse_down(&mut self, position: Point<Pixels>, button: MouseButton) {
        self.input_state.mouse_position = Some(position);
        self.input_state.pressed_buttons.insert(button);
        
        let event = MouseDownEvent {
            button,
            position,
            modifiers: self.input_state.modifiers,
            click_count: 1, // Would need tracking for double-click
        };
        
        self.dispatch_mouse_event(MouseEvent::Down(event));
    }
    
    /// Inject a mouse button release
    pub fn inject_mouse_up(&mut self, position: Point<Pixels>, button: MouseButton) {
        self.input_state.mouse_position = Some(position);
        self.input_state.pressed_buttons.remove(&button);
        
        let event = MouseUpEvent {
            button,
            position,
            modifiers: self.input_state.modifiers,
            click_count: 1,
        };
        
        self.dispatch_mouse_event(MouseEvent::Up(event));
    }
    
    /// Inject mouse leave (pointer left surface)
    pub fn inject_mouse_leave(&mut self) {
        self.input_state.mouse_position = None;
        self.input_state.pointer_focus = false;
        
        // Trigger hover state updates
        self.invalidate();
    }
    
    /// Inject a keyboard event
    pub fn inject_key_event(&mut self, event: KeyEvent) -> bool {
        if !self.input_state.keyboard_focus {
            return false;
        }
        
        // Dispatch to focused element
        self.dispatch_key_event(event)
    }
    
    fn dispatch_mouse_event(&mut self, event: MouseEvent) {
        // Hit test and dispatch to elements
        // Mark dirty if state changed
        self.invalidate();
    }
    
    fn dispatch_key_event(&mut self, event: KeyEvent) -> bool {
        // Dispatch to focused element
        // Return true if handled
        false
    }
}
```

---

## Render Scheduling

Different use cases require different render timing:

### Render Modes

```rust
/// How often a surface should re-render
pub enum RenderMode {
    /// Re-render every frame (for animations, real-time data)
    Continuous,
    
    /// Only re-render when content changed
    OnDemand,
    
    /// Re-render at most N times per second
    Throttled { max_fps: u32 },
    
    /// Manual control - only render when explicitly requested
    Manual,
}

impl GpuiSurfaceManager {
    /// Update and render surfaces based on their render mode
    pub fn update(&mut self, dt: Duration) {
        for surface in self.surfaces.values_mut() {
            surface.time_since_render += dt;
            
            let should_render = match surface.render_mode {
                RenderMode::Continuous => true,
                
                RenderMode::OnDemand => surface.dirty,
                
                RenderMode::Throttled { max_fps } => {
                    let min_interval = Duration::from_secs_f32(1.0 / max_fps as f32);
                    surface.dirty && surface.time_since_render >= min_interval
                }
                
                RenderMode::Manual => false, // Only via explicit render() call
            };
            
            if should_render {
                surface.render();
                surface.time_since_render = Duration::ZERO;
            }
        }
    }
}
```

### Dirty Tracking

```rust
impl EmbeddedGpuiSurface {
    /// Mark surface as needing re-render
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    
    /// Check if surface needs rendering
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    
    /// Force immediate render regardless of mode
    pub fn render(&mut self) {
        if self.dirty || matches!(self.render_mode, RenderMode::Continuous) {
            self.render_internal();
            self.dirty = false;
        }
    }
    
    /// Get texture, rendering if needed
    pub fn texture(&mut self) -> &GpuTexture {
        if self.dirty {
            self.render_internal();
            self.dirty = false;
        }
        &self.texture
    }
}
```

---

## Focus Management

With multiple GPUI surfaces, focus management becomes important:

```rust
impl GpuiSurfaceManager {
    /// Focus a surface for keyboard input
    pub fn focus_surface(&mut self, id: SurfaceId) {
        // Blur previous surface
        if let Some(prev_id) = self.focused_surface {
            if prev_id != id {
                if let Some(surface) = self.surfaces.get_mut(&prev_id) {
                    surface.on_blur();
                }
            }
        }
        
        // Focus new surface
        self.focused_surface = Some(id);
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.on_focus();
        }
    }
    
    /// Remove focus from all surfaces
    pub fn blur_all(&mut self) {
        if let Some(id) = self.focused_surface.take() {
            if let Some(surface) = self.surfaces.get_mut(&id) {
                surface.on_blur();
            }
        }
    }
    
    /// Dispatch keyboard event to focused surface
    pub fn dispatch_keyboard(&mut self, event: KeyEvent) -> bool {
        // First check for global shortcuts
        if self.handle_global_shortcut(&event) {
            return true;
        }
        
        // Then dispatch to focused surface
        if let Some(id) = self.focused_surface {
            if let Some(surface) = self.surfaces.get_mut(&id) {
                return surface.inject_key_event(event);
            }
        }
        
        false
    }
    
    fn handle_global_shortcut(&mut self, event: &KeyEvent) -> bool {
        // Handle shortcuts that work regardless of focused surface
        // e.g., Escape to close all surfaces
        false
    }
}

impl EmbeddedGpuiSurface {
    fn on_focus(&mut self) {
        self.input_state.keyboard_focus = true;
        self.invalidate();
    }
    
    fn on_blur(&mut self) {
        self.input_state.keyboard_focus = false;
        self.invalidate();
    }
}
```

---

## Integration Examples

### Bevy Integration

```rust
use bevy::prelude::*;
use gpui_embedded::{GpuiSurfaceManager, EmbeddedGpuiSurface, SurfaceId};

/// Bevy plugin for GPUI integration
pub struct GpuiPlugin;

impl Plugin for GpuiPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<GpuiManager>()
            .add_systems(Update, (
                update_gpui_surfaces,
                handle_gpui_input,
                sync_gpui_textures,
            ).chain());
    }
}

/// Resource holding the GPUI surface manager
#[derive(Resource)]
struct GpuiManager {
    manager: GpuiSurfaceManager,
}

/// Component for entities that display GPUI content
#[derive(Component)]
struct GpuiPanel {
    surface_id: SurfaceId,
    world_size: Vec2,
}

/// System: Update GPUI surfaces
fn update_gpui_surfaces(
    time: Res<Time>,
    mut gpui: ResMut<GpuiManager>,
) {
    gpui.manager.update(time.delta());
}

/// System: Handle input raycasting to GPUI panels
fn handle_gpui_input(
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    panels: Query<(&GpuiPanel, &GlobalTransform, &Handle<Mesh>)>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    mut gpui: ResMut<GpuiManager>,
    meshes: Res<Assets<Mesh>>,
) {
    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    
    // Get camera for raycasting
    let (camera, camera_transform) = cameras.single();
    let Some(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
        return;
    };
    
    // Check each panel for intersection
    for (panel, transform, mesh_handle) in panels.iter() {
        let Some(mesh) = meshes.get(mesh_handle) else { continue };
        
        // Raycast against panel
        if let Some((hit_point, uv)) = raycast_quad(&ray, transform, panel.world_size) {
            let surface = gpui.manager.get_surface_mut(panel.surface_id).unwrap();
            let size = surface.size();
            
            // Convert UV to pixel coordinates
            let pixel_pos = Point::new(
                px(uv.x * size.width.0 as f32),
                px(uv.y * size.height.0 as f32),
            );
            
            // Handle input
            if mouse_button.just_pressed(MouseButton::Left) {
                surface.inject_mouse_down(pixel_pos, gpui::MouseButton::Left);
                gpui.manager.focus_surface(panel.surface_id);
            } else if mouse_button.just_released(MouseButton::Left) {
                surface.inject_mouse_up(pixel_pos, gpui::MouseButton::Left);
            } else {
                surface.inject_mouse_move(pixel_pos);
            }
            
            return; // Only interact with frontmost panel
        }
    }
}

/// System: Sync GPUI textures to Bevy materials
fn sync_gpui_textures(
    mut gpui: ResMut<GpuiManager>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    panels: Query<(&GpuiPanel, &Handle<StandardMaterial>)>,
) {
    for (panel, material_handle) in panels.iter() {
        if let Some(surface) = gpui.manager.get_surface_mut(panel.surface_id) {
            if surface.is_dirty() {
                surface.render();
                
                // Update Bevy material with new texture
                // (Implementation depends on texture sharing strategy)
            }
        }
    }
}
```

### wgpu Direct Integration

```rust
use wgpu;
use gpui_embedded::{GpuiSurfaceManager, SharedGpuResources};

struct App {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    gpui_manager: GpuiSurfaceManager,
    panel_pipeline: wgpu::RenderPipeline,
    panel_bind_group_layout: wgpu::BindGroupLayout,
}

impl App {
    fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        // Create GPUI manager with shared GPU resources
        let gpui_manager = GpuiSurfaceManager::new(
            SharedGpuResources {
                device: Arc::clone(&device),
                queue: Arc::clone(&queue),
            }
        ).unwrap();
        
        // Create pipeline for rendering textured quads
        let panel_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpui_panel_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        
        // ... create pipeline ...
        
        Self {
            device,
            queue,
            gpui_manager,
            panel_pipeline,
            panel_bind_group_layout,
        }
    }
    
    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        // Update GPUI surfaces
        self.gpui_manager.update(self.dt);
        
        // Render each GPUI panel
        for surface in self.gpui_manager.surfaces() {
            // Get bind group for this surface's texture
            let bind_group = surface.bind_group(&self.panel_bind_group_layout);
            
            // Render the panel quad with GPUI texture
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpui_panel_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Don't clear, add to existing scene
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            
            render_pass.set_pipeline(&self.panel_pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1); // Draw quad
        }
    }
}
```

---

## Performance Considerations

### Texture Resolution

| Use Case | Recommended Resolution | Notes |
|----------|----------------------|-------|
| Distant UI | 256x256 - 512x512 | Lower res acceptable |
| Near UI | 1024x1024 - 2048x2048 | Sharp text needed |
| VR/AR | 1024x1024+ | High res for readability |
| Screen-space | Match window size | 1:1 pixel mapping |

### Render Frequency

| Content Type | Recommended Mode | FPS |
|--------------|------------------|-----|
| Static text/buttons | OnDemand | N/A |
| Animated UI | Throttled | 30-60 |
| Video/realtime data | Continuous | 60+ |
| Cursor/hover effects | Throttled | 60 |

### Memory Budget

```rust
/// Calculate memory usage for a surface
fn estimate_surface_memory(size: Size<DevicePixels>, format: TextureFormat) -> usize {
    let bytes_per_pixel = match format {
        TextureFormat::Rgba8 => 4,
        TextureFormat::Rgba16Float => 8,
        TextureFormat::Rgba32Float => 16,
        _ => 4,
    };
    
    let texture_memory = size.width.0 as usize 
        * size.height.0 as usize 
        * bytes_per_pixel;
    
    // Account for atlas, staging buffers, etc.
    texture_memory * 2
}
```

---

## Limitations

### Not Supported

1. **Platform Dialogs**: File pickers, alerts won't appear in 3D
2. **System Clipboard**: May not integrate with host platform
3. **IME Input**: Complex text input methods
4. **Accessibility**: Screen readers can't access 3D embedded UI
5. **Drag and Drop**: Cross-surface drag operations
6. **Context Menus**: May need custom 3D-aware implementation
7. **Window Management**: No minimize, maximize, resize handles

### Workarounds

| Limitation | Workaround |
|------------|------------|
| File dialogs | Use in-surface file browser component |
| Clipboard | Implement custom clipboard via app state |
| IME | Basic text input only, no CJK IME |
| Accessibility | Provide alternative 2D mode |

---

## Summary

Embedding GPUI in 3D environments requires:

1. **Texture-based rendering**: GPUI renders to texture, 3D engine samples it
2. **Input transformation**: 3D raycasts converted to 2D coordinates
3. **Focus management**: Track which surface receives keyboard input
4. **Render scheduling**: Balance update frequency with performance
5. **GPU resource sharing**: Choose appropriate texture sharing strategy

The architecture cleanly separates GPUI's 2D rendering from the 3D engine's scene management, allowing GPUI interfaces to be treated as textured surfaces in any 3D environment.

---

## Related Documents

- [Render-to-Texture Critique](./render-to-texture-critique.md) - Analysis of current implementation
- [One-Shot Rendering Architecture](./one-shot-rendering-architecture.md) - Foundation for texture rendering