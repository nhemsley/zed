# MRU Models Logic Changes Reference

This document contains the specific code changes needed for the `mru-models-logic` branch.

---

## File: `crates/agent/src/db.rs`

### 1. Add ModelMruInfo struct (after existing imports/structs)

```rust
#[derive(Debug, Clone)]
pub struct ModelMruInfo {
    pub model_id: String,
    pub last_used_at: String,
    pub use_count: i64,
}
```

### 2. Change ThreadsDatabase visibility

```rust
// Change from:
pub(crate) struct ThreadsDatabase {
// To:
pub struct ThreadsDatabase {
```

### 3. Add model_mru table creation in `ThreadsDatabase::new()`

Add after the `threads` table creation:

```rust
connection.exec(indoc! {"
    CREATE TABLE IF NOT EXISTS model_mru (
        model_id TEXT PRIMARY KEY,
        last_used_at TEXT NOT NULL,
        use_count INTEGER NOT NULL DEFAULT 1
    )
"})?()
.map_err(|e| anyhow!("Failed to create model_mru table: {}", e))?;
```

### 4. Add record_model_usage method

```rust
pub fn record_model_usage(&self, model_id: String) -> Task<Result<()>> {
    let connection = self.connection.clone();

    self.executor.spawn(async move {
        log::info!("MRU: Recording model usage: {}", model_id);
        let connection = connection.lock();
        let now = Utc::now().to_rfc3339();

        let mut upsert = connection.exec_bound::<(String, String, String)>(indoc! {"
            INSERT INTO model_mru (model_id, last_used_at, use_count)
            VALUES (?, ?, 1)
            ON CONFLICT(model_id) DO UPDATE SET
                last_used_at = ?,
                use_count = use_count + 1
        "})?;

        upsert((model_id.clone(), now.clone(), now))?;
        log::info!("MRU: Successfully recorded model usage: {}", model_id);

        Ok(())
    })
}
```

### 5. Add get_mru_models method

```rust
pub fn get_mru_models(&self, limit: usize) -> Task<Result<Vec<String>>> {
    let connection = self.connection.clone();

    self.executor.spawn(async move {
        log::debug!("MRU: Fetching top {} MRU models", limit);
        let connection = connection.lock();

        let mut select_all = connection.select_bound::<(), (String, String, i64)>(indoc! {"
            SELECT model_id, last_used_at, use_count FROM model_mru
            ORDER BY use_count DESC, last_used_at DESC
        "})?;

        let rows = select_all(())?;
        let model_ids: Vec<String> = rows
            .into_iter()
            .take(limit)
            .map(|(model_id, _, _)| model_id)
            .collect();

        log::info!("MRU: Retrieved {} models: {:?}", model_ids.len(), model_ids);
        Ok(model_ids)
    })
}
```

### 6. Add get_mru_models_with_stats method

```rust
pub fn get_mru_models_with_stats(&self, limit: usize) -> Task<Result<Vec<ModelMruInfo>>> {
    let connection = self.connection.clone();

    self.executor.spawn(async move {
        log::debug!("MRU: Fetching top {} MRU models with stats", limit);
        let connection = connection.lock();

        let mut select_all = connection.select_bound::<(), (String, String, i64)>(indoc! {"
            SELECT model_id, last_used_at, use_count FROM model_mru
            ORDER BY use_count DESC, last_used_at DESC
        "})?;

        let rows = select_all(())?;
        let model_infos: Vec<ModelMruInfo> = rows
            .into_iter()
            .take(limit)
            .map(|(model_id, last_used_at, use_count)| ModelMruInfo {
                model_id,
                last_used_at,
                use_count,
            })
            .collect();

        log::info!(
            "MRU: Retrieved {} models with stats: {:?}",
            model_infos.len(),
            model_infos
                .iter()
                .map(|m| format!("{}({})", m.model_id, m.use_count))
                .collect::<Vec<_>>()
        );
        Ok(model_infos)
    })
}
```

---

## File: `crates/agent/src/thread.rs`

### 1. Add import for ThreadsDatabase

```rust
use crate::{
    // ... existing imports ...
    ThreadsDatabase,
    // ...
};
```

### 2. Add MRU recording in Thread::send_existing (or send)

Add at the beginning of the send method, after getting the model:

```rust
pub fn send_existing(
    // ... params ...
) -> Result<mpsc::UnboundedReceiver<Result<ThreadEvent>>> {
    let model = self.model().context("No language model configured")?;

    log::info!(
        "Thread::send called with model: name={}, id={}, provider_id={}",
        model.name().0,
        model.id().0,
        model.provider_id().0
    );

    // Record model usage in MRU - use provider_id/model_id format to match picker's model IDs
    let model_id = format!("{}/{}", model.provider_id().0, model.id().0);
    let database_future = ThreadsDatabase::connect(cx);
    cx.background_spawn(async move {
        match database_future.await {
            Ok(db) => {
                db.record_model_usage(model_id).await.log_err();
            }
            Err(e) => {
                log::error!("MRU: Failed to connect to database: {:?}", e);
            }
        }
    })
    .detach();

    // ... rest of send method ...
}
```

---

## Key Design Decisions

### Model ID Format
The model ID stored in the MRU table uses the format `provider_id/model_id`:
- Example: `anthropic/claude-sonnet-4-20250514`
- This matches how models are identified in the UI picker
- Enables looking up the correct model from the available models list

### Ordering Strategy
MRU models are ordered by:
1. `use_count DESC` - Most frequently used first
2. `last_used_at DESC` - Among equal counts, most recent first

This ensures that heavily-used models stay at the top even if you briefly try a new model.

### Recording Trigger
MRU is recorded in `Thread::send()` rather than in the model picker selection:
- More accurate: tracks actual usage, not just selection
- Avoids recording models that were selected but never used
- Single point of recording, no duplication

---

## Database Schema

```sql
CREATE TABLE IF NOT EXISTS model_mru (
    model_id TEXT PRIMARY KEY,      -- format: provider_id/model_id
    last_used_at TEXT NOT NULL,     -- RFC3339 timestamp
    use_count INTEGER NOT NULL DEFAULT 1
);
```

---

## Public API Surface

From `crates/agent/src/db.rs`:

```rust
// Struct for returning MRU data with statistics
pub struct ModelMruInfo {
    pub model_id: String,
    pub last_used_at: String,
    pub use_count: i64,
}

// ThreadsDatabase methods
impl ThreadsDatabase {
    // Record that a model was used (insert or increment count)
    pub fn record_model_usage(&self, model_id: String) -> Task<Result<()>>;
    
    // Get top N model IDs ordered by usage
    pub fn get_mru_models(&self, limit: usize) -> Task<Result<Vec<String>>>;
    
    // Get top N models with full statistics
    pub fn get_mru_models_with_stats(&self, limit: usize) -> Task<Result<Vec<ModelMruInfo>>>;
}
```

---

## Testing Notes

The logic layer can be tested independently:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[gpui::test]
    async fn test_mru_recording(cx: &mut TestAppContext) {
        let db = ThreadsDatabase::new_in_memory(cx.background_executor().clone()).unwrap();
        
        // Record some usage
        db.record_model_usage("provider/model-a".to_string()).await.unwrap();
        db.record_model_usage("provider/model-b".to_string()).await.unwrap();
        db.record_model_usage("provider/model-a".to_string()).await.unwrap();
        
        // Check ordering
        let mru = db.get_mru_models(10).await.unwrap();
        assert_eq!(mru[0], "provider/model-a"); // Used twice
        assert_eq!(mru[1], "provider/model-b"); // Used once
    }
}
```
