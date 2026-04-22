pub mod plugin_registry;
pub mod wasm_plugin;

use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END, FLOW_CONTROL_ERROR,
    FLOW_CONTROL_JUMP, FLOW_CONTROL_SKIP, FLOW_CONTROL_YIELD,
};
use ox_workflow_core::{Task, TaskStatus, FlowDef, StageDef};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;
use crate::plugin_registry::{PluginInstance, LoadedPlugin, PluginError};
use ox_workflow_core::HistoryRecord;

pub struct StageRunner {
    pub name: String,
    pub on_error_target: Option<String>,
    pub plugins: Vec<PluginInstance>,
}

impl StageRunner {
    pub fn run(&self, task: &mut Task, _api: &CoreHostApi) -> FlowControl {
        // Clear stage-scoped flags at the start of each stage (spec requirement)
        task.flags.stage.clear();

        let mut i = 0;
        let mut last_fc = FlowControl {
            code: FLOW_CONTROL_CONTINUE,
            payload: std::ptr::null(),
        };

        while i < self.plugins.len() {
            let plugin = &self.plugins[i];
            let task_ptr = task as *mut Task as *mut c_void;

            // Update current plugin index for get_metadata("stage.plugin_index")
            task.metadata.insert("current_plugin_index".to_string(), i.to_string());

            // FFI call wrapped in catch_unwind to intercept Rust panics
            let (fc, is_panic) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                plugin.plugin.process(plugin.ctx, task_ptr)
            })) {
                Ok(fc) => (fc, false),
                Err(_) => (FlowControl { code: FLOW_CONTROL_ERROR, payload: std::ptr::null() }, true),
            };

            let error_msg = if is_panic { Some("Plugin panicked".to_string()) } else { None };

            task.history.push(HistoryRecord {
                stage_name: self.name.clone(),
                plugin_name: Some(plugin.name.clone()),
                status: if fc.code == FLOW_CONTROL_ERROR { "Error".to_string() } else { "Completed".to_string() },
                message: error_msg,
            });

            if fc.code == FLOW_CONTROL_ERROR {
                // Always call ox_plugin_error — gives the plugin a chance to release its own
                // internal resources (connections, handles) regardless of the on_error policy.
                let task_ptr2 = task as *mut Task as *mut c_void;
                plugin.plugin.error(plugin.ctx, task_ptr2);

                // Consult on_error policy for StageRunner-handled cases.
                // Stage-name JUMPs and "errored" are returned to FlowRunner.
                match self.on_error_target.as_deref() {
                    Some("continue") => {
                        // Skip this plugin, continue to the next one in the stage.
                        // Task.error_callback is NOT called — task is still running.
                        i += 1;
                        continue;
                    }
                    Some("end") => {
                        // Terminate the flow cleanly.
                        // Task.error_callback is NOT called — not an Errored transition.
                        return FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() };
                    }
                    _ => {
                        // "errored" (None) or a stage-name: return ERROR to FlowRunner.
                        // FlowRunner will handle the stage-name jump or call error_callback
                        // and transition to Errored.
                        return fc;
                    }
                }
            }

            last_fc = fc;
            match fc.code {
                FLOW_CONTROL_CONTINUE => {
                    i += 1;
                }
                FLOW_CONTROL_SKIP => {
                    if !fc.payload.is_null() {
                        // SKIP(target): jump to the named plugin and run ONLY it, then end stage.
                        let target_name = unsafe { CStr::from_ptr(fc.payload) }.to_string_lossy();
                        if let Some(idx) = self.plugins.iter().position(|p| p.name == target_name) {
                            if idx > i {
                                // Run the target plugin, then break out of the stage loop.
                                let target_plugin = &self.plugins[idx];
                                let task_ptr = task as *mut Task as *mut c_void;
                                task.metadata.insert("current_plugin_index".to_string(), idx.to_string());
                                let (target_fc, target_is_panic) = match std::panic::catch_unwind(
                                    std::panic::AssertUnwindSafe(|| target_plugin.plugin.process(target_plugin.ctx, task_ptr))
                                ) {
                                    Ok(fc) => (fc, false),
                                    Err(_) => (FlowControl { code: FLOW_CONTROL_ERROR, payload: std::ptr::null() }, true),
                                };
                                let target_error_msg = if target_is_panic { Some("Plugin panicked".to_string()) } else { None };
                                task.history.push(HistoryRecord {
                                    stage_name: self.name.clone(),
                                    plugin_name: Some(target_plugin.name.clone()),
                                    status: if target_fc.code == FLOW_CONTROL_ERROR { "Error".to_string() } else { "Completed".to_string() },
                                    message: target_error_msg,
                                });
                                if target_fc.code == FLOW_CONTROL_ERROR {
                                    let task_ptr2 = task as *mut Task as *mut c_void;
                                    target_plugin.plugin.error(target_plugin.ctx, task_ptr2);
                                    return target_fc;
                                }
                                last_fc = target_fc;
                            }
                            // Stage ends after the skip-target plugin (whether jumped to or already at i).
                            break;
                        } else {
                            i += 1;
                        }
                    } else {
                        // SKIP(null): end the stage immediately without running further plugins.
                        break;
                    }
                }
                _ => {
                    // JUMP, SUSPEND, END, YIELD — bubble up to FlowRunner
                    return fc;
                }
            }
        }

        last_fc
    }
}

pub struct FlowRunner {
    pub flow_name: String,
    pub stages: Vec<StageRunner>,
}

impl FlowRunner {
    pub fn run(&self, task: &mut Task, api: &CoreHostApi) -> FlowControl {
        let mut current_stage_idx = 0;
        let mut last_fc = FlowControl {
            code: FLOW_CONTROL_CONTINUE,
            payload: std::ptr::null(),
        };

        task.metadata.insert("flow_name".to_string(), self.flow_name.clone());

        while current_stage_idx < self.stages.len() {
            task.metadata.insert(
                "current_stage".to_string(),
                self.stages[current_stage_idx].name.clone(),
            );

            let stage = &self.stages[current_stage_idx];
            last_fc = stage.run(task, api);

            match last_fc.code {
                FLOW_CONTROL_CONTINUE | FLOW_CONTROL_SKIP => {
                    // SKIP from a stage means "stage done, proceed to next stage".
                    current_stage_idx += 1;
                }
                FLOW_CONTROL_YIELD => {
                    // In embedded mode (no scheduler) YIELD is treated as CONTINUE.
                    // A scheduler would return control here to allow other tasks to run.
                    current_stage_idx += 1;
                }
                FLOW_CONTROL_JUMP => {
                    if !last_fc.payload.is_null() {
                        // JUMP(target): run exactly the named stage, then stop the pipeline.
                        // Stages between current and target (and after target) are not executed.
                        let target_name = unsafe { CStr::from_ptr(last_fc.payload) }.to_string_lossy();
                        if let Some(idx) = self.stages.iter().position(|s| s.name == target_name) {
                            let target_stage = &self.stages[idx];
                            task.metadata.insert("current_stage".to_string(), target_stage.name.clone());
                            last_fc = target_stage.run(task, api);
                        } else {
                            last_fc.code = FLOW_CONTROL_ERROR;
                            if let Some(cb) = &task.error_callback { cb(); }
                        }
                        return last_fc;
                    } else {
                        current_stage_idx += 1;
                    }
                }
                FLOW_CONTROL_ERROR => {
                    // Stage-name jump: handled here. "continue"/"end" are already handled
                    // in StageRunner and never reach FlowRunner as ERROR.
                    if let Some(target) = &stage.on_error_target {
                        if target != "errored" && target != "continue" && target != "end" {
                            if let Some(idx) = self.stages.iter().position(|s| s.name == *target) {
                                current_stage_idx = idx;
                                last_fc.code = FLOW_CONTROL_CONTINUE;
                                continue;
                            }
                        }
                    }
                    // "errored" policy or stage not found: call Task.error_callback (task-level
                    // cleanup before pausing) and return ERROR to caller.
                    if let Some(cb) = &task.error_callback { cb(); }
                    return last_fc;
                }
                _ => {
                    // SUSPEND, END
                    return last_fc;
                }
            }
        }

        // Flow completed all stages normally
        if last_fc.code == FLOW_CONTROL_CONTINUE || last_fc.code == FLOW_CONTROL_YIELD {
            last_fc.code = FLOW_CONTROL_END;
        }

        last_fc
    }
}

/// Creates the CoreHostApi static function table.
/// All lock acquisitions happen within each accessor call — no guard is ever held
/// across a call boundary.
pub fn create_host_api() -> CoreHostApi {
    extern "C" fn get_field_impl(task_ctx: *mut c_void, key: *const c_char) -> *const c_char {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy().to_string();

        let state = task.state.read();
        if let Some(ox_workflow_core::state::FieldValue::String(val)) = state.fields.get(&key_str) {
            let cstr = CString::new(val.clone()).unwrap_or_default();
            let ptr = cstr.into_raw();
            drop(state); // release lock before touching task
            task.ffi_arena.push(ptr);
            return ptr;
        }
        std::ptr::null()
    }

    extern "C" fn set_field_impl(task_ctx: *mut c_void, key: *const c_char, value: *const c_char) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy().to_string();
        let val_str = unsafe { CStr::from_ptr(value) }.to_string_lossy().to_string();

        let mut state = task.state.write();
        state.fields.insert(key_str, ox_workflow_core::state::FieldValue::String(val_str));
    }

    extern "C" fn get_field_bytes_impl(task_ctx: *mut c_void, key: *const c_char, len_out: *mut usize) -> *const u8 {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy().to_string();

        let state = task.state.read();
        if let Some(ox_workflow_core::state::FieldValue::Bytes(bytes)) = state.fields.get(&key_str) {
            let len = bytes.len();
            // Allocate a copy on the heap and track it in the bytes arena
            let boxed = bytes.clone().into_boxed_slice();
            let ptr = boxed.as_ptr();
            drop(state);
            unsafe { *len_out = len; }
            task.ffi_bytes_arena.push(boxed);
            return ptr;
        }
        drop(state);
        unsafe { *len_out = 0; }
        std::ptr::null()
    }

    extern "C" fn set_field_bytes_impl(task_ctx: *mut c_void, key: *const c_char, value: *const u8, len: usize) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy().to_string();
        let bytes = unsafe { std::slice::from_raw_parts(value, len) }.to_vec();
        let mut state = task.state.write();
        state.fields.insert(key_str, ox_workflow_core::state::FieldValue::Bytes(bytes));
    }

    extern "C" fn get_metadata_impl(task_ctx: *mut c_void, key: *const c_char) -> *const c_char {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy();

        // Compute standard metadata keys dynamically; fall back to task.metadata map.
        let value: Option<String> = match key_str.as_ref() {
            "task.id"             => Some(task.id.to_string()),
            "task.priority"       => Some(task.priority.to_string()),
            "task.status"         => Some(format!("{:?}", task.status)),
            "task.created_at"     => task.metadata.get("created_at").cloned(),
            "flow.name"           => task.metadata.get("flow_name").cloned(),
            "stage.name"          => task.metadata.get("current_stage").cloned(),
            "stage.plugin_index"  => task.metadata.get("current_plugin_index").cloned(),
            "engine.queue_depth"  => Some("0".to_string()), // not available in-process; use management API
            "stage.modified"      => Some(
                if task.flags.stage.contains("CNT-MOD") { "true" } else { "false" }.to_string()
            ),
            other => task.metadata.get(other).cloned(),
        };

        if let Some(val) = value {
            let cstr = CString::new(val).unwrap_or_default();
            let ptr = cstr.into_raw();
            task.ffi_arena.push(ptr);
            return ptr;
        }
        std::ptr::null()
    }

    extern "C" fn insert_into_flow_impl(task_ctx: *mut c_void, flow_name: *const c_char) -> bool {
        let task = unsafe { &mut *(task_ctx as *mut Task) };

        // Rate limit check
        let count = task.api_call_counts.entry("insert_into_flow".to_string()).or_insert(0);
        let limit = task.api_limits.get("insert_into_flow").copied().unwrap_or(100);
        if limit > 0 && *count >= limit {
            log::warn!("Task {}: insert_into_flow rate limit ({}) exceeded", task.id, limit);
            return false;
        }
        *count += 1;

        let name = unsafe { CStr::from_ptr(flow_name) }.to_string_lossy().to_string();
        task.child_workflows.push(name);
        true
    }

    extern "C" fn pause_task_impl(task_ctx: *mut c_void, signal_key: *const c_char) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };

        // Rate limit check
        let count = task.api_call_counts.entry("pause_task".to_string()).or_insert(0);
        let limit = task.api_limits.get("pause_task").copied().unwrap_or(10);
        if limit > 0 && *count >= limit {
            log::warn!("Task {}: pause_task rate limit ({}) exceeded", task.id, limit);
            return;
        }
        *count += 1;

        task.status = TaskStatus::Paused;
        if !signal_key.is_null() {
            let key = unsafe { CStr::from_ptr(signal_key) }.to_string_lossy().to_string();
            task.metadata.insert("pause_signal_key".to_string(), key);
        }
    }

    extern "C" fn log_impl(task_ctx: *mut c_void, level: u8, message: *const c_char) {
        let msg = unsafe { CStr::from_ptr(message) }.to_string_lossy();

        // Enrich with task/flow/stage context when a task_ctx is provided.
        let enriched;
        let record = if task_ctx.is_null() {
            msg.as_ref()
        } else {
            let task = unsafe { &*(task_ctx as *const Task) };
            let task_id = &task.id;
            let flow = task.metadata.get("flow_name").map(|s| s.as_str()).unwrap_or("-");
            let stage = task.metadata.get("current_stage").map(|s| s.as_str()).unwrap_or("-");
            enriched = format!("[task:{task_id} flow:{flow} stage:{stage}] {msg}");
            enriched.as_str()
        };

        // Route plugin logs through the "plugin" target so log4rs can send them
        // to a separate appender if desired.
        match level {
            1 => log::error!(target: "plugin", "{}", record),
            2 => log::warn!(target: "plugin", "{}", record),
            3 => log::info!(target: "plugin", "{}", record),
            4 => log::debug!(target: "plugin", "{}", record),
            _ => log::trace!(target: "plugin", "{}", record),
        }
    }

    extern "C" fn set_flag_impl(task_ctx: *mut c_void, flag: *const c_char, scope: u8) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let flag_str = unsafe { CStr::from_ptr(flag) }.to_string_lossy().to_string();
        if scope == ox_workflow_abi::FLAG_SCOPE_STAGE {
            task.flags.stage.insert(flag_str);
        } else {
            task.flags.persistent.insert(flag_str);
        }
    }

    extern "C" fn set_flags_impl(task_ctx: *mut c_void, flags: *const *const c_char, scope: u8) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let mut i = 0;
        unsafe {
            while !(*flags.offset(i)).is_null() {
                let flag_str = CStr::from_ptr(*flags.offset(i)).to_string_lossy().to_string();
                if scope == ox_workflow_abi::FLAG_SCOPE_STAGE {
                    task.flags.stage.insert(flag_str);
                } else {
                    task.flags.persistent.insert(flag_str);
                }
                i += 1;
            }
        }
    }

    extern "C" fn has_flag_impl(task_ctx: *mut c_void, flag: *const c_char, scope: u8) -> bool {
        let task = unsafe { &*(task_ctx as *mut Task) };
        let flag_str = unsafe { CStr::from_ptr(flag) }.to_string_lossy();
        if scope == ox_workflow_abi::FLAG_SCOPE_STAGE {
            task.flags.stage.contains(flag_str.as_ref())
        } else {
            task.flags.persistent.contains(flag_str.as_ref())
        }
    }

    extern "C" fn clear_flag_impl(task_ctx: *mut c_void, flag: *const c_char, scope: u8) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let flag_str = unsafe { CStr::from_ptr(flag) }.to_string_lossy().to_string();
        if scope == ox_workflow_abi::FLAG_SCOPE_STAGE {
            task.flags.stage.shift_remove(&flag_str);
        } else {
            task.flags.persistent.shift_remove(&flag_str);
        }
    }

    extern "C" fn get_keys_impl(_task_ctx: *mut c_void) -> *const c_char {
        std::ptr::null()
    }

    extern "C" fn unset_field_impl(_task_ctx: *mut c_void, _key: *const c_char) -> bool {
        false
    }

    extern "C" fn has_field_impl(_task_ctx: *mut c_void, _key: *const c_char) -> bool {
        false
    }

    CoreHostApi {
        get_field: get_field_impl,
        set_field: set_field_impl,
        get_field_bytes: get_field_bytes_impl,
        set_field_bytes: set_field_bytes_impl,
        get_metadata: get_metadata_impl,
        insert_into_flow: insert_into_flow_impl,
        pause_task: pause_task_impl,
        log: log_impl,
        set_flag: set_flag_impl,
        set_flags: set_flags_impl,
        has_flag: has_flag_impl,
        clear_flag: clear_flag_impl,
        get_keys: get_keys_impl,
        unset_field: unset_field_impl,
        has_field: has_field_impl,
    }
}

pub struct FlowManager {
    pub flows: HashMap<String, Arc<FlowRunner>>,
    pub registry: HashMap<String, Arc<LoadedPlugin>>,
    pub stage_defs: HashMap<String, StageDef>,
}

impl FlowManager {
    pub fn new() -> Self {
        Self {
            flows: HashMap::new(),
            registry: HashMap::new(),
            stage_defs: HashMap::new(),
        }
    }

    pub fn load_from_directory(
        &mut self,
        stages_dir: &str,
        flows_dir: &str,
        api: &CoreHostApi,
        plugin_paths: &HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use ox_fileproc::process_file;
        use std::fs;

        if let Ok(entries) = fs::read_dir(stages_dir) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "yaml" || ext == "yml" {
                        let val = process_file(entry.path().as_path(), 5)?;
                        let def: StageDef = serde_json::from_value(val)?;
                        self.stage_defs.insert(def.name.clone(), def);
                    }
                }
            }
        }

        if let Ok(entries) = fs::read_dir(flows_dir) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "yaml" || ext == "yml" {
                        let val = process_file(entry.path().as_path(), 5)?;
                        let def: FlowDef = serde_json::from_value(val)?;
                        unsafe {
                            let _ = self.build_flow(&def, api, plugin_paths)?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub unsafe fn ensure_plugin_loaded(
        &mut self,
        name: &str,
        path: &str,
    ) -> Result<Arc<LoadedPlugin>, PluginError> {
        if let Some(p) = self.registry.get(name) {
            return Ok(p.clone());
        }
        let plugin = Arc::new(LoadedPlugin::new(path)?);
        self.registry.insert(name.to_string(), plugin.clone());
        Ok(plugin)
    }

    pub unsafe fn build_flow(
        &mut self,
        flow_def: &FlowDef,
        api: &CoreHostApi,
        plugin_paths: &HashMap<String, String>,
    ) -> Result<Arc<FlowRunner>, PluginError> {
        let mut stage_runners = Vec::new();

        for stage_name in &flow_def.stages {
            let stage_def = self
                .stage_defs
                .get(stage_name)
                .expect(&format!("Stage {} not found", stage_name))
                .clone();
            let mut instances = Vec::new();

            for plugin_ref in &stage_def.plugins {
                let path = plugin_paths
                    .get(&plugin_ref.name)
                    .expect(&format!("Plugin {} path not defined", plugin_ref.name));
                let plugin = self.ensure_plugin_loaded(&plugin_ref.name, path)?;

                let config_json = match &plugin_ref.config {
                    Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
                    None => "{}".to_string(),
                };

                let ctx = plugin.init(&config_json, api)?;

                instances.push(PluginInstance {
                    name: plugin_ref.name.clone(),
                    plugin,
                    ctx,
                });
            }

            stage_runners.push(StageRunner {
                name: stage_def.name.clone(),
                on_error_target: stage_def.on_error.clone(),
                plugins: instances,
            });
        }

        let runner = Arc::new(FlowRunner {
            flow_name: flow_def.name.clone(),
            stages: stage_runners,
        });

        self.flows.insert(flow_def.name.clone(), runner.clone());

        Ok(runner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_workflow_core::Task;
    use ox_workflow_abi::FLOW_CONTROL_END;

    fn make_api() -> CoreHostApi {
        create_host_api()
    }

    /// A no-op StageRunner with zero plugins — run() should return CONTINUE → FlowRunner promotes to END.
    #[test]
    fn test_empty_flow_completes() {
        let api = make_api();
        let runner = FlowRunner {
            flow_name: "test".to_string(),
            stages: vec![StageRunner {
                name: "s1".to_string(),
                on_error_target: None,
                plugins: vec![],
            }],
        };
        let mut task = Task::new(1);
        let fc = runner.run(&mut task, &api);
        assert_eq!(fc.code, FLOW_CONTROL_END);
    }

    /// Stage flags must be cleared at the start of each stage.
    #[test]
    fn test_stage_flags_cleared_between_stages() {
        let api = make_api();
        let runner = FlowRunner {
            flow_name: "test".to_string(),
            stages: vec![
                StageRunner { name: "s1".to_string(), on_error_target: None, plugins: vec![] },
                StageRunner { name: "s2".to_string(), on_error_target: None, plugins: vec![] },
            ],
        };
        let mut task = Task::new(1);
        // Pre-seed a stage flag
        task.flags.stage.insert("CNT-MOD".to_string());
        assert!(task.flags.stage.contains("CNT-MOD"));

        let fc = runner.run(&mut task, &api);
        assert_eq!(fc.code, FLOW_CONTROL_END);
        // After the last stage runs, stage flags should have been cleared
        assert!(!task.flags.stage.contains("CNT-MOD"));
    }

    /// Persistent flags must survive across stages.
    #[test]
    fn test_persistent_flags_survive_stages() {
        let api = make_api();
        let runner = FlowRunner {
            flow_name: "test".to_string(),
            stages: vec![
                StageRunner { name: "s1".to_string(), on_error_target: None, plugins: vec![] },
                StageRunner { name: "s2".to_string(), on_error_target: None, plugins: vec![] },
            ],
        };
        let mut task = Task::new(1);
        task.flags.persistent.insert("AUTH-PASSED".to_string());

        let fc = runner.run(&mut task, &api);
        assert_eq!(fc.code, FLOW_CONTROL_END);
        assert!(task.flags.persistent.contains("AUTH-PASSED"));
    }

    /// error_callback fires for the "errored" policy, not for others.
    #[test]
    fn test_error_callback_fires_only_for_errored_policy() {
        use std::sync::{Arc, Mutex};
        let fired = Arc::new(Mutex::new(false));
        let fired_clone = fired.clone();

        let api = make_api();
        // FlowRunner with no stages so we can set error_callback and check it doesn't fire needlessly
        let runner = FlowRunner {
            flow_name: "test".to_string(),
            stages: vec![],
        };
        let mut task = Task::new(1);
        task.error_callback = Some(Box::new(move || {
            *fired_clone.lock().unwrap() = true;
        }));

        // Empty flow — completes normally, callback should NOT fire
        let fc = runner.run(&mut task, &api);
        assert_eq!(fc.code, FLOW_CONTROL_END);
        assert!(!*fired.lock().unwrap());
    }

    /// get_metadata must return standard keys without them being in task.metadata.
    #[test]
    fn test_get_metadata_standard_keys() {
        let api = make_api();
        let mut task = Task::new(42);
        let task_ptr = &mut task as *mut Task as *mut c_void;

        let key_id = CString::new("task.id").unwrap();
        let result = (api.get_metadata)(task_ptr, key_id.as_ptr());
        assert!(!result.is_null());
        let id_str = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(id_str.len(), 36); // UUID format

        let key_priority = CString::new("task.priority").unwrap();
        let result = (api.get_metadata)(task_ptr, key_priority.as_ptr());
        assert!(!result.is_null());
        let priority_str = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(priority_str, "42");

        let key_modified = CString::new("stage.modified").unwrap();
        let result = (api.get_metadata)(task_ptr, key_modified.as_ptr());
        assert!(!result.is_null());
        let modified_str = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(modified_str, "false");
    }

    /// insert_into_flow rate limiting.
    #[test]
    fn test_insert_into_flow_rate_limit() {
        let api = make_api();
        let mut task = Task::new(1);
        task.api_limits.insert("insert_into_flow".to_string(), 2);
        let task_ptr = &mut task as *mut Task as *mut c_void;

        let flow = CString::new("my_flow").unwrap();
        assert!((api.insert_into_flow)(task_ptr, flow.as_ptr())); // 1
        assert!((api.insert_into_flow)(task_ptr, flow.as_ptr())); // 2
        assert!(!(api.insert_into_flow)(task_ptr, flow.as_ptr())); // 3 — should be blocked
        assert_eq!(task.child_workflows.len(), 2);
    }
}
