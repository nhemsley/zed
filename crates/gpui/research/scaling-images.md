# Scaling Images: Preserving Syntax Highlighting Colors

> Strategies for downscaling rendered code/text while preserving syntax highlighting colors.

## The Problem

When naively downscaling textures containing syntax-highlighted code:
- Text becomes illegible mush
- Colors get averaged with background → washed out
- Syntax highlighting information is lost
- Small colored tokens disappear entirely

**Goal**: At low zoom levels, text should become **colored blocks/lines** that preserve
the syntax highlighting colors (like VS Code's minimap, Sublime Text's minimap).

```
Zoomed in (readable):
  fn main() {
      println!("Hello");
  }

Zoomed out (colored blocks):
  ██ ████() {
      ███████("█████");
  }

Colors preserved: keywords=purple, strings=green, functions=blue, etc.
```

## Why Standard Downscaling Fails

Standard bilinear/bicubic interpolation **averages** pixels in each block:

```
Block of pixels:        Average result:
┌─────────────────┐
│ white white wht │     
│ white BLUE  wht │  →  very light blue (almost white)
│ white white wht │     
└─────────────────┘

The single blue keyword pixel gets diluted by surrounding whitespace.
```

## Solution Approaches

### 1. Dominant Color Per Block (Recommended)

Instead of averaging, pick the **most saturated** color in each block:

```rust
fn downscale_preserve_colors(
    pixels: &[u8],  // RGBA
    src_w: u32,
    src_h: u32, 
    scale: u32,
) -> Vec<u8> {
    let dst_w = src_w / scale;
    let dst_h = src_h / scale;
    let mut result = vec![0u8; (dst_w * dst_h * 4) as usize];
    
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let block = get_block(pixels, dx * scale, dy * scale, scale, src_w);
            
            // Pick most saturated color, not average
            let color = most_saturated_color(&block);
            
            set_pixel(&mut result, dx, dy, dst_w, color);
        }
    }
    result
}

fn most_saturated_color(block: &[[u8; 4]]) -> [u8; 4] {
    block
        .iter()
        .max_by_key(|[r, g, b, _a]| {
            // Saturation = how "colorful" vs gray
            let max = (*r).max(*g).max(*b);
            let min = (*r).min(*g).min(*b);
            (max - min) as u32  // Higher = more saturated
        })
        .copied()
        .unwrap_or([0, 0, 0, 255])
}
```

**Why it works**: Syntax highlighting colors (blue, green, purple, orange) are more
saturated than white/gray background. They "win" the competition.

### 2. Max Saturation Pooling with Luminance Weighting

Improved version that also considers how "different from background" a color is:

```rust
fn dominant_color_weighted(block: &[[u8; 4]], bg_color: [u8; 4]) -> [u8; 4] {
    block
        .iter()
        .max_by_key(|pixel| {
            let saturation = color_saturation(pixel);
            let bg_distance = color_distance(pixel, &bg_color);
            
            // Prefer saturated colors that are far from background
            saturation * 2 + bg_distance
        })
        .copied()
        .unwrap_or(bg_color)
}

fn color_distance(a: &[u8; 4], b: &[u8; 4]) -> u32 {
    let dr = (a[0] as i32 - b[0] as i32).abs() as u32;
    let dg = (a[1] as i32 - b[1] as i32).abs() as u32;
    let db = (a[2] as i32 - b[2] as i32).abs() as u32;
    dr + dg + db
}

fn color_saturation(pixel: &[u8; 4]) -> u32 {
    let max = pixel[0].max(pixel[1]).max(pixel[2]);
    let min = pixel[0].min(pixel[1]).min(pixel[2]);
    (max - min) as u32
}
```

### 3. Two-Pass: Background Separation

Explicitly separate foreground (text) from background:

```rust
fn downscale_with_bg_separation(
    pixels: &[u8],
    src_w: u32,
    src_h: u32,
    scale: u32,
    bg_color: [u8; 4],
    bg_threshold: u32,  // How close to bg to be considered bg
) -> Vec<u8> {
    let dst_w = src_w / scale;
    let dst_h = src_h / scale;
    let mut result = vec![0u8; (dst_w * dst_h * 4) as usize];
    
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let block = get_block(pixels, dx * scale, dy * scale, scale, src_w);
            
            // Separate foreground and background pixels
            let (fg_pixels, bg_pixels): (Vec<_>, Vec<_>) = block
                .iter()
                .partition(|p| color_distance(p, &bg_color) > bg_threshold);
            
            let color = if !fg_pixels.is_empty() {
                // Use dominant foreground color
                most_saturated_color(&fg_pixels)
            } else {
                // All background
                bg_color
            };
            
            set_pixel(&mut result, dx, dy, dst_w, color);
        }
    }
    result
}
```

### 4. Morphological Approach (Min/Max Pooling)

For dark text on light background, use **MIN pooling** (darkest pixel wins):

```rust
fn min_pool_downscale(pixels: &[u8], src_w: u32, src_h: u32, scale: u32) -> Vec<u8> {
    // For each block, take the darkest pixel
    // Dark text strokes will "win" over light background
    
    for_each_block(|block| {
        block
            .iter()
            .min_by_key(|[r, g, b, _]| (*r as u32 + *g as u32 + *b as u32))
            .copied()
    })
}

fn max_pool_downscale(pixels: &[u8], ...) -> Vec<u8> {
    // For light text on dark background, take brightest pixel
    for_each_block(|block| {
        block
            .iter()
            .max_by_key(|[r, g, b, _]| (*r as u32 + *g as u32 + *b as u32))
            .copied()
    })
}
```

### 5. LOD: Re-render as Colored Blocks

Don't downscale the texture at all - render **different content** at low zoom:

```rust
fn render_code_line(tokens: &[Token], zoom: f32) -> impl IntoElement {
    if zoom > 0.3 {
        // Normal text rendering
        div().children(tokens.iter().map(|t| {
            span().text_color(t.color).child(&t.text)
        }))
    } else {
        // Colored blocks (no text)
        div().flex().gap_px().children(tokens.iter().map(|t| {
            div()
                .w(px(t.text.len() as f32 * 1.5))  // Width proportional to token
                .h(px(2.0))
                .bg(t.color)
        }))
    }
}
```

This is what VS Code's minimap actually does - it's not downscaled text,
it's re-rendered as tiny colored rectangles.

## Integration with TexturedView

### Option A: Post-Process in Render Thread

```rust
pub enum DownscaleMode {
    /// Standard bilinear interpolation
    Linear,
    
    /// Preserve most saturated colors (syntax highlighting)
    PreserveSaturated,
    
    /// Preserve colors far from background
    PreserveForeground { bg_color: [u8; 4] },
    
    /// Nearest neighbor (blocky)
    Nearest,
}

impl TexturedView {
    /// Set downscale mode for when texture is displayed smaller than rendered
    pub fn downscale_mode(mut self, mode: DownscaleMode) -> Self {
        self.downscale_mode = mode;
        self
    }
}
```

Apply in the background render thread before sending pixels:

```rust
// In render thread, when zoom < threshold:
let processed_pixels = match downscale_mode {
    DownscaleMode::PreserveSaturated => {
        saturation_preserving_downscale(&pixels, width, height, scale_factor)
    }
    DownscaleMode::PreserveForeground { bg_color } => {
        foreground_preserving_downscale(&pixels, width, height, scale_factor, bg_color)
    }
    _ => pixels,
};

sender.send(RenderedFrame { pixels: processed_pixels, .. });
```

### Option B: GPU Shader

For real-time zoom, could implement as a custom shader:

```glsl
// Fragment shader for saturation-preserving downscale
uniform sampler2D texture;
uniform vec2 texel_size;
uniform float block_size;

void main() {
    vec4 most_saturated = vec4(0.0);
    float max_saturation = 0.0;
    
    for (int y = 0; y < block_size; y++) {
        for (int x = 0; x < block_size; x++) {
            vec2 offset = vec2(x, y) * texel_size;
            vec4 color = texture2D(texture, gl_TexCoord[0].xy + offset);
            
            float sat = max(color.r, max(color.g, color.b)) 
                      - min(color.r, min(color.g, color.b));
            
            if (sat > max_saturation) {
                max_saturation = sat;
                most_saturated = color;
            }
        }
    }
    
    gl_FragColor = most_saturated;
}
```

### Option C: Multi-Resolution Textures (Mipmaps)

Pre-compute multiple resolutions with color-preserving downscale:

```rust
struct TexturedViewWithLOD {
    /// Full resolution texture
    full_res: Arc<RenderImage>,
    /// Pre-computed downscaled versions with color preservation
    lod_levels: Vec<(f32, Arc<RenderImage>)>,  // (min_zoom, texture)
}

impl TexturedViewWithLOD {
    fn texture_for_zoom(&self, zoom: f32) -> &Arc<RenderImage> {
        self.lod_levels
            .iter()
            .find(|(min_zoom, _)| zoom >= *min_zoom)
            .map(|(_, tex)| tex)
            .unwrap_or(&self.full_res)
    }
}
```

## Recommendation

For **infinite canvas with syntax-highlighted code**:

1. **Primary**: Use **saturation-preserving downscale** (approach #1 or #2)
   - Simple to implement
   - Works for any syntax theme
   - No need to know token boundaries

2. **Enhancement**: Add **background separation** (approach #3) if you know the bg color
   - Even better color preservation
   - Can be derived from theme

3. **Future**: Consider **LOD re-rendering** (approach #5) for very small zoom
   - Best quality at extreme zoom-out
   - Requires access to token data, not just pixels

## Performance Considerations

| Approach | CPU Cost | Memory | Quality |
|----------|----------|--------|---------|
| Linear (standard) | Low | Low | Poor for text |
| Saturation pooling | Medium | Low | Good |
| Background separation | Medium | Low | Better |
| Pre-computed LOD | Low (runtime) | High | Best |
| GPU shader | Very Low | Low | Good |

For TexturedView with many canvas items, **pre-computed LOD** or **GPU shader**
is recommended to avoid per-frame CPU work.

## Related Files

- `gpui/research/textured_view_design.md` - TexturedView architecture
- `gpui/research/infinite_canvas_textured_api.md` - Canvas API design
- `gpui/examples/multi_app_textured.rs` - Working texture streaming example