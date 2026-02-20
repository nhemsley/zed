# TODO: Audit Cache Control Passthrough

We must not bypass or interfere with Zed's upstream cache_control handling.

## What to verify

- [ ] `build_request_messages()` in `thread.rs`: confirm we only set `cache: true` on the last message, same as upstream. No extra breakpoints, no removed breakpoints.
- [ ] `into_anthropic()` in `language_models/src/provider/anthropic.rs`: confirm our message filtering (skipping via `pinned_message_index`) doesn't change how `cache_control` is applied to the messages that remain.
- [ ] System prompt: confirm we don't alter the system prompt construction or its serialization format (`StringOrContents::String` vs `Content`).
- [ ] Message ordering: confirm skipped messages don't leave behind orphaned tool results or tool uses that could change how Anthropic merges adjacent same-role messages (which affects which content block gets `cache_control`).

## What could go wrong

1. **Orphaned tool results.** If `pinned_message_index` lands in the middle of a tool-use/tool-result pair, the tool result appears without its corresponding tool use. Anthropic may reject this or it changes the message structure enough to bust caching.
2. **Role merging changes.** `into_anthropic()` merges adjacent same-role messages. If pin mode removes a message that was separating two same-role blocks, they merge into one, changing the content block structure and cache key.
3. **Extra cache breakpoints.** If we ever add a second `cache: true` marker (e.g., on system prompt), confirm it matches what upstream does. Anthropic allows 4 breakpoints but each one has cost implications.

## Context

See `docs/pin-mode-cache-miss-analysis.md` for background on cache miss sources. The system prompt instability issue is separate from this audit — this is specifically about ensuring our pin mode message filtering doesn't accidentally change the cache_control behavior on the messages we do send.