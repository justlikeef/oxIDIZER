use std::ffi::{c_char, CStr, CString};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use libc::c_void;
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END,
    OX_LOG_ERROR,
};
use ox_forms::{
    registry::TypeRegistry,
    manager::PluginManager,
    render::FormEngine,
    schema::FormDefinition,
    traits::RenderContext,
};

const MODULE_NAME: &str = "ox_forms_server";

#[derive(serde::Deserialize, prost::Message)]
pub struct ModuleConfig {
    #[prost(string, optional, tag = "1")]
    pub forms_file: Option<String>,
}

pub struct OxModule {
    api: CoreHostApi,
    registry: Arc<Mutex<TypeRegistry>>,
    #[allow(dead_code)]
    plugin_manager: Arc<Mutex<PluginManager>>,
    forms: HashMap<String, FormDefinition>,
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let p = (api.get_field)(task_ctx, c_key.as_ptr());
    if p.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

#[allow(dead_code)]
fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 { return None; }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

#[allow(dead_code)]
fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
}

fn log_msg(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

impl OxModule {
    pub fn new(api: CoreHostApi, config: ModuleConfig) -> Self {
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

        if let Err(e) = manager.load_plugin(path_buf.as_path()) {
            eprintln!("Failed to load std renderers from {:?}: {}", path_buf, e);
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
            registry,
            plugin_manager: Arc::new(Mutex::new(manager)),
            forms,
        }
    }

    pub fn process(&self, task_ctx: *mut c_void) -> FlowControl {
        let api = &self.api;

        let verb = get_field(api, task_ctx, "request.verb");
        let verb = if !verb.is_empty() {
            verb
        } else {
            get_field(api, task_ctx, "request.method").to_lowercase()
        };

        if verb != "get" && verb != "read" {
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        let form_opt = self.forms.get("server_test_form")
            .or_else(|| self.forms.values().next());

        if let Some(form) = form_opt {
            let registry = self.registry.lock().unwrap();
            let engine = FormEngine::new(&registry);
            let render_ctx = RenderContext {
                props: &std::collections::HashMap::new(),
            };

            match engine.render(form, &render_ctx) {
                Ok(html) => {
                    set_field(api, task_ctx, "response.body", &html);
                    set_field(api, task_ctx, "response.header.Content-Type", "text/html");
                    set_field(api, task_ctx, "response.status", "200");
                },
                Err(e) => {
                    let err_msg = format!("Render Error: {}", e);
                    log_msg(api, task_ctx, OX_LOG_ERROR, &err_msg);
                    set_field(api, task_ctx, "response.status", "500");
                    set_field(api, task_ctx, "response.body", &err_msg);
                }
            }
        } else {
            set_field(api, task_ctx, "response.status", "404");
            set_field(api, task_ctx, "response.body", "Form not found");
        }

        FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let mut config = ModuleConfig { forms_file: None };
    if !plugin_config_ctx.is_null() {
        let params_json = unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy() };
        if let Ok(cfg) = serde_json::from_str::<ModuleConfig>(&params_json) {
            config = cfg;
        } else {
            eprintln!("[{}] Failed to parse module params: {}", MODULE_NAME, params_json);
        }
    }

    let module = OxModule::new(api, config);
    Box::into_raw(Box::new(module)) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let module = unsafe { &*(plugin_config_ctx as *mut OxModule) };
    module.process(task_ctx)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut OxModule);
    }
}
