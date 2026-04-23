use ox_workflow_abi::{FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_ERROR};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WasmPluginError {
    #[error("Failed to compile WASM module: {0}")]
    CompileError(String),
    #[error("Failed to instantiate: {0}")]
    InstantiationError(String),
    #[error("Memory limit exceeded")]
    MemoryLimitExceeded,
    #[error("Init failed")]
    InitFailed,
    #[error("Plugin trap: {0}")]
    Trap(String),
}

pub struct WasmPluginConfig {
    pub name: String,
    pub memory_limit_pages: u32,
    pub max_calls_per_task: u64,
}

impl Default for WasmPluginConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            memory_limit_pages: 64,
            max_calls_per_task: 1000,
        }
    }
}

pub struct WasmPluginInstance {
    config: WasmPluginConfig,
    #[allow(dead_code)]
    compiled: bool,
}

impl WasmPluginInstance {
    pub fn from_wasm_bytes(
        wasm_bytes: &[u8],
        config: WasmPluginConfig,
    ) -> Result<Self, WasmPluginError> {
        let _ = wasm_bytes;
        Ok(Self {
            config,
            compiled: false,
        })
    }

    pub fn instantiate(self) -> Result<WasmPlugin, WasmPluginError> {
        let memory_limit = (self.config.memory_limit_pages as u64) * 65536;
        Ok(WasmPlugin {
            config: self.config,
            ctx: None,
            call_count: 0,
            memory_allocated: 0,
            memory_limit,
        })
    }
}

pub struct WasmPlugin {
    config: WasmPluginConfig,
    ctx: Option<i32>,
    call_count: u64,
    memory_allocated: u64,
    memory_limit: u64,
}

impl WasmPlugin {
    fn alloc(&mut self, size: u32) -> Result<u32, WasmPluginError> {
        let size_u64 = size as u64;
        if self.memory_allocated + size_u64 > self.memory_limit {
            return Err(WasmPluginError::MemoryLimitExceeded);
        }
        let offset = self.memory_allocated;
        self.memory_allocated += size_u64;
        Ok(offset as u32)
    }

    pub fn init(&mut self, config_json: &str) -> Result<i32, WasmPluginError> {
        let offset = self.alloc(config_json.len() as u32 + 1)?;

        self.ctx = Some(offset as i32);
        self.call_count = 0;

        Ok(offset as i32)
    }

    pub fn process(&mut self, _task_ctx: i32) -> Result<FlowControl, WasmPluginError> {
        if self.ctx.is_none() {
            return Err(WasmPluginError::InitFailed);
        }

        if self.call_count >= self.config.max_calls_per_task {
            return Ok(FlowControl {
                code: FLOW_CONTROL_ERROR,
                payload: std::ptr::null(),
            });
        }

        self.call_count += 1;

        Ok(FlowControl {
            code: FLOW_CONTROL_CONTINUE,
            payload: std::ptr::null(),
        })
    }

    pub fn error(&self, _task_ctx: i32) {}

    pub fn destroy(&mut self) {
        self.ctx = None;
    }

    pub fn stats(&self) -> WasmPluginStats {
        WasmPluginStats {
            name: self.config.name.clone(),
            calls: self.call_count,
            memory_used: self.memory_allocated,
            memory_limit: self.memory_limit,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WasmPluginStats {
    pub name: String,
    pub calls: u64,
    pub memory_used: u64,
    pub memory_limit: u64,
}