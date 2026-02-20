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

/// Status returned by module execution to control pipeline flow.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineStatus {
    Continue,
    JumpTo(String),
}

/// A generic trait for any executable module in the pipeline.
pub trait PipelineModule: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, state: State) -> Result<PipelineStatus, String>;
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
        let mut stage_idx = 0;
        'stage_loop: while stage_idx < self.stages.len() {
            let stage = &self.stages[stage_idx];
            
            for module in &stage.modules {
                let result = module.execute(initial_state.clone());
                match result {
                    Ok(PipelineStatus::Continue) => {},
                    Ok(PipelineStatus::JumpTo(target_phase)) => {
                        // Find matching stage
                        if let Some(pos) = self.stages.iter().position(|s| s.name == target_phase) {
                             // Jump to that stage index
                             stage_idx = pos;
                             // Break module loop to restart outer loop at new index
                             continue 'stage_loop; 
                        } else {
                             // Target not found - treat as error or ignore? 
                             // Treating as error for safety
                             return PipelineResult::Aborted(format!("JumpTo target phase '{}' not found", target_phase), initial_state);
                        }
                    },
                    Err(e) => return PipelineResult::Aborted(e, initial_state),
                }
            }
            stage_idx += 1;
        }
        PipelineResult::Completed(initial_state)
    }
}
