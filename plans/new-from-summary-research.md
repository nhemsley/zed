# New from Summary - Research Notes

## Overview

"New from Summary" is a feature that allows users to start a new agent thread that includes a summary of a previous thread. This is useful when hitting token limits or wanting to continue work in a fresh context while preserving key information from the original conversation.

## User Experience

1. **Trigger Points:**
   - Menu option: "New From Summary" in the thread toolbar dropdown menu
   - Token limit callout: When a thread hits token limits, a callout appears with a "New From Summary" button

2. **What Happens:**
   - A new thread is created
   - The previous thread's summary is automatically inserted as a mention in the message editor
   - The user can then continue the conversation with the LLM having context about what was discussed

## Code Flow

### 1. Action Definition

```rust
// crates/agent_ui/src/agent_ui.rs
pub struct NewNativeAgentThreadFromSummary {
    from_session_id: agent_client_protocol::SessionId,
}
```

### 2. Action Handler Registration

```rust
// crates/agent_ui/src/agent_panel.rs L97-102
.register_action(
    |workspace, action: &NewNativeAgentThreadFromSummary, window, cx| {
        if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
            panel.update(cx, |panel, cx| {
                panel.new_native_agent_thread_from_summary(action, window, cx)
            });
        }
    }
)
```

### 3. Panel Method

```rust
// crates/agent_ui/src/agent_panel.rs L784-804
fn new_native_agent_thread_from_summary(
    &mut self,
    action: &NewNativeAgentThreadFromSummary,
    window: &mut Window,
    cx: &mut Context<Self>,
) {
    // Get the thread metadata from the thread store using session_id
    let thread = self.thread_store.read(cx)
        .thread_from_session_id(&action.from_session_id);
    
    // Call external_thread with summarize_thread parameter
    self.external_thread(
        Some(ExternalAgent::NativeAgent),
        None,                    // resume_thread: None
        Some(thread.clone()),    // summarize_thread: Some(...)
        window,
        cx,
    );
}
```

### 4. Thread View Creation

```rust
// crates/agent_ui/src/agent_panel.rs L1543-1553
AcpThreadView::new(
    server,
    resume_thread,       // None for new-from-summary
    summarize_thread,    // The thread to summarize
    workspace.clone(),
    project,
    // ... other params
)
```

### 5. Message Editor Initialization

```rust
// crates/agent_ui/src/acp/thread_view.rs L427-431
if let Some(entry) = summarize_thread {
    editor.insert_thread_summary(entry, window, cx);
}
```

### 6. Summary Insertion

```rust
// crates/agent_ui/src/acp/message_editor.rs L270-313
pub fn insert_thread_summary(
    &mut self,
    thread: agent::DbThreadMetadata,
    window: &mut Window,
    cx: &mut Context<Self>,
) {
    // Create a Thread mention URI
    let uri = MentionUri::Thread {
        id: thread.id.clone(),
        name: thread.title.to_string(),
    };
    
    // Format as a link and insert into editor
    let content = format!("{}\n", uri.as_link());
    
    // Set the text in the editor
    self.editor.update(cx, |editor, cx| {
        editor.set_text(content, window, cx);
        // ...
    });
    
    // Confirm the mention completion to make it interactive
    self.mention_set.update(cx, |mention_set, cx| {
        mention_set.confirm_mention_completion(/* ... */)
    });
}
```

## Summary Generation

The thread summary is generated lazily when needed:

```rust
// crates/agent/src/thread.rs L1862-1925
pub fn summary(&mut self, cx: &mut Context<Self>) -> Shared<Task<Option<SharedString>>> {
    // Return cached summary if available
    if let Some(summary) = self.summary.as_ref() {
        return Task::ready(Some(summary.clone())).shared();
    }
    
    // Return pending task if already generating
    if let Some(task) = self.pending_summary_generation.clone() {
        return task;
    }
    
    // Generate new summary using the summarization model
    let model = self.summarization_model.clone();
    let mut request = LanguageModelRequest {
        intent: Some(CompletionIntent::ThreadContextSummarization),
        // ...
    };
    
    // Include all messages in the request
    for message in &self.messages {
        request.messages.extend(message.to_request());
    }
    
    // Add summarization prompt
    request.messages.push(LanguageModelRequestMessage {
        content: vec![SUMMARIZE_THREAD_DETAILED_PROMPT.into()],
        // ...
    });
    
    // Stream completion and build summary
    // ...
}
```

## Key Data Structures

### DbThreadMetadata
```rust
// crates/agent/src/db.rs L27-32
pub struct DbThreadMetadata {
    pub id: acp::SessionId,
    pub title: SharedString,
    pub updated_at: DateTime<Utc>,
}
```

### MentionUri::Thread
```rust
MentionUri::Thread {
    id: SessionId,
    name: String,
}
```

## Token Limit Callout

When token limits are reached, a callout is shown:

```rust
// crates/agent_ui/src/acp/thread_view.rs L6364-6393
fn render_token_limit_callout(&self, ...) {
    let description = if burn_mode_available {
        "To continue, start a new thread from a summary or turn Burn Mode on."
    } else {
        "To continue, start a new thread from a summary."
    };
    
    // Button dispatches NewNativeAgentThreadFromSummary action
    Button::new("new-from-summary", "New From Summary")
        .on_click(|_, window, cx| {
            window.dispatch_action(
                NewNativeAgentThreadFromSummary {
                    from_session_id: session_id,
                }
                .boxed_clone(),
                cx,
            );
        })
}
```

## Relevance to Beads Mode

Beads Mode and "New from Summary" serve similar purposes:
- Both help manage token limits in long conversations
- Both preserve context while reducing token count

**Key Difference:**
- **Beads Mode**: Sliding window that automatically excludes old messages (same thread)
- **New from Summary**: Creates a new thread with a condensed summary (new thread)

**Potential Integration:**
- Beads Mode could potentially use the summary generation logic
- The token limit callout could mention Beads Mode as another option
- Summary could be shown when messages are excluded by beads mode

## Files Referenced

| File | Purpose |
|------|---------|
| `crates/agent_ui/src/agent_ui.rs` | Action definition |
| `crates/agent_ui/src/agent_panel.rs` | Action handling, thread creation |
| `crates/agent_ui/src/acp/thread_view.rs` | Thread view creation, token limit callout |
| `crates/agent_ui/src/acp/message_editor.rs` | Summary insertion into editor |
| `crates/agent/src/thread.rs` | Summary generation |
| `crates/agent/src/db.rs` | DbThreadMetadata struct |