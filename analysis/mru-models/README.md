# MRU Models Branch Analysis

## Overview

The `mru-models` branch implements "Most Recently Used" (MRU) model tracking for the Zed agent/AI assistant. It allows users to quickly switch between their frequently used AI models using keyboard shortcuts and displays them in a dedicated section of the model picker.

**Branch**: `mru-models`
**Base Commit**: `ecc392804056a1daa200197f4decf2009fdab455` (from `main`)
**Total Commits**: 11

---

## Commit Summary Table

| # | Commit | Short Message | Category | Files Changed |
|---|--------|---------------|----------|---------------|
| 01 | `8620033649` | Add MRU tracking for model usage | **LOGIC+UI** | db.rs, model_selector.rs |
| 02 | `d265bf76e9` | Track model usage in Thread::send | **LOGIC** | db.rs, thread.rs, thread_view.rs |
| 03 | `1fb2c4b90a` | Add 'Most Recently Used' section to chooser | **UI** | model_selector.rs |
| 04 | `4ab53451e4` | Add detailed logging for model ID tracking | **LOGIC** | thread.rs |
| 05 | `f7a9f44356` | Add Ctrl+1-9 keybindings for MRU selection | **UI** | keymaps, thread_view.rs, zed_actions |
| 06 | `3de4e7bc07` | Add logging to MRU model picker | **UI (Debug)** | model_selector.rs |
| 07 | `bc4f64218e` | Show model selector when holding Ctrl key | **UI** | thread.rs, config_options.rs, model_selector_popover.rs, thread_view.rs |
| 08 | `509642041f` | Filter out invalid MRU model IDs | **LOGIC+UI** | clear_mru.sh, model_selector.rs, thread_view.rs |
| 09 | `6421b87be3` | Change MRU to Ctrl+Alt and show numbers | **UI** | keymaps, model_selector.rs, thread_view.rs, model_selector_components.rs |
| 10 | `20cb60cf94` | Fix MRU ordering to prioritize use count | **LOGIC** | db.rs |
| 11 | `6c70f7c2d3` | Filter invalid model IDs and refresh MRU | **UI** | model_selector.rs |

---

## Detailed Commit Analysis

### Commit 01: `8620033649` - Add MRU tracking for model usage (zed-vb7.1)
**Category**: LOGIC + UI (Mixed)

**Changes**:
- `crates/agent/src/db.rs`:
  - Creates `model_mru` table schema (model_id, last_used_at, use_count)
  - Adds `record_model_usage()` method for upserting usage data
  - Adds `get_mru_models()` method to retrieve top N models
  - Makes `ThreadsDatabase` public (`pub struct` instead of `pub(crate)`)
  
- `crates/agent_ui/src/acp/model_selector.rs`:
  - Records MRU when model is selected in picker (later moved in commit 02)

**Logic Components**: DB schema, record_model_usage, get_mru_models, public ThreadsDatabase
**UI Components**: MRU recording in picker (temporary - moved later)

---

### Commit 02: `d265bf76e9` - Track model usage in MRU database and log MRU on Ctrl hold
**Category**: LOGIC (primarily)

**Changes**:
- `crates/agent/src/db.rs`:
  - Adds `ModelMruInfo` struct for returning usage statistics
  - Adds `get_mru_models_with_stats()` method to retrieve models with full stats

- `crates/agent/src/thread.rs`:
  - **Moves MRU tracking to `Thread::send()`** - this is more accurate than picker selection
  - Records model usage when message is actually sent

- `crates/agent_ui/src/acp/model_selector.rs`:
  - Removes duplicate MRU tracking from picker (moved to thread.rs)

- `crates/agent_ui/src/acp/thread_view.rs`:
  - Adds `ModifiersChangedEvent` handler to log MRU when Ctrl is held (debug feature)

- `.cargo/config.toml`:
  - **UNRELATED**: Adds linker config for x86_64-unknown-linux-gnu (should NOT be cherry-picked)

**Logic Components**: ModelMruInfo struct, get_mru_models_with_stats, MRU recording in Thread::send
**UI Components**: Ctrl-hold debug logging (temporary debug feature)

---

### Commit 03: `1fb2c4b90a` - Add 'Most Recently Used' section to Change Model Chooser
**Category**: UI

**Changes**:
- `crates/agent_ui/src/acp/model_selector.rs`:
  - Adds `mru_model_ids: Vec<ModelId>` to `AcpModelPickerDelegate`
  - Fetches MRU models from database on picker initialization
  - Updates `info_list_to_picker_entries()` to include MRU section before Favorites
  - Displays "Most Recently Used" separator and MRU models at top of picker
  - Updates all tests to include empty MRU slice parameter

**Logic Components**: None
**UI Components**: MRU section in model picker, picker entry generation

---

### Commit 04: `4ab53451e4` - Add detailed logging for model ID tracking
**Category**: LOGIC (Debug/Fix)

**Changes**:
- `crates/agent/src/thread.rs`:
  - Adds more detailed logging showing model name, id, and provider_id
  - Comment clarifies using `model.id()` to match picker's model IDs

**Logic Components**: Logging improvements for debugging model ID mismatches
**UI Components**: None

---

### Commit 05: `f7a9f44356` - Add Ctrl+1-9 keybindings for MRU model selection
**Category**: UI

**Changes**:
- `assets/keymaps/default-linux.json`: Adds Ctrl+1-9 bindings for AcpThread context
- `assets/keymaps/default-macos.json`: Adds Ctrl+1-9 bindings for AcpThread context
- `assets/keymaps/default-windows.json`: Adds Ctrl+1-9 bindings for AcpThread context

- `crates/zed_actions/src/lib.rs`:
  - Adds `SelectMruModel` action with `index` parameter (1-9)

- `crates/agent_ui/src/acp/thread_view.rs`:
  - Implements `select_mru_model()` handler
  - Fetches MRU from database, selects model at index, queues/sends message
  - Registers action handler with `.on_action(cx.listener(Self::select_mru_model))`

**Logic Components**: None (action struct is minimal)
**UI Components**: Keybindings, SelectMruModel action, select_mru_model handler

---

### Commit 06: `3de4e7bc07` - Add logging to MRU model picker for debugging
**Category**: UI (Debug)

**Changes**:
- `crates/agent_ui/src/acp/model_selector.rs`:
  - Adds logging when MRU IDs are loaded into picker
  - Adds logging for models not found in available models list
  - Adds logging for picker entry building stats

**Logic Components**: None
**UI Components**: Debug logging in picker

---

### Commit 07: `bc4f64218e` - Show model selector when holding Ctrl key
**Category**: UI

**Changes**:
- `crates/agent/src/thread.rs`:
  - **Fixes model ID format** to `provider_id/model_id` (LOGIC FIX)

- `crates/agent_ui/src/acp/config_options.rs`:
  - Adds `is_category_picker_open()` method

- `crates/agent_ui/src/acp/model_selector_popover.rs`:
  - Adds `is_deployed()` method

- `crates/agent_ui/src/acp/thread_view.rs`:
  - Adds `model_selector_opened_by_ctrl` state field
  - Modifies `handle_modifiers_changed()` to show/hide model selector on Ctrl hold
  - Supports both `model_selector` and `config_options_view` paths

**Logic Components**: Model ID format fix (provider_id/model_id)
**UI Components**: Ctrl-hold modal behavior, is_deployed, is_category_picker_open

---

### Commit 08: `509642041f` - Filter out invalid MRU model IDs and improve error handling
**Category**: LOGIC + UI (Mixed)

**Changes**:
- `clear_mru.sh`:
  - Utility script to clear MRU table (developer tool)

- `crates/agent_ui/src/acp/model_selector.rs`:
  - Filters out model IDs without provider prefix when loading MRU
  - Logs warning for invalid/old format model IDs

- `crates/agent_ui/src/acp/thread_view.rs`:
  - Improves error handling in `select_mru_model` to gracefully skip invalid IDs

**Logic Components**: Validation logic for model ID format
**UI Components**: Error handling in UI, warning logging

---

### Commit 09: `6421b87be3` - Change MRU to Ctrl+Alt and show numbers in picker
**Category**: UI

**Changes**:
- `assets/keymaps/default-*.json`:
  - Changes keybindings from Ctrl+1-9 to Ctrl+Alt+1-9

- `crates/agent_ui/src/acp/model_selector.rs`:
  - Changes `AcpModelPickerEntry::Model` to include `mru_index: Option<usize>`
  - Updates all pattern matches and test assertions

- `crates/agent_ui/src/acp/thread_view.rs`:
  - Changes Ctrl-hold to Ctrl+Alt-hold for modal trigger

- `crates/agent_ui/src/ui/model_selector_components.rs`:
  - Adds `mru_index` field to `ModelSelectorListItem`
  - Displays MRU index numbers (1-9) next to models in picker

**Logic Components**: None
**UI Components**: Keybinding changes, MRU index display, modal trigger change

---

### Commit 10: `20cb60cf94` - Fix MRU model ordering to prioritize use count over recency
**Category**: LOGIC

**Changes**:
- `crates/agent/src/db.rs`:
  - Changes `get_mru_models_with_stats` query from `ORDER BY last_used_at DESC` 
    to `ORDER BY use_count DESC, last_used_at DESC`

**Logic Components**: Query ordering fix
**UI Components**: None

---

### Commit 11: `6c70f7c2d3` - Filter invalid model IDs and refresh MRU list in model selector
**Category**: UI

**Changes**:
- `crates/agent_ui/src/acp/model_selector.rs`:
  - Refreshes MRU models from database when showing picker (not just on init)
  - Filters invalid model IDs in update_matches
  - Ensures MRU list is always current

**Logic Components**: None
**UI Components**: MRU refresh on picker show, validation in update_matches

---

## Classification Summary

### Pure Logic/DB Commits (for `mru-models-logic` branch)
- **Commit 10** (`20cb60cf94`): Query ordering fix

### Logic-Heavy Commits (partial cherry-pick needed)
- **Commit 01** (`8620033649`): DB schema + methods (extract db.rs changes only)
- **Commit 02** (`d265bf76e9`): ModelMruInfo struct + get_mru_models_with_stats (extract db.rs changes)
- **Commit 04** (`4ab53451e4`): Model ID format in thread.rs (debatable - mostly logging)
- **Commit 07** (`bc4f64218e`): Model ID format fix in thread.rs (extract just the format fix)

### UI Commits (for `mru-models-ui-in-chooser` branch)
- **Commit 03** (`1fb2c4b90a`): MRU section in model picker
- **Commit 05** (`f7a9f44356`): Keybindings + SelectMruModel action + handler
- **Commit 06** (`3de4e7bc07`): Debug logging
- **Commit 07** (`bc4f64218e`): Ctrl-hold modal (UI parts)
- **Commit 08** (`509642041f`): Invalid ID filtering in UI
- **Commit 09** (`6421b87be3`): Ctrl+Alt + MRU index display
- **Commit 11** (`6c70f7c2d3`): MRU refresh in picker

---

## Proposed Branch Structure

### Branch: `mru-models-logic`
Pure database and business logic layer. No UI changes.

**Files to include**:
- `crates/agent/src/db.rs`:
  - `model_mru` table creation
  - `ModelMruInfo` struct
  - `record_model_usage()` method
  - `get_mru_models()` method
  - `get_mru_models_with_stats()` method (with corrected ordering)
  - Make `ThreadsDatabase` public

- `crates/agent/src/thread.rs`:
  - MRU recording in `Thread::send()` with correct `provider_id/model_id` format

**DO NOT include**:
- `.cargo/config.toml` changes (unrelated linker config)
- Any model_selector.rs changes
- Any thread_view.rs UI changes

### Branch: `mru-models-ui-in-chooser`
Current UI approach - MRU section shown in model picker.

**Depends on**: `mru-models-logic`

**Files to include**:
- All keybinding changes
- `crates/zed_actions/src/lib.rs`: SelectMruModel action
- `crates/agent_ui/src/acp/model_selector.rs`: All changes
- `crates/agent_ui/src/acp/model_selector_popover.rs`: is_deployed method
- `crates/agent_ui/src/acp/config_options.rs`: is_category_picker_open method
- `crates/agent_ui/src/acp/thread_view.rs`: All UI changes
- `crates/agent_ui/src/ui/model_selector_components.rs`: mru_index display
- `clear_mru.sh`: Utility script

### Branch: `mru-models-ui-tooltip`
Alternative UI approach (to be implemented) - MRU shown in tooltip/overlay.

**Depends on**: `mru-models-logic`

**To be designed**: Could show MRU as:
- Floating tooltip near model selector button
- Status bar indicator
- Command palette integration
- Separate quick-switch panel

---

## Cherry-Pick Guide

### Creating `mru-models-logic` branch

```bash
# Start from main
git checkout main
git checkout -b mru-models-logic

# Apply DB changes from commit 01 (db.rs only)
git show 8620033649 -- crates/agent/src/db.rs | git apply

# Apply DB changes from commit 02 (db.rs only, skip .cargo/config.toml)
git show d265bf76e9 -- crates/agent/src/db.rs | git apply

# Apply thread.rs MRU recording from commit 02
git show d265bf76e9 -- crates/agent/src/thread.rs | git apply

# Apply model ID format fix from commit 07 (just the thread.rs line)
git show bc4f64218e -- crates/agent/src/thread.rs | git apply

# Apply query ordering fix from commit 10
git cherry-pick 20cb60cf94

# Commit the changes
git add -A
git commit -m "Add MRU tracking database layer and business logic"
```

### Creating `mru-models-ui-in-chooser` branch

```bash
# Start from mru-models-logic
git checkout mru-models-logic
git checkout -b mru-models-ui-in-chooser

# Cherry-pick UI commits in order
git cherry-pick 1fb2c4b90a  # MRU section in picker
git cherry-pick f7a9f44356  # Keybindings
git cherry-pick 3de4e7bc07  # Debug logging
git cherry-pick bc4f64218e  # Ctrl-hold modal (may need conflict resolution)
git cherry-pick 509642041f  # Invalid ID filtering
git cherry-pick 6421b87be3  # Ctrl+Alt + index display
git cherry-pick 6c70f7c2d3  # MRU refresh
```

---

## Files Reference

### Diff Files Location
All individual commit diffs are exported to:
```
research/mru-models/diffs/
├── 01-8620033649.diff
├── 02-d265bf76e9.diff
├── 03-1fb2c4b90a.diff
├── 04-4ab53451e4.diff
├── 05-f7a9f44356.diff
├── 06-3de4e7bc07.diff
├── 07-bc4f64218e.diff
├── 08-509642041f.diff
├── 09-6421b87be3.diff
├── 10-20cb60cf94.diff
└── 11-6c70f7c2d3.diff
```

### Key Files Modified

| File | Logic | UI |
|------|-------|-----|
| `crates/agent/src/db.rs` | ✅ | |
| `crates/agent/src/thread.rs` | ✅ | |
| `crates/agent_ui/src/acp/model_selector.rs` | | ✅ |
| `crates/agent_ui/src/acp/thread_view.rs` | | ✅ |
| `crates/agent_ui/src/acp/config_options.rs` | | ✅ |
| `crates/agent_ui/src/acp/model_selector_popover.rs` | | ✅ |
| `crates/agent_ui/src/ui/model_selector_components.rs` | | ✅ |
| `crates/zed_actions/src/lib.rs` | | ✅ |
| `assets/keymaps/default-*.json` | | ✅ |
| `clear_mru.sh` | | ✅ (dev tool) |

---

## Notes

1. **Commit 02 Warning**: Contains unrelated `.cargo/config.toml` linker changes - DO NOT include these in any cherry-pick.

2. **Model ID Format Evolution**: 
   - Initially used `model.id()` directly
   - Fixed in commit 07 to use `provider_id/model_id` format
   - This is important for matching picker model IDs

3. **Keybinding Evolution**:
   - Started as Ctrl+1-9
   - Changed to Ctrl+Alt+1-9 to avoid conflicts

4. **Current Branch State**: `mru-models` branch is fully working with the "UI in chooser" approach. Keep it as-is for reference.