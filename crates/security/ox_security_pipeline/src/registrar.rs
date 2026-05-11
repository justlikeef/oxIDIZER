use std::sync::Mutex;
use ox_security_core::registration::{ContextDefinition, ContextRegistrar};
use crate::pipeline::SecurityPipeline;

pub struct PipelineContextRegistrar {
    _pipeline: SecurityPipeline,
    context_def: ContextDefinition,
    registrations: Mutex<Vec<ContextDefinition>>,
}

impl PipelineContextRegistrar {
    pub fn new(pipeline: SecurityPipeline, context_def: ContextDefinition) -> Self {
        Self {
            _pipeline: pipeline,
            context_def,
            registrations: Mutex::new(Vec::new()),
        }
    }

    /// Returns the `ContextDefinition` this registrar was constructed with.
    /// This is the application-level root node passed to consuming crates.
    pub fn context_definition(&self) -> ContextDefinition {
        self.context_def
    }

    /// Returns a snapshot of all registrations stored so far.
    /// Used in tests; also callable by admin code that needs to enumerate registered contexts.
    pub fn stored_registrations(&self) -> Vec<ContextDefinition> {
        self.registrations.lock().unwrap().clone()
    }
}

impl ContextRegistrar for PipelineContextRegistrar {
    fn register_context(&self, def: ContextDefinition) {
        self.registrations.lock().unwrap().push(def);
    }
}
