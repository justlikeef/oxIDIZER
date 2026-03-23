-- oxWorkflow Initial Schema

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    priority INTEGER NOT NULL,
    status TEXT NOT NULL,
    flow_name TEXT,
    stage_name TEXT,
    state_blob BLOB NOT NULL,
    metadata_json TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS execution_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    stage_name TEXT NOT NULL,
    plugin_name TEXT,
    status TEXT NOT NULL,
    message TEXT,
    started_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    completed_at DATETIME,
    FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_flow ON tasks(flow_name);
