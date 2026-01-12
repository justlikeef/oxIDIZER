use std::ffi::{c_char, CStr};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use ox_webservice_api::{
    CoreHostApi, ModuleInterface, ModuleStatus, FlowControl, ReturnParameters, 
    HandlerResult, PipelineState
};
use ox_forms::{
    registry::TypeRegistry,
    manager::PluginManager,
    render::FormEngine,
    schema::FormDefinition,
    traits::RenderContext,
};

const MODULE_NAME: &str = "ox_forms_server";

#[derive(serde::Deserialize)]
pub struct ModuleConfig {
    pub forms_file: Option<String>,
}

pub struct OxModule {
    api: &'static CoreHostApi,
    module_id: String,
    registry: Arc<Mutex<TypeRegistry>>,
    // In a real app, we'd persist the Manager to keep plugins loaded
    plugin_manager: Arc<Mutex<PluginManager>>, 
    forms: HashMap<String, FormDefinition>,
}

impl OxModule {
    pub fn new(api: &'static CoreHostApi, module_id: String, config: ModuleConfig) -> Self {
        let mut registry = TypeRegistry::new();
        registry.load_from_config(ox_forms::registry::TypeMappingConfig {
            mappings: vec![
                ("integer".to_string(), ox_forms::registry::DefaultFieldConfig { component: "number-input".to_string(), default_props: serde_json::Value::Null }),
                ("float".to_string(), ox_forms::registry::DefaultFieldConfig { component: "number-input".to_string(), default_props: serde_json::Value::Null }),
                ("string".to_string(), ox_forms::registry::DefaultFieldConfig { component: "text-input".to_string(), default_props: serde_json::Value::Null }),
                ("password".to_string(), ox_forms::registry::DefaultFieldConfig { component: "password-input".to_string(), default_props: serde_json::Value::Null }),
                ("boolean".to_string(), ox_forms::registry::DefaultFieldConfig { component: "checkbox".to_string(), default_props: serde_json::Value::Null }),
                ("date".to_string(), ox_forms::registry::DefaultFieldConfig { component: "date-input".to_string(), default_props: serde_json::Value::Null }),
                ("select".to_string(), ox_forms::registry::DefaultFieldConfig { component: "select-input".to_string(), default_props: serde_json::Value::Null }),
            ].into_iter().collect()
        });
        let registry = Arc::new(Mutex::new(registry));
        let mut manager = PluginManager::new(registry.clone());
        
        let dylib_name = if cfg!(target_os = "linux") {
            "libox_forms_std_renderers.so"
        } else {
            "libox_forms_std_renderers.dylib"  
        };
        
        let path_buf = std::env::current_exe()
            .ok()
            .and_then(|pb| pb.parent().map(|p| p.join(dylib_name)))
            .unwrap_or_else(|| std::path::Path::new(".").join(dylib_name));
        
        let lib_path = path_buf.as_path();
        
        if let Err(e) = manager.load_plugin(lib_path) {
             // Use api.log_callback here?
             eprintln!("Failed to load std renderers from {:?}: {}", lib_path, e);
        }

        let mut forms = HashMap::new();
        if let Some(path) = config.forms_file {
            let path_buf = std::path::PathBuf::from(path.clone());
            match ox_fileproc::process_file(&path_buf, 5) {
                Ok(value) => {
                    if let Ok(loaded_forms) = serde_json::from_value::<Vec<FormDefinition>>(value) {
                        for form in loaded_forms {
                            forms.insert(form.id.clone(), form);
                        }
                    } else {
                        // Log error?
                         eprintln!("Failed to parse forms file: {}", path);
                    }
                },
                Err(e) => {
                    eprintln!("Failed to process forms file {}: {}", path, e);
                }
            }
        }

        Self {
            api,
            module_id,
            registry,
            plugin_manager: Arc::new(Mutex::new(manager)),
            forms,
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

        // Look up form (simple logic: get first one, or specific one by ID if parameterized)
        // For now, let's just use "server_test_form" if it exists, or the first one.
        
        let form_opt = self.forms.get("server_test_form")
            .or_else(|| self.forms.values().next());

        if let Some(form) = form_opt {
            let registry = self.registry.lock().unwrap();
            let engine = FormEngine::new(&registry);
            // Context for rendering
            let render_ctx = RenderContext {
                props: &std::collections::HashMap::new(),
            };
    
            match engine.render(form, &render_ctx) {
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
        } else {
             let _ = ctx.set("response.status", serde_json::json!(404));
             let _ = ctx.set("response.body", serde_json::Value::String("Form not found".to_string()));
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

    let mut config = ModuleConfig { forms_file: None };
    if !module_params_json_ptr.is_null() {
        let params_json = CStr::from_ptr(module_params_json_ptr).to_string_lossy();
        if let Ok(cfg) = serde_json::from_str::<ModuleConfig>(&params_json) {
            config = cfg;
        } else {
            // Log error?
             eprintln!("Failed to parse module params: {}", params_json);
        }
    }

    let module = OxModule::new(api, module_id, config);
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
