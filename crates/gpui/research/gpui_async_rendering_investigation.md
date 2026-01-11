# GPUI Async Rendering & Repaint Lifecycle Investigation

## Problem Summary

When using async background rendering (via `TexturedView` or custom implementations like `TexturedCanvasItemsProvider`), the UI doesn't update when renders complete **unless the user is actively interacting** (panning, scrolling, etc.).

### Observed Behavior

1. Background render threads complete and send frames via channels
2. `cx.notify()` is called to mark the view as dirty
3. **UI does NOT repaint** until user interaction occurs
4. Once user pans/scrolls, UI immediately shows the completed renders

### Current Workaround

Both `TexturedView` (in GPUI) and the infinite-canvas example use this pattern:

```rust
if should_keep_polling {
    window
        .spawn(cx, async move |cx| {
            Timer::after(Duration::from_millis(16)).await;
            cx.update(|window, _cx| {
                window.refresh();
            })
            .ok();
        })
        .detach();
}
```

This spawns an async task that waits 16ms then calls `window.refresh()` to force a repaint.

---

## Key Questions to Investigate

### 1. Why doesn't `cx.notify()` trigger a repaint when idle?

**Hypothesis**: `notify()` adds an effect to the pending effects queue, but effects are only processed when the event loop runs. When there's no user input, the event loop may be blocked/sleeping.

**Files to examine**:
- `gpui/src/app.rs` - `notify()` implementation (line ~2164)
- `gpui/src/app.rs` - `apply_notify_effect()` (line ~1396)
- `gpui/src/app.rs` - Effect processing loop

**Key code path**:
```
cx.notify(entity_id)
  → pending_effects.push_back(Effect::Notify { emitter })
  → ??? when is this processed?
```

### 2. What wakes up the event loop?

**Files to examine**:
- `gpui/src/platform/linux/wayland/client.rs` or `x11/client.rs`
- `gpui/src/platform/linux/platform.rs`
- Look for: `run()`, event loop, `poll`, `epoll`, `calloop`

**Questions**:
- Is the event loop blocking on I/O (waiting for Wayland/X11 events)?
- Is there a way to "wake" it from another thread?
- Does `window.refresh()` use a platform-specific wake mechanism?

### 3. How does `window.refresh()` differ from `cx.notify()`?

**Files to examine**:
- `gpui/src/window.rs` - `refresh()` implementation
- `gpui/src/app.rs` - `apply_refresh_effect()` (line ~1416)

**Current understanding**:
```rust
// apply_refresh_effect sets dirty flag on ALL windows
fn apply_refresh_effect(&mut self) {
    for window in self.windows.values_mut() {
        if let Some(window) = window.as_deref_mut() {
            window.refreshing = true;
            window.invalidator.set_dirty(true);
        }
    }
}
```

### 4. Is there a proper "wake from background thread" mechanism?

**Possibilities to investigate**:
- Does GPUI have a cross-thread notification system?
- Can we use platform wake mechanisms directly?
- Is there a `run_on_main_thread()` or similar?

**Search for**:
- `wake`, `wakeup`, `signal`
- `run_on_main`, `dispatch`, `post`
- `eventfd`, `pipe`, `channel` (for wake mechanisms)

---

## Architecture Overview

### Current Flow (Broken)

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ Background      │     │ Main Thread      │     │ Event Loop      │
│ Render Thread   │     │                  │     │                 │
├─────────────────┤     ├──────────────────┤     ├─────────────────┤
│                 │     │                  │     │                 │
│ render element  │     │                  │     │ SLEEPING        │
│       ↓         │     │                  │     │ (no events)     │
│ send(frame)─────┼────→│ receiver has     │     │                 │
│                 │     │ data, but...     │     │                 │
│                 │     │                  │     │                 │
│                 │     │ render() not     │     │ still sleeping  │
│                 │     │ called!          │     │                 │
└─────────────────┘     └──────────────────┘     └─────────────────┘
```

### Desired Flow

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ Background      │     │ Main Thread      │     │ Event Loop      │
│ Render Thread   │     │                  │     │                 │
├─────────────────┤     ├──────────────────┤     ├─────────────────┤
│                 │     │                  │     │                 │
│ render element  │     │                  │     │ SLEEPING        │
│       ↓         │     │                  │     │                 │
│ send(frame)─────┼────→│ receiver has     │     │                 │
│       ↓         │     │ data             │     │                 │
│ WAKE ══════════════════════════════════════════→│ WOKEN!         │
│                 │     │       ↓          │     │       ↓         │
│                 │     │ render() called  │←────│ process effects │
│                 │     │ UI updated!      │     │                 │
└─────────────────┘     └──────────────────┘     └─────────────────┘
```

---

## Files to Examine

### Core GPUI Files

| File | Purpose | Key Functions |
|------|---------|---------------|
| `src/app.rs` | Application context, effect processing | `notify()`, `apply_*_effect()`, effect loop |
| `src/window.rs` | Window management | `refresh()`, `draw()`, invalidation |
| `src/executor.rs` | Task execution | `spawn()`, `ForegroundExecutor` |
| `src/platform/linux/platform.rs` | Platform abstraction | `run()`, event loop setup |
| `src/platform/linux/wayland/client.rs` | Wayland event handling | Event dispatch, wake mechanism |

### Our Custom Files

| File | Purpose | Issue |
|------|---------|-------|
| `src/textured_view.rs` | TexturedView implementation | Uses Timer workaround |
| `infinite-canvas/src/textured_provider.rs` | Canvas texture provider | Same Timer workaround |

---

## Potential Solutions

### Option 1: Fix the Wake Mechanism

Add a proper cross-thread wake to GPUI's event loop:

```rust
// Hypothetical API
impl App {
    /// Wake the event loop from any thread
    pub fn wake(&self) {
        self.platform.wake_event_loop();
    }
}

// In background thread:
app_handle.wake();
```

**Pros**: Clean, efficient, no polling
**Cons**: Requires platform-specific implementation

### Option 2: Use Existing Channel + Wake

If GPUI already has internal channels that wake the event loop:

```rust
// Use GPUI's internal dispatch mechanism
cx.spawn(|cx| async move {
    // This runs on main thread, waking event loop
    cx.update(|_, cx| cx.notify()).ok();
}).detach();
```

**Question**: Does `spawn()` wake the event loop when the future is ready?

### Option 3: Platform Event Injection

Inject a synthetic event to wake the loop:

```rust
// Send a custom user event through the platform
platform.post_empty_event();
```

### Option 4: Improve the Timer Workaround

If we must use timers, at least make them efficient:

```rust
// Only schedule ONE refresh, not one per render() call
if !self.refresh_scheduled {
    self.refresh_scheduled = true;
    window.spawn(cx, async move |cx| {
        Timer::after(Duration::from_millis(16)).await;
        cx.update(|window, cx| {
            // Reset flag and refresh
            window.refresh();
        }).ok();
    }).detach();
}
```

---

## Test Cases

### Test 1: Verify notify() behavior

```rust
#[test]
fn test_notify_triggers_render() {
    // Create view
    // Call cx.notify() from timer (not user input)
    // Assert render() is called
}
```

### Test 2: Verify cross-thread wake

```rust
#[test]
fn test_background_thread_wake() {
    // Spawn background thread
    // Send message to main thread
    // Assert main thread receives it without user input
}
```

### Test 3: Measure latency

```rust
#[test]
fn test_render_latency() {
    // Record time when frame sent from background
    // Record time when render() sees the frame
    // Assert latency < 32ms (2 frames)
}
```

---

## Investigation Log

### Session 1: Initial Discovery

**Date**: 2025-01-11

**Findings**:
1. `cx.notify()` alone does NOT trigger repaint when idle
2. User interaction (pan/scroll) processes pending effects
3. `window.refresh()` via `window.spawn()` + `Timer` works around this
4. Both `TexturedView` and `TexturedCanvasItemsProvider` use this workaround

**Logs showing the issue**:
```
[04:17:20] render() CALLED: ready=0/6, active=4, pending=2
[04:17:20] render(): active=4 pending=2, calling cx.notify()
           ^^^ notify called, but no more render() until...
[04:17:22] PAN START  ← user interaction!
[04:17:22] render() CALLED: ready=0/6, active=4, pending=6
           ^^^ NOW render is called again
```

**After adding window.refresh() workaround**:
```
[04:19:48] render() CALLED: ready=0/6, active=4, pending=2
[04:19:48] render(): active=4 pending=2, scheduling refresh
[04:19:49] render() CALLED: ready=0/6, active=4, pending=2
           ^^^ render called every ~16ms without user input!
```

---

## Next Steps

1. [ ] Examine GPUI event loop implementation
2. [ ] Find where `notify()` effects are processed
3. [ ] Determine why event loop doesn't wake on pending effects
4. [ ] Look for existing wake mechanisms in platform code
5. [ ] Prototype a proper cross-thread wake
6. [ ] Consider if this is a GPUI bug or expected behavior
7. [ ] Document findings and propose fix

---

## Related Issues

- Streaming mode in `TexturedView` marked as "not working" - likely same root cause
- Any async data loading (network, file I/O) would hit this same issue
- Animation systems would need continuous refresh anyway

---

## References

- GPUI source: `vendor/zed/crates/gpui/`
- `TexturedView`: `vendor/zed/crates/gpui/src/textured_view.rs`
- `TexturedCanvasItemsProvider`: `crates/infinite-canvas/src/textured_provider.rs`
- Linux platform: `vendor/zed/crates/gpui/src/platform/linux/`
