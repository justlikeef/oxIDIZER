use std::ffi::{c_char, CStr, CString};
use std::sync::{Arc, Mutex};
use ox_webservice_api::{
    CoreHostApi, ModuleInterface, ModuleStatus, FlowControl, ReturnParameters, 
    HandlerResult, PipelineState
};
use ox_forms::{
    registry::TypeRegistry,
    manager::PluginManager,
    render::FormEngine,
    schema::{FormDefinition, FieldDefinition},
    traits::RenderContext,
};

const MODULE_NAME: &str = "ox_forms_server";

pub struct OxModule {
    api: &'static CoreHostApi,
    module_id: String,
    registry: Arc<Mutex<TypeRegistry>>,
    // In a real app, we'd persist the Manager to keep plugins loaded
    plugin_manager: Arc<Mutex<PluginManager>>, 
}

impl OxModule {
    pub fn new(api: &'static CoreHostApi, module_id: String) -> Self {
        let registry = Arc::new(Mutex::new(TypeRegistry::new()));
        let mut manager = PluginManager::new(registry.clone());
        
        // Try to load standard renderers. In production, this path should be configurable.
        // Assuming running from target/debug for now
        let dylib_name = if cfg!(target_os = "linux") {
            "libox_forms_std_renderers.so"
        } else {
            "libox_forms_std_renderers.dylib"  
        };
        // Path logic needs to be robust. For now, try hardcoded relative path from execution dir?
        // Or assume they are in the same dir as the main binary.
        let lib_path = std::path::Path::new(".").join(dylib_name);
        
        if let Err(e) = manager.load_plugin(&lib_path) {
             // Use api.log_callback here?
             // eprintln!("Failed to load std renderers: {}", e);
        }

        Self {
            api,
            module_id,
            registry,
            plugin_manager: Arc::new(Mutex::new(manager)),
        }
    }


    pub fn process(&self, pipeline_state: &mut PipelineState) -> HandlerResult {
        // Initialize Context wrapper
        let arena_ptr = &pipeline_state.arena as *const bumpalo::Bump as *const std::ffi::c_void;
        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state as *mut PipelineState as *mut std::ffi::c_void, 
            arena_ptr
        ) };

        // Check Verb (Generic)
        let verb_json = ctx.get("request.verb");
        let verb = verb_json.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_else(|| pipeline_state.request_method.to_lowercase());

        // Only handle GET (read) requests for now
        if verb != "get" && verb != "read" {
             return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue, 
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
             };
        }

        // Hardcoded form for demo
        let form = FormDefinition {
            id: "server_test_form".to_string(),
            title: "Server Side Form".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "full_name".to_string(),
                    label: "Full Name".to_string(),
                    data_type: "string".to_string(),
                    component: None, 
                    plugins: vec![],
                    validation: vec![],
                    dependencies: vec![],
                    props: serde_json::Value::Null,
                },
                FieldDefinition {
                    name: "quantity".to_string(),
                    label: "Quantity".to_string(),
                    data_type: "integer".to_string(),
                    component: Some("number-input".to_string()),
                    plugins: vec![],
                    validation: vec![],
                    dependencies: vec![],
                    props: serde_json::Value::Null,
                },
            ],
            layout: None,
            actions: vec![],
            data_source_binding: None,
        };

        let registry = self.registry.lock().unwrap();
        let engine = FormEngine::new(&registry);
        // Context for rendering
        let render_ctx = RenderContext {
            props: &std::collections::HashMap::new(),
        };

        match engine.render(&form, &render_ctx) {
            Ok(html) => {
                // Populate Generic Response
                let _ = ctx.set("response.body", serde_json::Value::String(html.clone()));
                let _ = ctx.set("response.type", serde_json::Value::String("text/html".to_string()));
                let _ = ctx.set("response.status", serde_json::json!(200));

            },
            Err(e) => {
                let err_msg = format!("Render Error: {}", e);
                let _ = ctx.set("response.status", serde_json::json!(500));
                let _ = ctx.set("response.body", serde_json::Value::String(err_msg.clone()));
                
            }
        }

        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Halt, 
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        }
    }
}

// C-compatible handler function
unsafe extern "C" fn ox_forms_server_handler(
    instance_ptr: *mut std::ffi::c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log_callback: ox_webservice_api::LogCallback,
    _alloc_fn: ox_webservice_api::AllocFn,
    _arena: *const std::ffi::c_void,
) -> HandlerResult {
    let module = &*(instance_ptr as *mut OxModule);
    let state = &mut *pipeline_state_ptr; // Still pass state ptr, but logic now uses Context wrapper
    module.process(state)
}

extern "C" fn get_config(
    _state: *mut std::ffi::c_void,
    _arena: *const std::ffi::c_void,
    _alloc_fn: ox_webservice_api::AllocStrFn,
) -> *mut std::ffi::c_char {
    std::ptr::null_mut() 
}

#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    let api = &*api_ptr;
    let module_id = if !module_id_ptr.is_null() {
        CStr::from_ptr(module_id_ptr).to_string_lossy().to_string()
    } else {
        MODULE_NAME.to_string()
    };

    let module = OxModule::new(api, module_id);
    let instance_ptr = Box::into_raw(Box::new(module)) as *mut std::ffi::c_void;

    // Return the struct, allocated on heap to pass ownership? 
    // Usually ModuleInterface is returned as a pointer.
    let interface = Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: ox_forms_server_handler,
        log_callback: api.log_callback,
        get_config,
    });

    Box::into_raw(interface)
}
