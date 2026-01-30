# Beads Mode: Sliding Context Window for Zed Agent

## Status: PROPOSED

## Problem Statement

Currently, Zed Agent accumulates all messages in a conversation thread, sending the entire history to the LLM on each request. For long-running conversations, this leads to:

1. **Token waste**: Paying for re-processing old context that may no longer be relevant
2. **Context overflow**: Eventually hitting model token limits, requiring manual truncation
3. **Slower responses**: Larger context windows take longer to process
4. **Cost inefficiency**: Especially problematic for users on metered plans

Users who want quick, focused interactions without the overhead of full conversation history currently have no lightweight option.

## Proposed Solution: Beads Mode

Add a new "Beads Mode" toggle to the Zed Agent panel that implements a sliding context window, keeping only the most recent N tokens of conversation history.

### User Experience

- New icon button next to the existing Burn Mode toggle in the message editor controls
- Clicking toggles beads mode on/off for the current thread
- When enabled, only the last 1-5k tokens of conversation are sent to the LLM
- Visual indicator shows when beads mode is active

## Architecture Overview

### Current Message Flow

```
Thread::send()
  → Thread::run_turn()
    → Thread::build_completion_request()
      → Thread::build_request_messages()
        → Iterates ALL messages in self.messages
        → Sends to LLM
```

### Key Files

| File | Purpose |
|------|---------|
| `crates/agent/src/thread.rs` | Core thread logic, message storage, request building |
| `crates/agent_ui/src/acp/thread_view.rs` | UI rendering, controls (burn mode toggle is here) |
| `crates/agent_settings/src/agent_settings.rs` | Settings definitions (CompletionMode enum) |
| `crates/acp_thread/src/acp_thread.rs` | Token usage tracking |

### Current Context Assembly (`thread.rs` L2141-2178)

```rust
fn build_request_messages(
    &self,
    available_tools: Vec<SharedString>,
    cx: &App,
) -> Vec<LanguageModelRequestMessage> {
    // System prompt always included
    let system_prompt = SystemPromptTemplate { ... }.render(&self.templates);
    let mut messages = vec![system_prompt_message];
    
    // Currently: ALL messages are included
    for message in &self.messages {
        messages.extend(message.to_request());
    }
    
    // Pending message added
    if let Some(message) = self.pending_message.as_ref() {
        messages.extend(message.to_request());
    }
    
    messages
}
```

### Current UI Controls (`thread_view.rs` L5132-5140)

```rust
// In render_message_editor()
.child(
    h_flex()
        .gap_0p5()
        .child(self.render_add_context_button(cx))
        .child(self.render_follow_toggle(cx))
        .children(self.render_burn_mode_toggle(cx))  // Beads mode would go here
)
```

## Implementation Plan

### Step 1: Add BeadsMode Setting

**File: `crates/agent_settings/src/agent_settings.rs`**

Add a new field to track beads mode state and token limit:

```rust
pub struct AgentSettings {
    // ... existing fields ...
    pub beads_mode_enabled: bool,
    pub beads_mode_token_limit: u32,  // Default: 4096
}
```

### Step 2: Add BeadsMode to Thread State

**File: `crates/agent/src/thread.rs`**

Add beads mode state to Thread struct:

```rust
pub struct Thread {
    // ... existing fields ...
    beads_mode: bool,
    beads_token_limit: u32,
}

impl Thread {
    pub fn beads_mode(&self) -> bool {
        self.beads_mode
    }
    
    pub fn set_beads_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.beads_mode = enabled;
        cx.notify();
    }
}
```

### Step 3: Implement Sliding Window in build_request_messages

**File: `crates/agent/src/thread.rs`**

Modify `build_request_messages` to respect beads mode:

```rust
fn build_request_messages(
    &self,
    available_tools: Vec<SharedString>,
    cx: &App,
) -> Vec<LanguageModelRequestMessage> {
    let system_prompt = SystemPromptTemplate { ... }.render(&self.templates);
    let mut messages = vec![system_prompt_message];
    
    if self.beads_mode {
        // Sliding window: only include recent messages up to token limit
        let mut token_count = 0;
        let mut windowed_messages = Vec::new();
        
        // Iterate in reverse to get most recent messages first
        for message in self.messages.iter().rev() {
            let message_tokens = estimate_tokens(message);
            if token_count + message_tokens > self.beads_token_limit {
                break;
            }
            token_count += message_tokens;
            windowed_messages.push(message);
        }
        
        // Reverse to restore chronological order
        windowed_messages.reverse();
        
        for message in windowed_messages {
            messages.extend(message.to_request());
        }
    } else {
        // Original behavior: include all messages
        for message in &self.messages {
            messages.extend(message.to_request());
        }
    }
    
    if let Some(message) = self.pending_message.as_ref() {
        messages.extend(message.to_request());
    }
    
    messages
}

fn estimate_tokens(message: &Message) -> u32 {
    // Rough estimation: ~4 characters per token
    // Could be made more accurate with a proper tokenizer
    match message {
        Message::User(user_msg) => {
            user_msg.content.iter().map(|c| match c {
                UserMessageContent::Text(t) => t.len() as u32 / 4,
                UserMessageContent::Mention { content, .. } => content.len() as u32 / 4,
                UserMessageContent::Image(_) => 1000, // Images count as ~1000 tokens
            }).sum()
        }
        Message::Agent(agent_msg) => {
            agent_msg.content.iter().map(|c| match c {
                AgentMessageContent::Text(t) => t.len() as u32 / 4,
                AgentMessageContent::Thinking { text, .. } => text.len() as u32 / 4,
                AgentMessageContent::RedactedThinking(_) => 0,
                AgentMessageContent::ToolUse(_) => 500, // Rough estimate for tool calls
            }).sum()
        }
        Message::Resume => 0,
    }
}
```

### Step 4: Add UI Toggle

**File: `crates/agent_ui/src/acp/thread_view.rs`**

Add beads mode toggle next to burn mode:

```rust
fn render_beads_mode_toggle(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
    let thread = self.as_native_thread(cx)?.read(cx);
    let beads_mode_enabled = thread.beads_mode();
    
    let icon = if beads_mode_enabled {
        IconName::Beads  // Need to add this icon
    } else {
        IconName::BeadsOff
    };

    Some(
        IconButton::new("beads-mode", icon)
            .icon_size(IconSize::Small)
            .icon_color(Color::Muted)
            .toggle_state(beads_mode_enabled)
            .selected_icon_color(Color::Accent)
            .on_click(cx.listener(|this, _event, window, cx| {
                this.toggle_beads_mode(window, cx);
            }))
            .tooltip(move |_window, cx| {
                Tooltip::with_meta(
                    if beads_mode_enabled { "Beads Mode (On)" } else { "Beads Mode (Off)" },
                    None,
                    "Limits context to recent messages only",
                    cx,
                )
            })
            .into_any_element(),
    )
}

fn toggle_beads_mode(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    if let Some(thread) = self.as_native_thread(cx) {
        thread.update(cx, |thread, cx| {
            let current = thread.beads_mode();
            thread.set_beads_mode(!current, cx);
        });
    }
}
```

Update `render_message_editor` to include the new toggle:

```rust
.child(
    h_flex()
        .gap_0p5()
        .child(self.render_add_context_button(cx))
        .child(self.render_follow_toggle(cx))
        .children(self.render_beads_mode_toggle(cx))  // NEW
        .children(self.render_burn_mode_toggle(cx))
)
```

### Step 5: Add Icon

**File: `crates/ui/src/icon.rs`** (or wherever icons are defined)

Add new icon variants:
- `IconName::Beads` - Active beads mode icon (could be a chain/beads visual)
- `IconName::BeadsOff` - Inactive beads mode icon

## Open Questions

1. **Token limit**: Should it be configurable? Default suggestions:
   - 2048 tokens (minimal)
   - 4096 tokens (default)
   - 8192 tokens (extended)

2. **Icon design**: What should the beads icon look like?
   - Chain links?
   - Circular beads?
   - Window/frame icon?

3. **Interaction with Burn Mode**: Should beads mode and burn mode be mutually exclusive, or can they work together?

4. **System prompt handling**: Should the system prompt count against the token limit, or always be included in addition to the limit?

5. **Token estimation**: Should we use a proper tokenizer (tiktoken) or is rough character-based estimation sufficient?

6. **Per-thread vs global**: Should beads mode be a per-thread setting or a global preference?

## Success Criteria

- [ ] Beads mode toggle visible in agent panel message editor
- [ ] Toggle correctly enables/disables sliding window
- [ ] Only recent messages sent to LLM when enabled
- [ ] Token count estimation reasonably accurate
- [ ] UI clearly indicates when beads mode is active
- [ ] Setting persists across thread sessions
- [ ] Works correctly with both native agent and text threads

## Testing Plan

1. **Unit tests**: Test `estimate_tokens` function accuracy
2. **Unit tests**: Test `build_request_messages` with beads mode on/off
3. **Integration tests**: Verify correct messages sent to mock LLM
4. **Manual testing**: Long conversations with beads mode toggling
5. **Performance testing**: Measure response time improvement with beads mode

## Related Code References

- Burn mode toggle: `thread_view.rs` L5307-5340
- CompletionMode enum: `agent_settings.rs` L114-119
- Token usage tracking: `acp_thread.rs` L835-863
- Message building: `thread.rs` L2141-2178