pub mod routes {
    use axum::{
        extract::{Path, State, Query},
        http::StatusCode,
        routing::{get, post, delete},
        Json, Router,
    };
    use ox_workflow_core::{Task, TaskStatus};
    use ox_workflow_storage::WorkflowStorage;
    use ox_event_bus::EventBus;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Clone)]
    pub struct ApiState {
        pub storage: WorkflowStorage,
        pub event_bus: Arc<dyn EventBus>,
    }

    #[derive(Deserialize)]
    pub struct EnqueueFlowRequest {
        pub flow_name: String,
        pub priority: u32,
        pub metadata: std::collections::HashMap<String, String>,
    }

    #[derive(Serialize)]
    pub struct EnqueueFlowResponse {
        pub task_id: Uuid,
        pub status: String,
    }

    #[derive(Serialize)]
    pub struct TaskResponse {
        pub id: Uuid,
        pub status: TaskStatus,
        pub priority: u32,
        pub metadata: std::collections::HashMap<String, String>,
    }

    pub fn create_router() -> Router<ApiState> {
        Router::new()
            .route("/flows", post(enqueue_flow))
            .route("/tasks", get(list_tasks))
            .route("/tasks/:id", get(get_task))
            .route("/tasks/:id", delete(delete_task))
            .route("/tasks/:id/history", get(get_task_history))
            .route("/tasks/:id/cancel", post(cancel_task))
            .route("/tasks/:id/resume", post(resume_task))
    }

    pub async fn start_server(state: ApiState, addr: &str) -> std::io::Result<()> {
        let app = create_router().with_state(state);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await
    }

    #[axum::debug_handler]
    async fn enqueue_flow(
        State(state): State<ApiState>,
        Json(payload): Json<EnqueueFlowRequest>,
    ) -> Result<Json<EnqueueFlowResponse>, StatusCode> {
        let mut task = Task::new(payload.priority);
        task.metadata = payload.metadata;
        task.metadata.insert("flow_name".to_string(), payload.flow_name.clone());

        // Save to DB
        state.storage.save_task(&task, Some(&payload.flow_name), None).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Publish to pending queue to wake scheduler
        let task_id_str = task.id.to_string();
        state.event_bus.publish_to_queue("tasks.pending", payload.priority as u8, task_id_str.as_bytes())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok(Json(EnqueueFlowResponse {
            task_id: task.id.clone(),
            status: "Queued".to_string(),
        }))
    }

    #[axum::debug_handler]
    async fn get_task(
        State(state): State<ApiState>,
        Path(id): Path<Uuid>,
    ) -> Result<Json<TaskResponse>, StatusCode> {
        let task = state.storage.load_task(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        match task {
            Some(t) => Ok(Json(TaskResponse {
                id: t.id.clone(),
                status: t.status,
                priority: t.priority,
                metadata: t.metadata.clone(),
            })),
            None => Err(StatusCode::NOT_FOUND),
        }
    }

    #[axum::debug_handler]
    async fn get_task_history(
        State(state): State<ApiState>,
        Path(id): Path<Uuid>,
    ) -> Result<Json<Vec<ox_workflow_core::HistoryRecord>>, StatusCode> {
        let history = state.storage.get_task_history(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(Json(history))
    }

    #[derive(Deserialize)]
    pub struct ListTasksQuery {
        pub status: Option<String>,
        pub flow: Option<String>,
    }

    async fn list_tasks(
        State(state): State<ApiState>,
        Query(params): Query<ListTasksQuery>,
    ) -> Result<Json<Vec<TaskResponse>>, StatusCode> {
        let tasks_res = if let Some(status_str) = params.status {
            let status = match status_str.as_str() {
                "Queued" => TaskStatus::Queued,
                "Running" => TaskStatus::Running,
                "Paused" => TaskStatus::Paused,
                "Errored" => TaskStatus::Errored,
                "Completed" => TaskStatus::Completed,
                _ => return Err(StatusCode::BAD_REQUEST),
            };
            state.storage.list_tasks_by_status(status).await
        } else if let Some(flow_name) = params.flow {
            state.storage.list_tasks_by_flow(&flow_name).await
        } else {
            return Err(StatusCode::NOT_IMPLEMENTED);
        };

        match tasks_res {
            Ok(tasks) => {
                let responses = tasks.into_iter().map(|t| TaskResponse {
                    id: t.id.clone(),
                    status: t.status,
                    priority: t.priority,
                    metadata: t.metadata.clone(),
                }).collect();
                Ok(Json(responses))
            }
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    #[derive(Deserialize)]
    pub struct ResumeTaskRequest {
        pub signal: String,
    }

    #[axum::debug_handler]
    async fn resume_task(
        State(state): State<ApiState>,
        Path(id): Path<Uuid>,
        Json(_payload): Json<ResumeTaskRequest>,
    ) -> Result<StatusCode, StatusCode> {
        let task = state.storage.load_task(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(mut t) = task {
            if t.status == TaskStatus::Paused {
                t.status = TaskStatus::Queued;
                state.storage.update_task_status(id, TaskStatus::Queued).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                
                state.event_bus.publish_to_queue("tasks.pending", t.priority as u8, id.to_string().as_bytes())
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                
                Ok(StatusCode::OK)
            } else {
                Err(StatusCode::CONFLICT)
            }
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    }

    #[axum::debug_handler]
    async fn cancel_task(
        State(state): State<ApiState>,
        Path(id): Path<Uuid>,
    ) -> Result<StatusCode, StatusCode> {
        let task = state.storage.load_task(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(t) = task {
            if t.status == TaskStatus::Queued || t.status == TaskStatus::Running || t.status == TaskStatus::Paused {
                state.storage.update_task_status(id, TaskStatus::Errored).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                Ok(StatusCode::OK)
            } else {
                Err(StatusCode::CONFLICT)
            }
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    }

    #[axum::debug_handler]
    async fn delete_task(
        State(state): State<ApiState>,
        Path(id): Path<Uuid>,
    ) -> Result<StatusCode, StatusCode> {
        let task = state.storage.load_task(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if task.is_some() {
            state.storage.delete_task(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(StatusCode::NO_CONTENT)
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    }
}
