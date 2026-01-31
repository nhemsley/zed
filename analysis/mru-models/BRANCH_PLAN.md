# MRU Models Branch Restructuring Plan

## Current State

- **Branch**: `mru-models` (11 commits, fully working)
- **Base**: `main` at `ecc392804056`

## Goal

Split the monolithic `mru-models` branch into layered branches:

```
main
  └── mru-models-logic (DB + business logic only)
        ├── mru-models-ui-in-chooser (current UI approach)
        └── mru-models-ui-tooltip (alternative UI - to be implemented)
```

## Branch Descriptions

### `mru-models-logic`
Pure database and business logic layer with no UI dependencies.

**Scope**:
- Database schema (model_mru table)
- Data structures (ModelMruInfo)
- CRUD operations (record_model_usage, get_mru_models, get_mru_models_with_stats)
- MRU recording trigger point (in Thread::send)
- Query ordering (use_count DESC, last_used_at DESC)

**Files**:
- `crates/agent/src/db.rs`
- `crates/agent/src/thread.rs`

### `mru-models-ui-in-chooser`
Current UI implementation showing MRU section in model picker.

**Depends on**: `mru-models-logic`

**Features**:
- "Most Recently Used" section at top of model picker
- Ctrl+Alt+1-9 keybindings for quick model selection
- MRU index numbers (1-9) displayed next to models
- Ctrl+Alt hold to show model selector
- Invalid model ID filtering

**Files**:
- `assets/keymaps/default-*.json`
- `crates/zed_actions/src/lib.rs`
- `crates/agent_ui/src/acp/model_selector.rs`
- `crates/agent_ui/src/acp/model_selector_popover.rs`
- `crates/agent_ui/src/acp/config_options.rs`
- `crates/agent_ui/src/acp/thread_view.rs`
- `crates/agent_ui/src/ui/model_selector_components.rs`
- `clear_mru.sh`

### `mru-models-ui-tooltip`
Alternative UI approach (to be designed).

**Depends on**: `mru-models-logic`

**Potential features**:
- Floating tooltip showing MRU models
- Less intrusive than modifying the picker
- Could appear on hover or keyboard shortcut

---

## Execution Steps

### Step 1: Create `mru-models-logic` branch

```bash
git checkout main
git checkout -b mru-models-logic

# 1. Apply db.rs changes (schema, structs, methods)
git show 8620033649 -- crates/agent/src/db.rs | git apply
git show d265bf76e9 -- crates/agent/src/db.rs | git apply
git show 20cb60cf94 -- crates/agent/src/db.rs | git apply

# 2. Apply thread.rs changes (MRU recording)
git show d265bf76e9 -- crates/agent/src/thread.rs | git apply
git show bc4f64218e -- crates/agent/src/thread.rs | git apply

# 3. Commit
git add crates/agent/src/db.rs crates/agent/src/thread.rs
git commit -m "Add MRU (Most Recently Used) model tracking

- Add model_mru table to ThreadsDatabase schema
- Add ModelMruInfo struct for usage statistics
- Add record_model_usage() to track model usage
- Add get_mru_models() and get_mru_models_with_stats() queries
- Record MRU in Thread::send() using provider_id/model_id format
- Order MRU by use_count DESC, then last_used_at DESC
- Make ThreadsDatabase public for external access"
```

### Step 2: Verify `mru-models-logic` builds

```bash
cd crates/agent
cargo build -q
cargo test -q
```

### Step 3: Create `mru-models-ui-in-chooser` branch

```bash
git checkout mru-models-logic
git checkout -b mru-models-ui-in-chooser

# Cherry-pick UI commits (may need manual conflict resolution)
git cherry-pick 1fb2c4b90a  # MRU section in picker
git cherry-pick f7a9f44356  # Keybindings + action
git cherry-pick 3de4e7bc07  # Debug logging
git cherry-pick bc4f64218e  # Ctrl-hold modal (skip thread.rs)
git cherry-pick 509642041f  # Invalid ID filtering
git cherry-pick 6421b87be3  # Ctrl+Alt + index display
git cherry-pick 6c70f7c2d3  # MRU refresh
```

**Note**: Some cherry-picks may have conflicts due to shared files. Resolve by keeping only UI changes.

### Step 4: Verify UI branch works

```bash
cargo build -q
cargo test -q -p agent_ui
```

### Step 5: Keep original branch

```bash
# Keep mru-models as-is for reference
git branch -m mru-models mru-models-original  # Optional rename
```

---

## Commit Source Map

Which commits go where:

| Commit | Logic | UI-Chooser | Notes |
|--------|-------|------------|-------|
| 01 `8620033649` | db.rs ✅ | model_selector.rs (superseded) | Split required |
| 02 `d265bf76e9` | db.rs, thread.rs ✅ | thread_view.rs | Split required, skip .cargo/config.toml |
| 03 `1fb2c4b90a` | | ✅ | Pure UI |
| 04 `4ab53451e4` | ✅ (logging) | | Minor, can skip |
| 05 `f7a9f44356` | | ✅ | Pure UI |
| 06 `3de4e7bc07` | | ✅ | Pure UI |
| 07 `bc4f64218e` | thread.rs ✅ | Rest ✅ | Split required |
| 08 `509642041f` | | ✅ | Pure UI |
| 09 `6421b87be3` | | ✅ | Pure UI |
| 10 `20cb60cf94` | ✅ | | Pure logic |
| 11 `6c70f7c2d3` | | ✅ | Pure UI |

---

## Verification Checklist

### `mru-models-logic` branch
- [ ] `model_mru` table created on DB init
- [ ] `record_model_usage()` works
- [ ] `get_mru_models()` returns correct data
- [ ] `get_mru_models_with_stats()` returns stats with correct ordering
- [ ] MRU recorded when sending message in thread
- [ ] Model ID format is `provider_id/model_id`
- [ ] No UI code included
- [ ] Builds without errors

### `mru-models-ui-in-chooser` branch
- [ ] MRU section appears in model picker
- [ ] MRU section shows before Favorites
- [ ] Ctrl+Alt+1-9 selects MRU models
- [ ] Ctrl+Alt hold shows model picker
- [ ] MRU index numbers (1-9) displayed
- [ ] Invalid model IDs filtered out
- [ ] MRU list refreshes when picker opens
- [ ] Builds without errors

---

## Future: `mru-models-ui-tooltip` branch

Ideas for alternative UI:

1. **Floating Tooltip**
   - Shows on Ctrl+Alt hold near model selector button
   - Displays MRU models with keyboard hints
   - Dismisses on release or selection

2. **Status Bar Integration**
   - Current model shown in status bar
   - Click to see MRU dropdown

3. **Quick Switcher Panel**
   - Small floating panel
   - Always visible or toggle-able
   - Shows top 3 MRU models

4. **Command Palette Integration**
   - MRU models appear first in model-related commands
   - "Switch to recent model" command

To implement, branch from `mru-models-logic` and build new UI components.