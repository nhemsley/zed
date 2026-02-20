# Pin Mode Cache Miss Analysis

## What Changed

Old: sliding window (`num_messages: Option<usize>`) — reverse offset from end, "last N messages."
New: pinned index (`pinned_message_index: Option<usize>`) — absolute forward index, "everything from message N onward."

Pin mode produces a stable prefix, which is correct for Anthropic's cumulative cache keys. The data model change is sound.

## Why We're Getting Cache Misses

### 1. System Prompt Instability (PRIMARY SUSPECT)

`build_request_messages()` regenerates the system prompt every request from `SystemPromptTemplate`, which includes:
- `ProjectContext` (worktrees, rules files, OS, arch, shell)
- `available_tools` (tool name list)
- `model_name`

Anthropic's cache keys are cumulative — each block's hash depends on ALL preceding content. The system prompt is the first message, the root of the hash chain. If it changes by a single byte, **the entire cache is invalidated**.

During agent tool use, `ProjectContext.worktrees` changes constantly (files created/deleted/modified), so the system prompt is almost certainly different between turns.

### 2. No Cache Breakpoint on System Prompt

The system message is emitted as `StringOrContents::String(...)` (plain string), which cannot carry `cache_control`. The only cache breakpoint is on the last user/assistant message:

```rust
// thread.rs — only breakpoint
if let Some(last_message) = messages.last_mut() {
    last_message.cache = true;
}
```

```rust
// anthropic.rs — system sent as plain string, no cache_control possible
system: Some(anthropic::StringOrContents::String(system_message))
```

Anthropic looks backwards from the breakpoint for a prefix match, but if the system prompt (the root) changed, nothing matches.

## Recommendations

### HIGH IMPACT

1. **Cache the system prompt as a content block.** Change `StringOrContents::String(...)` to `StringOrContents::Content(vec![RequestContent::Text { text, cache_control: Some(CacheControl { cache_type: Ephemeral }) }])`. This gives Anthropic an explicit cache breakpoint on the system prompt (2 breakpoints total, Anthropic allows 4).

2. **Stabilize the system prompt.** Either snapshot it at conversation start and reuse it, or ensure `ProjectContext` serialization is deterministic and doesn't change from file operations during tool use.

### LOW IMPACT

3. **Normalize `Some(0)` to `None`.** Toggling pin mode ON sets `Some(0)`, which produces identical messages to `None` (session mode). Not a cache issue, but a semantic oddity worth cleaning up.