use ox_workflow_core::{Task, TaskStatus};
use parking_lot::RwLock;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;
use ox_workflow_core::state::TaskState;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    DbError(#[from] sqlx::Error),
    #[error("Migration error: {0}")]
    MigrateError(#[from] sqlx::migrate::MigrateError),
    #[error("Protobuf codec error: {0}")]
    CodecError(#[from] prost::DecodeError),
    #[error("JSON codec error: {0}")]
    JsonError(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct WorkflowStorage {
    pool: SqlitePool,
}

impl WorkflowStorage {
    /// Creates a new storage instance and runs migrations
    pub async fn new(db_url: &str) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(db_url)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    /// Saves a task (insert or update)
    pub async fn save_task(&self, task: &Task, flow_name: Option<&str>, stage_name: Option<&str>) -> Result<(), StorageError> {
        let state_blob = {
            let state_guard = task.state.read();
            state_guard.to_proto_bytes()
        };

        let metadata_json = serde_json::to_string(&task.metadata)?;
        let status_str = format!("{:?}", task.status);

        sqlx::query(
            r#"
            INSERT INTO tasks (id, priority, status, flow_name, stage_name, state_blob, metadata_json, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO UPDATE SET
                priority = excluded.priority,
                status = excluded.status,
                flow_name = excluded.flow_name,
                stage_name = excluded.stage_name,
                state_blob = excluded.state_blob,
                metadata_json = excluded.metadata_json,
                updated_at = CURRENT_TIMESTAMP
            "#
        )
        .bind(task.id.to_string())
        .bind(task.priority)
        .bind(status_str)
        .bind(flow_name)
        .bind(stage_name)
        .bind(state_blob)
        .bind(metadata_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Loads a single task by ID
    pub async fn load_task(&self, id: Uuid) -> Result<Option<Task>, StorageError> {
        let id_str = id.to_string();
        let row = sqlx::query(
            r#"
            SELECT priority, status, state_blob, metadata_json
            FROM tasks WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let status_str: String = row.try_get("status")?;
            let status = match status_str.as_str() {
                "Queued" => TaskStatus::Queued,
                "Running" => TaskStatus::Running,
                "Paused" => TaskStatus::Paused,
                "Errored" => TaskStatus::Errored,
                "Completed" => TaskStatus::Completed,
                _ => TaskStatus::Errored, // Fallback
            };

            let state_blob: Vec<u8> = row.try_get("state_blob")?;
            let metadata_json: String = row.try_get("metadata_json")?;

            let priority: i64 = row.try_get("priority")?;

            let state = TaskState::from_proto_bytes(&state_blob)?;
            let metadata: HashMap<String, String> = serde_json::from_str(&metadata_json)?;

            Ok(Some(Task {
                id,
                priority: priority as u32,
                status,
                state: Arc::new(RwLock::new(state)),
                metadata,
                flags: ox_workflow_core::TaskFlags::new(),
                child_workflows: Vec::new(),
                history: Vec::new(),
                error_callback: None,
                ffi_arena: Vec::new(),
                ffi_bytes_arena: Vec::new(),
                api_call_counts: std::collections::HashMap::new(),
                api_limits: std::collections::HashMap::new(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Update just the status of a task
    pub async fn update_task_status(&self, id: Uuid, status: TaskStatus) -> Result<(), StorageError> {
        let status_str = format!("{:?}", status);
        sqlx::query("UPDATE tasks SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(status_str)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a task
    pub async fn delete_task(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn append_history(&self, task_id: Uuid, stage_name: &str, plugin_name: Option<&str>, status: &str, message: Option<&str>) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO execution_history (task_id, stage_name, plugin_name, status, message)
            VALUES (?, ?, ?, ?, ?)
            "#
        )
        .bind(task_id.to_string())
        .bind(stage_name)
        .bind(plugin_name)
        .bind(status)
        .bind(message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_task_history(&self, task_id: Uuid) -> Result<Vec<ox_workflow_core::HistoryRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT stage_name, plugin_name, status, message FROM execution_history WHERE task_id = ? ORDER BY started_at ASC"
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut history = Vec::new();
        for row in rows {
            let stage_name: String = row.try_get("stage_name")?;
            let plugin_name: Option<String> = row.try_get("plugin_name")?;
            let status: String = row.try_get("status")?;
            let message: Option<String> = row.try_get("message")?;
            
            history.push(ox_workflow_core::HistoryRecord {
                stage_name,
                plugin_name,
                status,
                message,
            });
        }
        Ok(history)
    }

    async fn fetch_tasks(&self, query_str: &str, bind_val: &str) -> Result<Vec<Task>, StorageError> {
        let rows = sqlx::query(query_str)
            .bind(bind_val)
            .fetch_all(&self.pool)
            .await?;
        
        let mut tasks = Vec::new();
        for row in rows {
            let id_str: String = row.try_get("id")?;
            let id = Uuid::parse_str(&id_str).unwrap_or_default();
            
            let status_str: String = row.try_get("status")?;
            let status = match status_str.as_str() {
                "Queued" => TaskStatus::Queued,
                "Running" => TaskStatus::Running,
                "Paused" => TaskStatus::Paused,
                "Errored" => TaskStatus::Errored,
                "Completed" => TaskStatus::Completed,
                _ => TaskStatus::Errored,
            };

            let state_blob: Vec<u8> = row.try_get("state_blob")?;
            let metadata_json: String = row.try_get("metadata_json")?;
            let priority: i64 = row.try_get("priority")?;

            let state = TaskState::from_proto_bytes(&state_blob)?;
            let metadata: HashMap<String, String> = serde_json::from_str(&metadata_json)?;

            tasks.push(Task {
                id,
                priority: priority as u32,
                status,
                state: Arc::new(RwLock::new(state)),
                metadata,
                flags: ox_workflow_core::TaskFlags::new(),
                child_workflows: Vec::new(),
                history: Vec::new(),
                error_callback: None,
                ffi_arena: Vec::new(),
                ffi_bytes_arena: Vec::new(),
                api_call_counts: std::collections::HashMap::new(),
                api_limits: std::collections::HashMap::new(),
            });
        }
        Ok(tasks)
    }

    pub async fn list_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>, StorageError> {
        let status_str = format!("{:?}", status);
        self.fetch_tasks("SELECT id, priority, status, state_blob, metadata_json FROM tasks WHERE status = ?", &status_str).await
    }

    pub async fn list_tasks_by_flow(&self, flow_name: &str) -> Result<Vec<Task>, StorageError> {
        self.fetch_tasks("SELECT id, priority, status, state_blob, metadata_json FROM tasks WHERE flow_name = ?", flow_name).await
    }
}
