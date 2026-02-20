# Focused Context Mode — Rebase Instructions

## Overview

This feature adds a **Focused Context Mode** to agent threads that limits the conversation context sent to the LLM to only recent messages, enabling longer conversations without hitting context limits.

**Branch:** `focused-context-mode` (4 commits on top of `ecc39280`)
**Target:** Rebase onto current `main` (~1025 commits ahead)

---

## What This Feature Does

- Toggle button in the thread view footer to enable/disable focused context mode
- Interactive slider to adjust the number of recent messages included
- Visual histogram showing message distribution (user vs. agent message colors)
- Click histogram bars to adjust the context window; Ctrl+click to also scroll to that message
- Arrow button on each user message to set context window start point
- Auto-caps at a configurable limit (default 100 messages) when first enabled
- Persists mode state and message count per thread in the database

---

## Directory Contents

```
patches/focused-context-mode/
├── INSTRUCTIONS.md                 ← This file
├── full-combined.patch             ← Single combined diff of ALL changes vs merge-base
├── 0001-Add-Focused-Context-Mode-for-agent-threads.patch   ← git format-patch (commit 1)
├── 0002-fix-scrolling-from-histogram.patch                  ← git format-patch (commit 2)
├── 0003-Fix-reentrancy-panic-when-opening-agent-threads.patch ← git format-patch (commit 3)
├── 0004-Fix-focused-context-mode-initialization-for-new-thre.patch ← git format-patch (commit 4)
├── diff-*.patch                    ← Per-file diffs (our changes only, vs merge-base)
├── assets/
│   └── zed_focused_context.svg     ← New icon asset
└── final-state/
    ├── assets/icons/zed_focused_context.svg
    └── crates/agent_ui/src/acp/focused_context_panel.rs   ← Entirely new file (399 lines)
```

---

## Staggered Rebase Plan

Apply changes in this order. Each stage is independently compilable (though the feature won't fully work until all stages are done).

### Stage 1: Data Model & Settings (low conflict risk)

**Files:**
- `crates/settings/src/settings_content/agent.rs` — Add `focused_context_message_limit: Option<usize>` field
- `crates/agent_settings/src/agent_settings.rs` — Add `focused_context_message_limit: usize` field + deserialization
- `assets/settings/default.json` — Add `"focused_context_message_limit": 100` entry under `agent`
- `crates/agent/src/db.rs` — Add `focused_context_mode: Option<bool>` and `num_messages: Option<usize>` to `DbThread`

**What to do:**

1. In `crates/settings/src/settings_content/agent.rs`, add after `show_turn_stats`:
   ```rust
   pub focused_context_message_limit: Option<usize>,
   ```
   With doc comment: "When focused context mode is enabled, automatically cap the context window at this many messages."

2. In `crates/agent_settings/src/agent_settings.rs`:
   - Add `pub focused_context_message_limit: usize` to `AgentSettings` struct
   - In `Settings::from_all()`, add: `focused_context_message_limit: agent.focused_context_message_limit.unwrap_or(100),`

3. In `assets/settings/default.json`, under the `agent` section after `"show_turn_stats": false,` add:
   ```json
   "focused_context_message_limit": 100,
   ```

4. In `crates/agent/src/db.rs`, add two fields to `DbThread`:
   ```rust
   #[serde(default)]
   pub focused_context_mode: Option<bool>,
   #[serde(default)]
   pub num_messages: Option<usize>,
   ```
   Initialize them to `None` in all constructors (`SharedThread::to_db_thread`, `DbThread::from_thread`, tests).

**Patch files:** `diff-settings_content_agent.patch`, `diff-agent_settings.patch`, `diff-db.patch`, `diff-thread_store.patch`

---

### Stage 2: Thread (core message windowing logic)

**Files:**
- `crates/agent/src/thread.rs` — Add fields + `build_request_messages()` windowing logic

**What to do:**

1. Add fields to `Thread` struct:
   ```rust
   focused_context_mode: bool,
   num_messages: Option<usize>,
   ```

2. Initialize in `Thread::new()`: `focused_context_mode: false, num_messages: None`

3. In `Thread::from_db()`, restore from DbThread:
   ```rust
   focused_context_mode: db_thread.focused_context_mode.unwrap_or(false),
   num_messages: db_thread.num_messages,
   ```

4. In `Thread::to_db_thread()` / save method, persist:
   ```rust
   focused_context_mode: Some(self.focused_context_mode),
   num_messages: self.num_messages,
   ```

5. Add accessor/mutator methods:
   - `focused_context_mode() -> bool`
   - `set_focused_context_mode(&mut self, enabled: bool, cx)` — sets + notifies
   - `num_messages() -> Option<usize>`
   - `set_num_messages(&mut self, num_messages: Option<usize>, cx)` — sets + notifies
   - `message_count() -> usize` — returns `self.messages.len()`
   - `messages() -> &[Message]`

6. Modify `build_request_messages()` — after the system prompt, apply windowing:
   ```rust
   if self.focused_context_mode {
       let take_count = self.num_messages.unwrap_or(self.messages.len());
       let skip_count = self.messages.len().saturating_sub(take_count);
       for message in self.messages.iter().skip(skip_count) {
           messages.extend(message.to_request());
       }
   } else {
       for message in &self.messages {
           messages.extend(message.to_request());
       }
   }
   ```

**Patch file:** `diff-thread.patch`

**Conflict notes:** `thread.rs` has had significant changes on main. The `build_request_messages` function may have been refactored. Look for wherever messages are iterated and `to_request()` is called — that's where the windowing logic goes.

---

### Stage 3: ACP Thread Layer (connection trait + state management)

**Files:**
- `crates/acp_thread/src/connection.rs` — Add trait methods
- `crates/acp_thread/src/acp_thread.rs` — Add state fields + methods

**What to do:**

1. In `AgentConnection` trait (`connection.rs`), add two default methods:
   ```rust
   fn set_focused_context_mode(&self, _session_id: &acp::SessionId, _enabled: bool, _cx: &mut App) {}
   fn set_num_messages(&self, _session_id: &acp::SessionId, _num_messages: Option<usize>, _cx: &mut App) {}
   ```

2. In `AcpThread` struct, add fields:
   ```rust
   focused_context_mode: bool,
   num_messages: Option<usize>,
   ```

3. Add methods to `AcpThread`:
   - `focused_context_mode() -> bool`
   - `set_focused_context_mode(&mut self, enabled: bool, cx)` — auto-caps at `focused_context_message_limit` when enabling if `num_messages` is `None`, calls connection methods, notifies
   - `restore_focused_context_state(&mut self, focused_context_mode: bool, num_messages: Option<usize>, cx)` — sets fields without notifying connection (used during init)
   - `num_messages() -> Option<usize>`
   - `set_num_messages(&mut self, num_messages: Option<usize>, cx)` — calls connection, notifies
   - `message_count() -> usize` — counts `UserMessage` and `AssistantMessage` entries

**Patch files:** `diff-connection.patch`, `diff-acp_thread.patch`

**Conflict notes:** `acp_thread.rs` has been modified on main. The struct fields at the end should be straightforward to add. The `connection.rs` trait changes should be low-conflict since they're adding new default methods.

---

### Stage 4: Agent (NativeAgent bridge + persistence)

**Files:**
- `crates/agent/src/agent.rs` — Wire AcpThread ↔ Thread, implement connection trait methods, persist to DB

**What to do:**

1. In `NativeAgent::create_session()` (or wherever `AcpThread::new()` is called), after creating the acp_thread, restore state:
   ```rust
   let focused_context_mode = thread.focused_context_mode();
   let num_messages = thread.num_messages();
   // ... cx.new(|cx| AcpThread::new(...)) ...
   if focused_context_mode || num_messages.is_some() {
       acp_thread.update(cx, |acp_thread, cx| {
           acp_thread.restore_focused_context_state(focused_context_mode, num_messages, cx);
       });
   }
   ```
   **IMPORTANT:** The restore call must be OUTSIDE the `cx.new()` closure to avoid reentrancy panics.

2. In the save/persist method (where `db_thread` is built before `database.save_thread()`):
   ```rust
   let focused_context_mode = session.acp_thread.upgrade()
       .map(|acp| acp.read(cx).focused_context_mode());
   let num_messages = session.acp_thread.upgrade()
       .and_then(|acp| acp.read(cx).num_messages());
   // ... later in the spawn:
   let mut db_thread = db_thread.await;
   if let Some(focused_context_mode) = focused_context_mode {
       db_thread.focused_context_mode = Some(focused_context_mode);
   }
   db_thread.num_messages = num_messages;
   ```

3. Implement `AgentConnection` trait methods on `NativeAgentConnection`:
   ```rust
   fn set_focused_context_mode(&self, session_id, enabled, cx) {
       // Look up session.thread, call thread.update(cx, |t, cx| t.set_focused_context_mode(enabled, cx))
   }
   fn set_num_messages(&self, session_id, num_messages, cx) {
       // Look up session.thread, call thread.update(cx, |t, cx| t.set_num_messages(num_messages, cx))
   }
   ```

**Patch file:** `diff-agent.patch`

**Conflict notes:** `agent.rs` is the HIGHEST conflict risk file — it has been extensively modified on main (refactoring, new features, reentrancy fixes). Pay close attention to:
- Where sessions are created (the `cx.new()` closure pattern)
- Where threads are saved to DB
- The `AgentConnection` trait impl block

---

### Stage 5: Icon Registration

**Files:**
- `assets/icons/zed_focused_context.svg` — New file (copy from `assets/` dir)
- `crates/icons/src/icons.rs` — Add `ZedFocusedContext` variant

**What to do:**

1. Copy `assets/zed_focused_context.svg` to `assets/icons/zed_focused_context.svg`

2. In `crates/icons/src/icons.rs`, add `ZedFocusedContext` to the `IconName` enum (alphabetically near `ZedBurnMode`). The icon path mapping should follow the existing pattern — the file is `zed_focused_context.svg`.

**Patch files:** `diff-icons.patch`, asset file in `assets/`

---

### Stage 6: UI — Focused Context Panel (new file, zero conflicts)

**Files:**
- `crates/agent_ui/src/acp/focused_context_panel.rs` — **Entirely new file** (399 lines)
- `crates/agent_ui/src/acp.rs` — Add `mod focused_context_panel;`

**What to do:**

1. Copy `final-state/crates/agent_ui/src/acp/focused_context_panel.rs` into position.

2. Add `mod focused_context_panel;` to `crates/agent_ui/src/acp.rs` (alphabetically).

3. The panel may need import adjustments if APIs have changed on main. Key dependencies:
   - `acp_thread::AcpThread`, `AgentThreadEntry`
   - `gpui::*` (Entity, Context, Window, Render, etc.)
   - `ui::*` (div, h_flex, v_flex, IconButton, Tooltip, etc.)
   - `agent_settings::AgentSettings`

**Patch files:** `diff-focused_context_panel.patch`, `diff-acp-mod.patch`
**Full file:** `final-state/crates/agent_ui/src/acp/focused_context_panel.rs`

---

### Stage 7: UI — Thread View Integration (highest conflict risk)

**Files:**
- `crates/agent_ui/src/acp/thread_view.rs` — Wire panel into thread view

**What to do:**

1. Add fields to `AcpThreadView`:
   ```rust
   focused_context_panel: Option<Entity<focused_context_panel::FocusedContextPanel>>,
   _focused_context_subscription: Option<Subscription>,
   ```

2. Initialize both to `None` in `AcpThreadView::new()`.

3. When a thread is set/opened (look for where `config_options_view` and similar are created), create the panel:
   ```rust
   this.focused_context_panel = this.thread().map(|thread| {
       cx.new(|cx| focused_context_panel::FocusedContextPanel::new(thread.clone(), cx))
   });
   this._focused_context_subscription = this.focused_context_panel.as_ref().map(|panel| {
       cx.subscribe_in(panel, window, Self::handle_focused_context_panel_event)
   });
   ```

4. Add the toggle button in the footer (near burn mode toggle):
   ```rust
   .children(self.render_focused_context_mode_toggle(cx))
   ```

5. Implement `render_focused_context_mode_toggle()` — renders an `IconButton` with `IconName::ZedFocusedContext`, toggles state on click.

6. Implement `toggle_focused_context_mode()` — calls `thread.update(cx, |t, cx| t.set_focused_context_mode(!current, cx))`.

7. Add `set_context_from_entry()` method — given an entry index, counts message entries before it and sets `num_messages` accordingly.

8. In `render_entry()` for `UserMessage`, add a small arrow button that calls `set_context_from_entry(entry_ix)`.

9. Handle `FocusedContextPanelEvent::ScrollToEntry` — scrolls the list to the given entry index.

10. In the `Render` impl, add `.children(self.focused_context_panel.clone())` to render the panel in the view.

**Patch file:** `diff-thread_view.patch`

**Conflict notes:** `thread_view.rs` is the MOST heavily modified file on main. The file was **significantly refactored** — an `active_thread.rs` submodule was removed and merged back in, there have been major UI changes, and the struct fields and render methods have changed. You will need to:
- Find the equivalent struct, constructor, and render methods in the current main version
- Adapt the field additions and panel creation to the current code structure
- Find where footer buttons are rendered and add the toggle there
- The `render_entry` changes for user messages may need significant adaptation

---

## Conflict Hotspots (ranked by risk)

| File | Risk | Reason |
|------|------|--------|
| `crates/agent_ui/src/acp/thread_view.rs` | 🔴 HIGH | Major refactoring on main, `active_thread.rs` removed/merged |
| `crates/agent/src/agent.rs` | 🔴 HIGH | Extensive changes, session management refactored |
| `crates/agent/src/thread.rs` | 🟡 MEDIUM | `build_request_messages` may have changed |
| `crates/acp_thread/src/acp_thread.rs` | 🟡 MEDIUM | Struct modified, new fields added on main |
| `crates/agent/src/db.rs` | 🟢 LOW | Additive changes only |
| `crates/acp_thread/src/connection.rs` | 🟢 LOW | Adding default trait methods |
| `crates/agent_settings/src/agent_settings.rs` | 🟢 LOW | Additive field |
| `crates/agent_ui/src/acp/focused_context_panel.rs` | 🟢 NONE | Entirely new file |
| `assets/icons/zed_focused_context.svg` | 🟢 NONE | New file |
| `crates/icons/src/icons.rs` | 🟢 NONE | Additive enum variant |

---

## Quick-Start: Attempting a Direct Rebase

If you want to try a straight rebase first:

```sh
git checkout focused-context-mode
git rebase main
# Resolve conflicts as they arise
```

If that's too painful, use the staggered approach above — create a new branch from main and apply changes stage by stage, referencing the per-file diffs and the full new file in `final-state/`.

---

## Key Design Decisions to Preserve

1. **Windowing is applied in `Thread::build_request_messages()`** — this is the core mechanism. The system prompt is always included; only user/assistant messages are windowed.

2. **State flows: AcpThread → (connection trait) → Thread** — `AcpThread` is the source of truth for the UI; it forwards to `Thread` via `AgentConnection` trait methods.

3. **Restoration without connection notification** — `restore_focused_context_state()` exists specifically so that loading from DB doesn't trigger redundant connection calls. This is called OUTSIDE `cx.new()` to avoid reentrancy panics.

4. **Auto-capping** — When focused context mode is first enabled and `num_messages` is `None`, it auto-caps at `focused_context_message_limit` from settings (if the thread has more messages than the limit).

5. **Persistence** — `focused_context_mode` and `num_messages` are stored in `DbThread` and round-trip through save/load.