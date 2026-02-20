use acp_thread::{AcpThread, AgentThreadEntry, AssistantMessageChunk};
use gpui::{
    div, px, App, Context, CursorStyle, DragMoveEvent, Entity, EventEmitter, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use language_model::Role;
use ui::{
    prelude::*, Color, Divider, DividerColor, IconButton, IconName, IconSize, Label, LabelSize,
    Tooltip,
};

struct SliderDrag;

pub enum FocusedContextPanelEvent {
    ScrollToEntry { entry_index: usize },
}

pub struct FocusedContextPanel {
    thread: Entity<AcpThread>,
}

impl EventEmitter<FocusedContextPanelEvent> for FocusedContextPanel {}

impl FocusedContextPanel {
    pub fn new(thread: Entity<AcpThread>, _cx: &mut Context<Self>) -> Self {
        Self { thread }
    }

    fn effective_num_messages(&self, cx: &App) -> usize {
        let thread = self.thread.read(cx);
        let total = thread.message_count();
        thread.num_messages().unwrap_or(total).min(total)
    }

    fn set_num_messages_from_fraction(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let total = self.thread.read(cx).message_count();
        if total == 0 {
            return;
        }
        let fraction = fraction.clamp(0.0, 1.0);
        let value = ((fraction * total as f32).round() as usize)
            .max(1)
            .min(total);
        let num_messages = if value >= total { None } else { Some(value) };
        self.thread.update(cx, |thread, cx| {
            thread.set_num_messages(num_messages, cx);
        });
    }

    fn entry_char_count(entry: &AgentThreadEntry, cx: &App) -> u32 {
        match entry {
            AgentThreadEntry::UserMessage(msg) => msg.content.to_markdown(cx).len() as u32,
            AgentThreadEntry::AssistantMessage(msg) => msg
                .chunks
                .iter()
                .map(|chunk| match chunk {
                    AssistantMessageChunk::Message { block }
                    | AssistantMessageChunk::Thought { block } => {
                        block.to_markdown(cx).len() as u32
                    }
                })
                .sum(),
            AgentThreadEntry::ToolCall(_) => 0,
        }
    }

    fn estimate_entry_tokens(entry: &AgentThreadEntry, cx: &App) -> usize {
        Self::entry_char_count(entry, cx) as usize / 4
    }

    fn entry_preview(entry: &AgentThreadEntry, max_words: usize, cx: &App) -> String {
        let text = match entry {
            AgentThreadEntry::UserMessage(msg) => msg.content.to_markdown(cx).to_string(),
            AgentThreadEntry::AssistantMessage(msg) => msg
                .chunks
                .iter()
                .map(|chunk| match chunk {
                    AssistantMessageChunk::Message { block } => block.to_markdown(cx).to_string(),
                    AssistantMessageChunk::Thought { block } => block.to_markdown(cx).to_string(),
                })
                .collect::<Vec<_>>()
                .join(" "),
            AgentThreadEntry::ToolCall(_) => "[tool call]".to_string(),
        };
        let trimmed = text.trim().replace('\n', " ");
        if trimmed.is_empty() {
            return "[no preview]".to_string();
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        if words.len() > max_words {
            format!("{}…", words[..max_words].join(" "))
        } else {
            trimmed
        }
    }

    fn render_slider(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = self.thread.read(cx);
        let total = thread.message_count();
        let included = self.effective_num_messages(cx);
        let fraction = if total == 0 {
            1.0
        } else {
            included as f32 / total as f32
        };

        let entries = thread.entries();
        let message_entries: Vec<&AgentThreadEntry> = entries
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    AgentThreadEntry::UserMessage(_) | AgentThreadEntry::AssistantMessage(_)
                )
            })
            .collect();

        let total_tokens: usize = message_entries
            .iter()
            .map(|e| Self::estimate_entry_tokens(e, cx))
            .sum();
        let skip_count = total.saturating_sub(included);
        let included_tokens: usize = message_entries
            .iter()
            .skip(skip_count)
            .map(|e| Self::estimate_entry_tokens(e, cx))
            .sum();

        let format_tokens = |tokens: usize| -> String {
            if tokens >= 1000 {
                format!("~{}k", (tokens + 500) / 1000)
            } else {
                format!("~{}", tokens)
            }
        };

        let label_text: SharedString = if total == 0 {
            "No messages".into()
        } else if included >= total {
            format!("{} / {} ({})", included, total, format_tokens(total_tokens)).into()
        } else {
            format!(
                "{} / {} ({} / {})",
                included,
                total,
                format_tokens(included_tokens),
                format_tokens(total_tokens)
            )
            .into()
        };

        let track_height = px(6.0);
        let thumb_size = px(14.0);

        let theme = cx.theme();
        let track_bg = theme.colors().border;
        let fill_color = theme.colors().icon_accent;
        let thumb_color = theme.colors().icon_accent;
        let thumb_border = theme.colors().border;

        div()
            .id("focused-context-slider")
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div().w(px(115.0)).flex_shrink_0().child(
                    Label::new(label_text)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            )
            .child(
                div()
                    .id("focused-context-slider-track-area")
                    .flex_1()
                    .h(thumb_size + px(4.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, window, _cx| {
                        window.prevent_default();
                    })
                    .on_drag(SliderDrag, |_, _, _window, cx| cx.new(|_cx| gpui::Empty))
                    .on_drag_move::<SliderDrag>(cx.listener(
                        move |this, event: &DragMoveEvent<SliderDrag>, _window, cx| {
                            let bounds = event.bounds;
                            let relative_x_from_right =
                                bounds.origin.x + bounds.size.width - event.event.position.x;
                            let frac = relative_x_from_right / bounds.size.width;
                            this.set_num_messages_from_fraction(frac, cx);
                        },
                    ))
                    .child(
                        div()
                            .w_full()
                            .h(track_height)
                            .rounded(track_height / 2.0)
                            .bg(track_bg)
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .right_0()
                                    .top_0()
                                    .h_full()
                                    .rounded(track_height / 2.0)
                                    .bg(fill_color)
                                    .w(gpui::relative(fraction)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(-(thumb_size - track_height) / 2.0)
                                    .right(gpui::relative(fraction))
                                    .mr(-(thumb_size / 2.0))
                                    .size(thumb_size)
                                    .rounded(thumb_size / 2.0)
                                    .bg(thumb_color)
                                    .border_1()
                                    .border_color(thumb_border)
                                    .shadow_sm(),
                            ),
                    ),
            )
            .child(
                IconButton::new("focused-context-reset", IconName::RotateCcw)
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Muted)
                    .tooltip(Tooltip::text("Include all messages"))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.thread.update(cx, |thread, cx| {
                            thread.set_num_messages(None, cx);
                        });
                    })),
            )
    }

    fn render_histogram(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = self.thread.read(cx);
        let total = thread.message_count();
        let included = self.effective_num_messages(cx);
        let skip_count = total.saturating_sub(included);

        let theme = cx.theme();
        let user_color = theme.colors().icon_accent;
        let agent_color = theme.colors().icon_muted;
        let dimmed_user = user_color.opacity(0.25);
        let dimmed_agent = agent_color.opacity(0.25);

        let histogram_height = px(40.0);
        let bar_gap = px(1.0);

        struct HistogramBar {
            role: Role,
            char_count: u32,
            preview: SharedString,
            entry_index: usize,
        }

        let messages: Vec<HistogramBar> = thread
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(
                    e,
                    AgentThreadEntry::UserMessage(_) | AgentThreadEntry::AssistantMessage(_)
                )
            })
            .map(|(entry_index, entry)| {
                let role = match entry {
                    AgentThreadEntry::UserMessage(_) => Role::User,
                    AgentThreadEntry::AssistantMessage(_) => Role::Assistant,
                    _ => Role::User,
                };
                HistogramBar {
                    role,
                    char_count: Self::entry_char_count(entry, cx),
                    preview: Self::entry_preview(entry, 20, cx).into(),
                    entry_index,
                }
            })
            .collect();

        let max_chars = messages
            .iter()
            .map(|m| m.char_count)
            .max()
            .unwrap_or(1)
            .max(1);

        div()
            .w_full()
            .h(histogram_height)
            .flex()
            .flex_row()
            .items_end()
            .gap(bar_gap)
            .overflow_x_hidden()
            .justify_end()
            .children(
                messages
                    .into_iter()
                    .enumerate()
                    .map(move |(msg_index, bar)| {
                        let is_included = msg_index >= skip_count;
                        let height_fraction =
                            (bar.char_count as f32 / max_chars as f32).clamp(0.1, 1.0);
                        let is_user = bar.role == Role::User;
                        let bar_color = match (is_user, is_included) {
                            (true, true) => user_color,
                            (true, false) => dimmed_user,
                            (false, true) => agent_color,
                            (false, false) => dimmed_agent,
                        };

                        let role_label = if is_user { "User" } else { "Agent" };
                        let included_label = if is_included { "Included" } else { "Excluded" };
                        let tooltip_text: SharedString = format!(
                            "{} · {} · ~{} chars\n{}",
                            role_label, included_label, bar.char_count, bar.preview
                        )
                        .into();

                        let entry_index = bar.entry_index;
                        div()
                            .id(ElementId::NamedInteger(
                                "histogram-bar".into(),
                                msg_index as u64,
                            ))
                            .flex_1()
                            .h(histogram_height * height_fraction)
                            .bg(bar_color)
                            .rounded_t(px(1.0))
                            .cursor(CursorStyle::PointingHand)
                            .tooltip(Tooltip::text(tooltip_text))
                            .on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, _window, cx| {
                                    let total = this.thread.read(cx).message_count();
                                    let new_included = total.saturating_sub(msg_index);

                                    let num_messages = if new_included >= total {
                                        None
                                    } else {
                                        Some(new_included)
                                    };
                                    this.thread.update(cx, |thread, cx| {
                                        thread.set_num_messages(num_messages, cx);
                                    });

                                    if event.modifiers().control {
                                        cx.emit(FocusedContextPanelEvent::ScrollToEntry {
                                            entry_index,
                                        });
                                    }
                                },
                            ))
                    }),
            )
    }
}

impl Render for FocusedContextPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = self.thread.read(cx);
        let focused_context_mode = thread.focused_context_mode();
        let has_messages = thread.message_count() > 0;

        if !focused_context_mode || !has_messages {
            return div().into_any();
        }

        v_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .border_t_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Label::new("Context Window")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(Divider::horizontal().color(DividerColor::Border)),
            )
            .child(self.render_histogram(window, cx))
            .child(self.render_slider(window, cx))
            .into_any()
    }
}
