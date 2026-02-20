use acp_thread::{AcpThread, AgentThreadEntry, AssistantMessageChunk};
use gpui::{
    div, px, App, Context, CursorStyle, Entity, EventEmitter, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use language_model::Role;
use ui::{prelude::*, CircularProgress, Color, Tooltip};

pub enum FocusedContextPanelEvent {
    ScrollToEntry { entry_index: usize },
}

pub struct FocusedContextPanel {
    thread: Entity<AcpThread>,
    hovered_message_index: Option<usize>,
}

impl EventEmitter<FocusedContextPanelEvent> for FocusedContextPanel {}

impl FocusedContextPanel {
    pub fn new(thread: Entity<AcpThread>, _cx: &mut Context<Self>) -> Self {
        Self {
            thread,
            hovered_message_index: None,
        }
    }

    fn effective_included_count(&self, cx: &App) -> usize {
        let thread = self.thread.read(cx);
        let total = thread.message_count();
        match thread.pinned_message_index() {
            Some(pinned_index) => total.saturating_sub(pinned_index),
            None => total,
        }
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

    /// Given a message_index (from the histogram), find the nearest user message
    /// at or before it. This mirrors the snap logic in Thread::snap_to_user_message
    /// but operates on AcpThread entries.
    fn snap_to_user_message_index(&self, message_index: usize, cx: &App) -> Option<usize> {
        let thread = self.thread.read(cx);
        let mut counter = 0usize;
        let mut last_user_index_at_or_before = None;

        for entry in thread.entries().iter() {
            match entry {
                AgentThreadEntry::UserMessage(_) => {
                    if counter <= message_index {
                        last_user_index_at_or_before = Some(counter);
                    }
                    counter += 1;
                }
                AgentThreadEntry::AssistantMessage(_) => {
                    counter += 1;
                }
                AgentThreadEntry::ToolCall(_) => {}
            }
            if counter > message_index + 1 {
                break;
            }
        }

        last_user_index_at_or_before
    }

    fn render_status(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = self.thread.read(cx);
        let total = thread.message_count();
        let included = self.effective_included_count(cx);

        let progress_color = cx.theme().colors().text_muted;
        let included_str = included.to_string();
        let total_str = total.to_string();

        div()
            .id("focused-context-status")
            .flex_shrink_0()
            .child(
                CircularProgress::new(included as f32, total as f32, px(16.0), cx)
                    .stroke_width(px(2.0))
                    .progress_color(progress_color),
            )
            .tooltip(Tooltip::element(move |_, _cx| {
                v_flex()
                    .min_w_24()
                    .child(
                        ui::Label::new("Context window")
                            .color(Color::Muted)
                            .size(ui::LabelSize::Small),
                    )
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(ui::Label::new(included_str.clone()))
                            .child(ui::Label::new("/").color(Color::Muted))
                            .child(ui::Label::new(total_str.clone()).color(Color::Muted)),
                    )
                    .into_any_element()
            }))
    }

    fn render_histogram(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = self.thread.read(cx);
        let pinned_index = thread.pinned_message_index();

        let theme = cx.theme();
        let user_color = theme.colors().icon_accent;
        let agent_color = theme.colors().icon_muted;
        let dimmed_user = user_color.opacity(0.25);
        let dimmed_agent = agent_color.opacity(0.25);
        let highlight_color = theme.colors().icon_accent.opacity(0.6);

        let histogram_height = px(40.0);
        let bar_gap = px(1.0);

        struct HistogramBar {
            role: Role,
            char_count: u32,
            preview: SharedString,
            entry_index: usize,
            message_index: usize,
            is_hover_target: bool,
        }

        let hover_snap_index = self
            .hovered_message_index
            .and_then(|idx| self.snap_to_user_message_index(idx, cx));

        let mut message_counter = 0usize;
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
                let is_hover_target = hover_snap_index == Some(message_counter);
                let bar = HistogramBar {
                    role,
                    char_count: Self::entry_char_count(entry, cx),
                    preview: Self::entry_preview(entry, 20, cx).into(),
                    entry_index,
                    message_index: message_counter,
                    is_hover_target,
                };
                message_counter += 1;
                bar
            })
            .collect();

        let max_chars = messages
            .iter()
            .map(|m| m.char_count)
            .max()
            .unwrap_or(1)
            .max(1);

        div()
            .flex_1()
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
                    .map(move |(display_index, bar)| {
                        let is_included = match pinned_index {
                            Some(pinned) => bar.message_index >= pinned,
                            None => true,
                        };
                        let height_fraction =
                            (bar.char_count as f32 / max_chars as f32).clamp(0.1, 1.0);
                        let is_user = bar.role == Role::User;

                        let bar_color = if bar.is_hover_target {
                            highlight_color
                        } else {
                            match (is_user, is_included) {
                                (true, true) => user_color,
                                (true, false) => dimmed_user,
                                (false, true) => agent_color,
                                (false, false) => dimmed_agent,
                            }
                        };

                        let role_label = if is_user { "User" } else { "Agent" };
                        let included_label = if is_included { "Included" } else { "Excluded" };
                        let tooltip_text: SharedString = format!(
                            "{} · {} · ~{} chars\n{}",
                            role_label, included_label, bar.char_count, bar.preview
                        )
                        .into();

                        let entry_index = bar.entry_index;
                        let clicked_message_index = bar.message_index;
                        let hover_message_index = bar.message_index;
                        div()
                            .id(ElementId::NamedInteger(
                                "histogram-bar".into(),
                                display_index as u64,
                            ))
                            .flex_1()
                            .h(histogram_height)
                            .flex()
                            .flex_col()
                            .justify_end()
                            .cursor(CursorStyle::PointingHand)
                            .tooltip(Tooltip::text(tooltip_text))
                            .on_hover(cx.listener(
                                move |this, hovered: &bool, _window, cx| {
                                    let new_value = if *hovered {
                                        Some(hover_message_index)
                                    } else {
                                        None
                                    };
                                    if this.hovered_message_index != new_value {
                                        this.hovered_message_index = new_value;
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, _window, cx| {
                                    let new_pinned = if clicked_message_index == 0 {
                                        None
                                    } else {
                                        Some(clicked_message_index)
                                    };
                                    this.thread.update(cx, |thread, cx| {
                                        thread.set_pinned_message_index(new_pinned, cx);
                                    });

                                    if event.modifiers().control {
                                        cx.emit(FocusedContextPanelEvent::ScrollToEntry {
                                            entry_index,
                                        });
                                    }
                                },
                            ))
                            .child(
                                div()
                                    .w_full()
                                    .h(histogram_height * height_fraction)
                                    .bg(bar_color)
                                    .rounded_t(px(1.0)),
                            )
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
            .border_t_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_end()
                    .child(self.render_status(window, cx))
                    .child(self.render_histogram(window, cx)),
            )
            .into_any()
    }
}
