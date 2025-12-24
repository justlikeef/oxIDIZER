use std::collections::HashMap;
use std::sync::{Arc, RwLock}; // still needed for RwLock if used, but State is Any now
use std::any::Any;

/// Generic Pipeline State.
/// A generic container for state passed between modules.
pub type State = Arc<dyn Any + Send + Sync>;

/// result of a Pipeline Execution
#[derive(Debug, Clone)]
pub enum PipelineResult {
    Completed(State),
    Aborted(String, State),
}

/// A generic trait for any executable module in the pipeline.
pub trait PipelineModule: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, state: State) -> Result<(), String>;
    fn get_config(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

/// A named stage containing a list of modules.
pub struct Stage {
    pub name: String,
    pub modules: Vec<Box<dyn PipelineModule>>,
}

/// The Pipeline Executor.
pub struct Pipeline {
    pub stages: Vec<Stage>,
}

impl Pipeline {
    pub fn new(stages: Vec<Stage>) -> Self {
        Self { stages }
    }

    pub fn start(&self, initial_state: State) -> PipelineResult {
        for stage in &self.stages {
            for module in &stage.modules {
                if let Err(e) = module.execute(initial_state.clone()) {
                    return PipelineResult::Aborted(e, initial_state);
                }
                
                // Flow control logic removal:
                // Since State is Any, "ox_pipeline" cannot know how to read "ox.flow_control".
                // The implementation of `module.execute` (which is Host logic) must handle checks and return Err if halted,
                // OR `State` must explicitly support a generic `is_halted()`?
                // Given "The generic function... is to do nothing more than pass the state", I remove the generic flow control check.
                // The `PipelineModule::execute` return value (Result) indicates success (continue) or failure (abort).
                // A logic "Halt" can be treated as an Err("Halted") or we add `PipelineStatus` enum to execute return?
                // I'll stick to Result. Modules return Err to stop.
            }
        }
        PipelineResult::Completed(initial_state)
    }
}
