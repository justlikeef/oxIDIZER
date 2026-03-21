use ox_event_bus::EventBus;
use ox_workflow_config::EngineConfig;
use ox_workflow_core::TaskStatus;
use ox_workflow_storage::WorkflowStorage;
use ox_workflow_executor::FlowManager;
use ox_workflow_abi::CoreHostApi;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use futures::stream::StreamExt;
use tokio::sync::Semaphore;
use std::ffi::{c_char, c_void};
use ox_workflow_core::Task;

pub struct WorkflowScheduler {
    pub config: EngineConfig,
    pub storage: WorkflowStorage,
    pub event_bus: Arc<dyn EventBus>,
    pub flow_manager: Arc<FlowManager>,
    pub api_ptr: *const CoreHostApi,
    pub queues: Vec<String>,
}

unsafe impl Send for WorkflowScheduler {}
unsafe impl Sync for WorkflowScheduler {}

impl WorkflowScheduler {
    pub fn new(
        config: EngineConfig, 
        storage: WorkflowStorage, 
        event_bus: Arc<dyn EventBus>, 
        flow_manager: Arc<FlowManager>, 
        api_ptr: *const CoreHostApi,
        queues: Vec<String>,
    ) -> Self {
        Self {
            config,
            storage,
            event_bus,
            flow_manager,
            api_ptr,
            queues,
        }
    }

    /// Starts the main event loop
    pub async fn run(self: Arc<Self>) -> Result<(), String> {
        let max_concurrent = self.config.max_concurrent_tasks;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));

        log::info!("Scheduler initialized with {} max concurrent tasks", max_concurrent);

        let mut handles = Vec::new();

        for queue_name in &self.queues {
            let q_name = queue_name.clone();
            let scheduler = self.clone();
            let sem_clone = semaphore.clone();
            
            let handle = tokio::spawn(async move {
                let mut stream = match scheduler.event_bus.subscribe_to_queue(&q_name).await {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("Failed to subscribe to queue {}: {:?}", q_name, e);
                        return;
                    }
                };
                    
                log::info!("Listening on queue: {}", q_name);

                while let Some(msg) = stream.next().await {
                    // Backpressure: wait for a permit before processing
                    let permit = sem_clone.clone().acquire_owned().await.unwrap();

                    if let Ok(task_id_str) = String::from_utf8(msg.payload.clone()) {
                        if let Ok(task_id) = uuid::Uuid::parse_str(&task_id_str) {
                            let sched_clone = scheduler.clone();
                            tokio::spawn(async move {
                                sched_clone.spawn_task(task_id).await;
                                drop(permit);
                            });
                        } else {
                            drop(permit);
                        }
                    } else {
                        drop(permit);
                    }
                }
            });
            handles.push(handle);
        }

        let mut ticker = interval(Duration::from_millis(self.config.tick_interval_ms));
        let handle = tokio::spawn(async move {
            loop {
                ticker.tick().await;
                // Periodic maintenance: retry dead letters, check timeouts
            }
        });
        handles.push(handle);

        for h in handles {
            let _ = h.await;
        }

        Ok(())
    }

    async fn spawn_task(&self, task_id: uuid::Uuid) {
        let storage = self.storage.clone();
        let event_bus = self.event_bus.clone();
        
        if let Ok(Some(mut task)) = storage.load_task(task_id).await {
            
            // Flow name from task metadata or default
            let flow_name = task.metadata.get("flow_name").cloned().unwrap_or_else(|| "default".to_string());
            let flow_runner_opt = self.flow_manager.flows.get(&flow_name).cloned();
            let api_ptr_addr = self.api_ptr as usize;

            tokio::spawn(async move {
                log::debug!("Executing Task {}", task.id);
                
                if let Some(runner) = flow_runner_opt {
                    let api = unsafe { &*(api_ptr_addr as *const CoreHostApi) };
                    let fc = runner.run(&mut task, api);
                    
                    if fc.code == ox_workflow_abi::FLOW_CONTROL_SUSPEND {
                        task.status = TaskStatus::Paused;
                    } else if fc.code == ox_workflow_abi::FLOW_CONTROL_ERROR {
                        task.status = TaskStatus::Errored;
                    } else {
                        task.status = TaskStatus::Completed;
                    }
                    
                    let _ = storage.save_task(&task, Some(&flow_name), None).await;

                    for record in task.history.drain(..) {
                        let _ = storage.append_history(
                            task.id, 
                            &record.stage_name, 
                            record.plugin_name.as_deref(), 
                            &record.status, 
                            record.message.as_deref()
                        ).await;
                    }

                    for child_flow in task.child_workflows.drain(..) {
                        let mut child = Task::new(task.priority);
                        child.metadata.insert("flow_name".to_string(), child_flow.clone());
                        child.metadata.insert("parent_task_id".to_string(), task.id.to_string());
                        
                        if let Ok(_) = storage.save_task(&child, Some(&child_flow), None).await {
                            let _ = event_bus.publish_to_queue("tasks.pending", child.priority as u8, child.id.to_string().as_bytes()).await;
                        }
                    }
                } else {
                    log::error!("Flow {} not found for task {}", flow_name, task.id);
                    task.status = TaskStatus::Errored;
                    let _ = storage.update_task_status(task_id, TaskStatus::Errored).await;
                }
            });
        }
    }
}

// Global shutdown channel for cdylib
static SHUTDOWN_TX: std::sync::OnceLock<tokio::sync::broadcast::Sender<()>> = std::sync::OnceLock::new();

#[no_mangle]
pub extern "C" fn scheduler_start(
    _config_path: *const c_char,
    _runtime_handle: *const c_void,  // Would cast to tokio::runtime::Handle safely in real impl, kept c_void to avoid dependency coupling in ABI
) -> bool {
    let (tx, _) = tokio::sync::broadcast::channel(1);
    SHUTDOWN_TX.set(tx).unwrap_or(());

    // In a full implementation, this parses the config, establishes db/event bus, 
    // and spawns `scheduler.run()` on the provided tokio runtime handle.
    log::info!("scheduler_start called");
    
    true
}

#[no_mangle]
pub extern "C" fn scheduler_stop() {
    if let Some(tx) = SHUTDOWN_TX.get() {
        let _ = tx.send(());
        log::info!("scheduler_stop called");
    }
}
