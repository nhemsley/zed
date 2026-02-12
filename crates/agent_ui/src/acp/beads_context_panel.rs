use agent::{Message, Thread};
use gpui::{
    App, Context, CursorStyle, DragMoveEvent, Entity, EventEmitter, IntoElement, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use language_model::Role;
use ui::{
    Color, Divider, DividerColor, IconButton, IconName, IconSize, Label, LabelSize, Tooltip,
    prelude::*,
};

struct SliderDrag;

pub enum BeadsContextPanelEvent {
    ScrollToMessage { message_index: usize },
}

pub struct BeadsContextPanel {
    thread: Entity<Thread>,
}

impl EventEmitter<BeadsContextPanelEvent> for BeadsContextPanel {}

impl BeadsContextPanel {
    pub fn new(thread: Entity<Thread>, _cx: &mut Context<Self>) -> Self {
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

    fn estimate_message_tokens(message: &Message) -> usize {
        match message {
            Message::User(user_msg) => user_msg
                .content
                .iter()
                .map(|c| match c {
                    agent::UserMessageContent::Text(t) => t.len() / 4,
                    agent::UserMessageContent::Mention { content, .. } => content.len() / 4,
                    agent::UserMessageContent::Image(img) => img.estimate_tokens(),
                })
                .sum(),
            Message::Agent(agent_msg) => {
                let content_tokens: usize = agent_msg
                    .content
                    .iter()
                    .map(|c| match c {
                        agent::AgentMessageContent::Text(t) => t.len() / 4,
                        agent::AgentMessageContent::Thinking { text, .. } => text.len() / 4,
                        agent::AgentMessageContent::RedactedThinking(_) => 0,
                        agent::AgentMessageContent::ToolUse(tu) => tu.raw_input.len() / 4,
                    })
                    .sum();
                let tool_result_tokens: usize = agent_msg
                    .tool_results
                    .values()
                    .map(|tr| match &tr.content {
                        language_model::LanguageModelToolResultContent::Text(t) => t.len() / 4,
                        language_model::LanguageModelToolResultContent::Image(img) => {
                            img.estimate_tokens()
                        }
                    })
                    .sum();
                content_tokens + tool_result_tokens
            }
            Message::Resume => 10, // "Continue where you left off"
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

        // Calculate token estimates
        let messages = thread.messages();
        let total_tokens: usize = messages.iter().map(Self::estimate_message_tokens).sum();
        let skip_count = total.saturating_sub(included);
        let included_tokens: usize = messages
            .iter()
            .skip(skip_count)
            .map(Self::estimate_message_tokens)
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
            .id("beads-slider")
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                Label::new(label_text)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                div()
                    .id("beads-slider-track-area")
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
                            // Calculate position from the RIGHT edge to match right-anchored visual
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
                IconButton::new("beads-reset", IconName::RotateCcw)
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

    fn message_preview(message: &Message, max_len: usize) -> String {
        let text = match message {
            Message::User(user_msg) => user_msg
                .content
                .iter()
                .filter_map(|c| match c {
                    agent::UserMessageContent::Text(t) => Some(t.as_str()),
                    agent::UserMessageContent::Mention { content, .. } => Some(content.as_str()),
                    agent::UserMessageContent::Image(_) => Some("[image]"),
                })
                .collect::<Vec<_>>()
                .join(" "),
            Message::Agent(agent_msg) => agent_msg
                .content
                .iter()
                .filter_map(|c| match c {
                    agent::AgentMessageContent::Text(t) => Some(t.as_str()),
                    agent::AgentMessageContent::Thinking { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
            Message::Resume => "Continue where you left off".to_string(),
        };
        let trimmed = text.trim().replace('\n', " ");
        if trimmed.len() > max_len {
            format!("{}…", &trimmed[..max_len])
        } else {
            trimmed
        }
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
        }

        let messages: Vec<HistogramBar> = thread
            .messages()
            .iter()
            .map(|message| HistogramBar {
                role: message.role(),
                char_count: Thread::message_char_count(message),
                preview: Self::message_preview(message, 100).into(),
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
            .children(messages.into_iter().enumerate().map(move |(index, bar)| {
                let is_included = index >= skip_count;
                let height_fraction = (bar.char_count as f32 / max_chars as f32).clamp(0.1, 1.0);
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

                div()
                    .id(ElementId::NamedInteger(
                        "histogram-bar".into(),
                        index as u64,
                    ))
                    .flex_1()
                    .h(histogram_height * height_fraction)
                    .bg(bar_color)
                    .rounded_t(px(1.0))
                    .cursor(CursorStyle::PointingHand)
                    .tooltip(Tooltip::text(tooltip_text.clone()))
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, _window, cx| {
                            if !event.modifiers().control {
                                return;
                            }
                            let total = this.thread.read(cx).message_count();
                            let new_included = total.saturating_sub(index);
                            let current_included = this.effective_num_messages(cx);
                            if new_included > current_included {
                                let num_messages = if new_included >= total {
                                    None
                                } else {
                                    Some(new_included)
                                };
                                this.thread.update(cx, |thread, cx| {
                                    thread.set_num_messages(num_messages, cx);
                                });
                            }
                            cx.emit(BeadsContextPanelEvent::ScrollToMessage {
                                message_index: index,
                            });
                        }),
                    )
            }))
    }
}

impl Render for BeadsContextPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread = self.thread.read(cx);
        let beads_mode = thread.beads_mode();
        let has_messages = thread.message_count() > 0;

        if !beads_mode || !has_messages {
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
