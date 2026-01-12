//! Example: TexturedView Stress Test
//!
//! This example aggressively stress tests TexturedView to expose potential
//! race conditions, memory issues, or segfaults in the rendering pipeline.
//!
//! **WARNING**: Each TexturedView spawns a background thread with its own
//! `Application::textured()` instance and GPU context. Creating too many
//! views too quickly can exhaust system resources (GPU contexts, threads,
//! file descriptors).
//!
//! Stress scenarios:
//! 1. Multiple simultaneous streaming views at high FPS
//! 2. Rapid creation and destruction of views
//! 3. Varying frame rates across views
//! 4. High memory pressure with many views
//!
//! Run with: cargo run -p gpui --example textured_view_stress_test
//!
//! For aggressive auto-stress (debugging segfaults):
//!   STRESS_AUTO=1 cargo run -p gpui --example textured_view_stress_test --release
//!
//! Watch for:
//! - Segfaults (unsafe code issues)
//! - Memory leaks (watch process memory over time)
//! - Rendering glitches
//! - Thread panics
//! - GPU context exhaustion

use gpui::{
    App, Application, Bounds, Context, ElementId, Entity, ParentElement, Render, Styled,
    TexturedView, Timer, WeakEntity, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb,
    size,
};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

// Global counters for tracking
static TOTAL_FRAMES_RENDERED: AtomicU64 = AtomicU64::new(0);
static TOTAL_VIEWS_CREATED: AtomicU32 = AtomicU32::new(0);
static TOTAL_VIEWS_DESTROYED: AtomicU32 = AtomicU32::new(0);

/// Render function for streaming views
fn render_stress_content() -> gpui::Div {
    // Use a simple incrementing counter based on total frames
    let frame = TOTAL_FRAMES_RENDERED.fetch_add(1, Ordering::Relaxed);

    // Chaotic animation to stress the renderer
    let t = frame as f32 / 60.0;

    // Multiple moving elements
    let x1 = ((t * 3.7).sin() * 0.5 + 0.5) * 180.0;
    let y1 = ((t * 2.3).cos() * 0.5 + 0.5) * 80.0;
    let x2 = ((t * 5.1).cos() * 0.5 + 0.5) * 180.0;
    let y2 = ((t * 4.7).sin() * 0.5 + 0.5) * 80.0;
    let x3 = ((t * 2.9).sin() * 0.5 + 0.5) * 180.0;
    let y3 = ((t * 3.3).cos() * 0.5 + 0.5) * 80.0;

    // Rapidly changing colors
    let hue1 = ((frame * 7) % 360) as f32 / 360.0;
    let hue2 = ((frame * 11) % 360) as f32 / 360.0;
    let hue3 = ((frame * 13) % 360) as f32 / 360.0;

    let color1 = hsl_to_rgb_packed(hue1, 0.8, 0.5);
    let color2 = hsl_to_rgb_packed(hue2, 0.8, 0.5);
    let color3 = hsl_to_rgb_packed(hue3, 0.8, 0.5);

    div()
        .size_full()
        .bg(rgb(0x0a0a15))
        .relative()
        .overflow_hidden()
        // Element 1
        .child(
            div()
                .absolute()
                .left(px(x1))
                .top(px(y1))
                .w(px(30.))
                .h(px(30.))
                .bg(color1)
                .rounded_full(),
        )
        // Element 2
        .child(
            div()
                .absolute()
                .left(px(x2))
                .top(px(y2))
                .w(px(25.))
                .h(px(25.))
                .bg(color2)
                .rounded_md(),
        )
        // Element 3
        .child(
            div()
                .absolute()
                .left(px(x3))
                .top(px(y3))
                .w(px(20.))
                .h(px(20.))
                .bg(color3),
        )
        // Many small particles for extra stress
        .children((0..10).map(|i| {
            let offset = i as f32 * 0.3;
            let px_val = ((t + offset) * (2.0 + i as f32 * 0.5)).sin() * 90.0 + 100.0;
            let py_val = ((t + offset) * (1.5 + i as f32 * 0.3)).cos() * 40.0 + 50.0;
            let particle_hue = ((frame as usize + i * 17) % 360) as f32 / 360.0;

            div()
                .absolute()
                .left(px(px_val))
                .top(px(py_val))
                .w(px(8.))
                .h(px(8.))
                .bg(hsl_to_rgb_packed(particle_hue, 0.9, 0.6))
                .rounded_full()
        }))
        // Frame counter overlay
        .child(
            div()
                .absolute()
                .right(px(4.))
                .top(px(4.))
                .text_color(rgb(0x666666))
                .text_xs()
                .child(format!("{}", frame % 10000)),
        )
}

fn hsl_to_rgb_packed(h: f32, s: f32, l: f32) -> gpui::Hsla {
    gpui::hsla(h, s, l, 1.0)
}

type StreamingRenderFn = fn() -> gpui::Div;

struct StressView {
    id: u32,
    fps: u32,
    view: Entity<TexturedView<StreamingRenderFn>>,
    created_at: std::time::Instant,
}

struct StressTestApp {
    views: Vec<StressView>,
    next_id: u32,
    stress_mode: StressMode,
    auto_stress: bool,
    stress_interval_ms: u64,
    max_views: usize,
    started_at: std::time::Instant,
}

#[derive(Clone, Copy, PartialEq)]
enum StressMode {
    /// Add views steadily until max, then hold
    Accumulate,
    /// Rapidly create and destroy views
    Churn,
    /// Create many views at once, destroy all, repeat
    Burst,
    /// Random FPS changes on existing views
    FpsFlux,
}

impl StressTestApp {
    fn new() -> Self {
        // Check for STRESS_AUTO env var for aggressive auto-start mode
        let aggressive = std::env::var("STRESS_AUTO").is_ok();

        Self {
            views: Vec::new(),
            next_id: 0,
            stress_mode: if aggressive {
                StressMode::Churn
            } else {
                StressMode::Accumulate
            },
            auto_stress: aggressive, // Auto-start if STRESS_AUTO is set
            stress_interval_ms: if aggressive { 200 } else { 1000 }, // 200ms if aggressive
            max_views: if aggressive { 6 } else { 6 }, // Keep views limited to avoid FD exhaustion
            started_at: std::time::Instant::now(),
        }
    }

    fn add_view(&mut self, fps: u32, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;

        TOTAL_VIEWS_CREATED.fetch_add(1, Ordering::Relaxed);

        let render_fn: StreamingRenderFn = render_stress_content;
        let view = cx.new(|cx| {
            TexturedView::streaming(size(px(220.), px(120.)), fps, window, cx, render_fn)
        });

        self.views.push(StressView {
            id,
            fps,
            view,
            created_at: std::time::Instant::now(),
        });

        cx.notify();
    }

    fn remove_view(&mut self, index: usize, _cx: &mut Context<Self>) {
        if index < self.views.len() {
            self.views.remove(index);
            TOTAL_VIEWS_DESTROYED.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn remove_oldest(&mut self, cx: &mut Context<Self>) {
        if !self.views.is_empty() {
            self.remove_view(0, cx);
            cx.notify();
        }
    }

    fn remove_all(&mut self, cx: &mut Context<Self>) {
        let count = self.views.len();
        self.views.clear();
        TOTAL_VIEWS_DESTROYED.fetch_add(count as u32, Ordering::Relaxed);
        cx.notify();
    }

    fn random_fps() -> u32 {
        // Random-ish FPS between 15 and 120
        let seed = TOTAL_FRAMES_RENDERED.load(Ordering::Relaxed);
        let fps_options = [15, 24, 30, 45, 60, 90, 120];
        fps_options[(seed as usize) % fps_options.len()]
    }

    fn do_stress_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.stress_mode {
            StressMode::Accumulate => {
                if self.views.len() < self.max_views {
                    self.add_view(Self::random_fps(), window, cx);
                }
            }
            StressMode::Churn => {
                // Add one, remove one (if we have any)
                if self.views.len() > 0 && TOTAL_VIEWS_CREATED.load(Ordering::Relaxed) % 2 == 0 {
                    self.remove_oldest(cx);
                }
                if self.views.len() < self.max_views {
                    self.add_view(Self::random_fps(), window, cx);
                }
            }
            StressMode::Burst => {
                if self.views.is_empty() {
                    // Add many at once
                    for _ in 0..self.max_views.min(8) {
                        self.add_view(Self::random_fps(), window, cx);
                    }
                } else if self.views.len() >= self.max_views {
                    // Remove all
                    self.remove_all(cx);
                }
            }
            StressMode::FpsFlux => {
                // Keep views but recreate them with different FPS
                if self.views.len() < 4 {
                    self.add_view(Self::random_fps(), window, cx);
                } else {
                    // Remove and re-add oldest with new FPS
                    self.remove_oldest(cx);
                    self.add_view(Self::random_fps(), window, cx);
                }
            }
        }
    }

    fn schedule_stress(
        entity: WeakEntity<Self>,
        interval_ms: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let interval = Duration::from_millis(interval_ms);

        window
            .spawn(cx, async move |cx| {
                Timer::after(interval).await;
                cx.update(|window, cx| {
                    if let Some(entity) = entity.upgrade() {
                        entity.update(cx, |this, cx| {
                            if this.auto_stress {
                                this.do_stress_action(window, cx);
                                Self::schedule_stress(
                                    cx.weak_entity(),
                                    this.stress_interval_ms,
                                    window,
                                    cx,
                                );
                            }
                        });
                    }
                    window.refresh();
                })
                .ok();
            })
            .detach();
    }
}

impl Render for StressTestApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Start stress loop on first render (if auto_stress is enabled)
        if self.views.is_empty() && self.auto_stress {
            // Small delay before starting to let the main window stabilize
            Self::schedule_stress(cx.weak_entity(), 500, window, cx);
        }

        let elapsed = self.started_at.elapsed().as_secs();
        let total_created = TOTAL_VIEWS_CREATED.load(Ordering::Relaxed);
        let total_destroyed = TOTAL_VIEWS_DESTROYED.load(Ordering::Relaxed);
        let total_frames = TOTAL_FRAMES_RENDERED.load(Ordering::Relaxed);
        let view_count = self.views.len();
        let auto_stress = self.auto_stress;
        let stress_mode = self.stress_mode;
        let interval = self.stress_interval_ms;

        div()
            .size_full()
            .bg(rgb(0x0f0f1a))
            .flex()
            .flex_col()
            .overflow_hidden()
            // Header with stats
            .child(
                div()
                    .p(px(16.))
                    .bg(rgb(0x1a1a2e))
                    .border_b_1()
                    .border_color(rgb(0x2a2a4a))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(0xff6b6b))
                                    .child("⚠ TexturedView Stress Test (1 view = 1 GPU context)"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x888888))
                                    .child(format!("Uptime: {}s", elapsed)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_6()
                            .text_sm()
                            .child(
                                div()
                                    .text_color(rgb(0x4ecdc4))
                                    .child(format!("Views: {}", view_count)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0xffe66d))
                                    .child(format!("Created: {}", total_created)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0xff6b6b))
                                    .child(format!("Destroyed: {}", total_destroyed)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x95e1d3))
                                    .child(format!("Frames: {}", total_frames)),
                            ),
                    ),
            )
            // Controls
            .child(
                div()
                    .p(px(12.))
                    .bg(rgb(0x151525))
                    .border_b_1()
                    .border_color(rgb(0x2a2a4a))
                    .flex()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    // Auto stress toggle
                    .child(
                        div()
                            .id("toggle-auto")
                            .px(px(12.))
                            .py(px(6.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(if auto_stress {
                                rgb(0x4ecdc4)
                            } else {
                                rgb(0x3a3a5a)
                            })
                            .text_color(if auto_stress {
                                rgb(0x0f0f1a)
                            } else {
                                rgb(0x888888)
                            })
                            .hover(|s| s.opacity(0.8))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.auto_stress = !this.auto_stress;
                                if this.auto_stress {
                                    Self::schedule_stress(
                                        cx.weak_entity(),
                                        this.stress_interval_ms,
                                        window,
                                        cx,
                                    );
                                }
                                cx.notify();
                            }))
                            .child(if auto_stress {
                                "⏸ Pause"
                            } else {
                                "▶ Start"
                            }),
                    )
                    // Mode buttons
                    .child(div().text_color(rgb(0x666666)).child("|"))
                    .child(mode_button(
                        "Accumulate",
                        StressMode::Accumulate,
                        stress_mode,
                        cx,
                    ))
                    .child(mode_button("Churn", StressMode::Churn, stress_mode, cx))
                    .child(mode_button("Burst", StressMode::Burst, stress_mode, cx))
                    .child(mode_button(
                        "FPS Flux",
                        StressMode::FpsFlux,
                        stress_mode,
                        cx,
                    ))
                    // Interval controls
                    .child(div().text_color(rgb(0x666666)).child("|"))
                    .child(
                        div()
                            .text_color(rgb(0x888888))
                            .text_sm()
                            .child(format!("{}ms", interval)),
                    )
                    .child(interval_button("50", 50, interval, cx))
                    .child(interval_button("100", 100, interval, cx))
                    .child(interval_button("250", 250, interval, cx))
                    .child(interval_button("500", 500, interval, cx))
                    .child(interval_button("1000", 1000, interval, cx))
                    // Manual controls
                    .child(div().text_color(rgb(0x666666)).child("|"))
                    .child(
                        div()
                            .id("add-view")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(rgb(0x4a9c6d))
                            .text_color(rgb(0xffffff))
                            .hover(|s| s.opacity(0.8))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_view(Self::random_fps(), window, cx);
                            }))
                            .child("+Add"),
                    )
                    .child(
                        div()
                            .id("remove-view")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(rgb(0x9c4a4a))
                            .text_color(rgb(0xffffff))
                            .hover(|s| s.opacity(0.8))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remove_oldest(cx);
                            }))
                            .child("-Remove"),
                    )
                    .child(
                        div()
                            .id("clear-all")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(rgb(0x9c4a7a))
                            .text_color(rgb(0xffffff))
                            .hover(|s| s.opacity(0.8))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remove_all(cx);
                            }))
                            .child("Clear All"),
                    ),
            )
            // Views grid (scrollable)
            .child(
                div().flex_1().overflow_hidden().child(
                    div()
                        .id("views-scroll")
                        .size_full()
                        .overflow_scroll()
                        .p(px(16.))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_3()
                                .children(self.views.iter().map(|sv| {
                                    let age_ms = sv.created_at.elapsed().as_millis() as u64;

                                    div()
                                        .id(ElementId::Name(format!("sv-{}", sv.id).into()))
                                        .flex()
                                        .flex_col()
                                        .bg(rgb(0x1a1a2e))
                                        .rounded_lg()
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(rgb(0x2a2a4a))
                                        // Header
                                        .child(
                                            div()
                                                .px(px(8.))
                                                .py(px(4.))
                                                .bg(rgb(0x252540))
                                                .flex()
                                                .justify_between()
                                                .text_xs()
                                                .child(
                                                    div()
                                                        .text_color(rgb(0x4ecdc4))
                                                        .child(format!("#{}", sv.id)),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(rgb(0xffe66d))
                                                        .child(format!("{}fps", sv.fps)),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(rgb(0x666666))
                                                        .child(format!("{}ms", age_ms)),
                                                ),
                                        )
                                        // The actual textured view
                                        .child(sv.view.clone())
                                })),
                        ),
                ),
            )
            // Footer
            .child(
                div()
                    .p(px(12.))
                    .bg(rgb(0x1a1a2e))
                    .border_t_1()
                    .border_color(rgb(0x2a2a4a))
                    .text_xs()
                    .text_color(rgb(0xff6b6b))
                    .child(
                        "⚠ Each view spawns a thread + GPU context. Too many views or too fast creation can exhaust resources. \
                         Start slow (1000ms) and work up.",
                    ),
            )
    }
}

fn mode_button(
    label: &str,
    mode: StressMode,
    current: StressMode,
    cx: &mut Context<StressTestApp>,
) -> impl IntoElement {
    let is_active = mode == current;

    div()
        .id(ElementId::Name(format!("mode-{}", label).into()))
        .px(px(10.))
        .py(px(5.))
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .bg(if is_active {
            rgb(0x6c5ce7)
        } else {
            rgb(0x2a2a4a)
        })
        .text_color(if is_active {
            rgb(0xffffff)
        } else {
            rgb(0x888888)
        })
        .hover(|s| if is_active { s } else { s.bg(rgb(0x3a3a5a)) })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.stress_mode = mode;
            cx.notify();
        }))
        .child(label.to_string())
}

fn interval_button(
    label: &str,
    ms: u64,
    current: u64,
    cx: &mut Context<StressTestApp>,
) -> impl IntoElement {
    let is_active = ms == current;

    div()
        .id(ElementId::Name(format!("int-{}", ms).into()))
        .px(px(6.))
        .py(px(4.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_xs()
        .bg(if is_active {
            rgb(0x45b7d1)
        } else {
            rgb(0x2a2a4a)
        })
        .text_color(if is_active {
            rgb(0x0f0f1a)
        } else {
            rgb(0x666666)
        })
        .hover(|s| if is_active { s } else { s.bg(rgb(0x3a3a5a)) })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.stress_interval_ms = ms;
            cx.notify();
        }))
        .child(label.to_string())
}

fn main() {
    // Set up panic hook to catch panics in threads
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("!!! PANIC DETECTED !!!");
        eprintln!("{}", panic_info);
        eprintln!("This may indicate a race condition or memory corruption.");
    }));

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| StressTestApp::new()),
        )
        .unwrap();

        cx.activate(true);
    });
}
