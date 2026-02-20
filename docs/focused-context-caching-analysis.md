# Focused Context Mode & Anthropic Prompt Caching Analysis

## Summary

**Pin mode is cache-optimal.** By pinning context to a specific message index rather than using a sliding window, the message prefix sent to Anthropic stays stable as the conversation grows. This means every subsequent request gets a cache hit on the prefix, identical to session mode — but with a smaller context window.

## Modes

1. **Session** — all messages included (default). Prefix grows monotonically. Perfect caching.
2. **Pin** — user pins to a message index; everything from that index forward is context. Prefix is stable and grows forward. Perfect caching, smaller window.

## How Zed Builds Requests

In `crates/agent/src/thread.rs`, `build_request_messages()`:

1. System prompt is always first.
2. In **session mode**: all messages included.
3. In **pin mode**: messages from `pinned_message_index` forward are included.
4. The last message gets `cache: true` → `cache_control: {"type": "ephemeral"}` on the Anthropic API.

## How Anthropic's Prompt Caching Works

Key mechanics from [Anthropic's docs](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching):

- Cache keys are **cumulative** — the hash for each block depends on all content before it.
- The system checks up to **20 blocks backwards** from a `cache_control` breakpoint.
- Cache entries have a **5-minute TTL** (refreshed on hit, or 1-hour at 2x cost).
- **Exact matching** required — any change to the prefix invalidates everything after the change point.
- Minimum cacheable length: **1024 tokens** (Sonnet/Opus 4.x) or **4096 tokens** (Opus 4.5/4.6, Haiku 4.5).

### Pricing (per million tokens, Sonnet 4.x as reference)

| Type | Cost |
|------|------|
| Base input | $3 |
| Cache write (5min) | $3.75 (1.25x) |
| Cache read | $0.30 (0.1x) |
| Output | $15 |

Cache reads are **10x cheaper** than base input.

## Why Pin Mode Is Cache-Optimal

### The Problem with Sliding Windows (Old Cap Mode)

A sliding window of the last N messages shifts the prefix every turn:

```
Turn 1: [System] [Msg11..Msg30]  → cache write
Turn 2: [System] [Msg12..Msg31]  → CACHE MISS (Msg11 gone, Msg12 now first)
Turn 3: [System] [Msg13..Msg32]  → CACHE MISS (prefix shifted again)
```

Every turn is a full cache miss because the first message in the array changes.

### Pin Mode: Stable Prefix

Pinning to an index means the start of context is fixed. New messages append to the end:

```
Turn 1: [System] [Msg15] [Msg16] ... [Msg30]         → cache write
Turn 2: [System] [Msg15] [Msg16] ... [Msg30] [Msg31]  → cache HIT, write Msg31
Turn 3: [System] [Msg15] [Msg16] ... [Msg31] [Msg32]  → cache HIT, write Msg32
```

The prefix from `[System]` through `[Msg30]` is identical across all three requests. Anthropic hashes it, finds the cached KV state, and only processes the new tail. This is the same caching behavior as session mode.

### Session Mode (For Comparison)

```
Turn 1: [System] [Msg1..Msg30]         → cache write
Turn 2: [System] [Msg1..Msg30] [Msg31]  → cache HIT, write Msg31
Turn 3: [System] [Msg1..Msg30..Msg32]   → cache HIT, write Msg32
```

Identical caching pattern. The only difference is pin mode skips early messages, so total token count is smaller.

## Cost Comparison

Consider a 200-message conversation (~400k tokens total). User pins at message 150 (~80k tokens from pin forward).

### Session mode (all 400k tokens, cache hit):
- 400k × $0.30/MTok = **$0.12 read** + small write for new message
- Fast (cached prefill), but processes 400k tokens of KV state

### Pin mode (80k tokens from pin, cache hit):
- 80k × $0.30/MTok = **$0.024 read** + small write for new message
- 5x cheaper input cost, faster prefill, model stays focused

### Pin mode (first request after pinning, cache miss):
- 80k × $3.75/MTok = **$0.30 write** (one-time cost)
- Subsequent turns hit cache at $0.024/request

The one-time cache write pays for itself after ~2 subsequent cache-hit turns vs session mode pricing.

## When Pin Mode Helps Most

1. **Long conversations** — pinning past irrelevant early context saves tokens and money on every turn.
2. **Direction changes** — user realizes early messages are leading the model astray, pins to the turning point.
3. **Multi-turn tool loops** — agent makes many sequential requests; the pinned prefix is stable so every request after the first is a cache hit.
4. **Context limit pressure** — approaching the model's max context window, pin to shed old messages without losing cache efficiency.

## Data Model

```
// Old (sliding window, cache-hostile)
num_messages: Option<usize>  // reverse offset from end

// New (pinned index, cache-friendly)
pinned_message_index: Option<usize>  // absolute index, None = session mode
```

- `None` → session mode (all messages)
- `Some(0)` → equivalent to session mode
- `Some(15)` → context starts at message 15, grows forward

## Potential Future Optimizations

1. **Cache breakpoint on system prompt**: Add a second `cache_control` breakpoint on the system prompt so tools/instructions stay cached even when the user changes the pin point. Anthropic allows up to 4 breakpoints; we use 1 today.

2. **Automatic caching**: Set `"cache_control": {"type": "ephemeral"}` at the request level to let Anthropic auto-manage the breakpoint on the last block, in addition to an explicit breakpoint on the system prompt.

3. **1-hour TTL for system prompt**: Tool definitions and instructions rarely change. A 1-hour cache (2x write cost) with $0.30/MTok reads across many requests could pay off for heavy users.

## Conclusion

Pin mode gives users control over context scope while preserving Anthropic's prompt caching. The prefix is stable and grows forward — the ideal pattern for cumulative cache keys. Users get smaller context, lower cost, better latency, and better model focus, without sacrificing cache efficiency.