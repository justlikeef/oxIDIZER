use ox_data_object_manager::{
    DataDictionary, DataObjectDefinition, DataStoreContainer
};
use ox_persistence_dictionary_manager::get_dictionary_config;
use std::collections::HashMap;
use anyhow::Result;
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE};

pub struct BootstrapService;

impl BootstrapService {
    /// bootstrapping the dictionary from the configured datastore.
    pub fn load_dictionary() -> Result<DataDictionary> {
         let config = get_dictionary_config()
            .ok_or_else(|| anyhow::anyhow!("Dictionary configuration not set"))?;

        let _meta_container = DataStoreContainer {
            id: "ox_definitions".to_string(),
            datasource_id: "dictionary_source".to_string(),
            name: config.parameters.get("container_name").cloned().unwrap_or("ox_definitions".to_string()),
            container_type: "table".to_string(),
            fields: vec![],
            metadata: HashMap::new(),
        };

        let driver_name = &config.driver;
        let driver_registry = ox_persistence::PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        let (driver, _) = driver_registry.get_driver(driver_name)
            .ok_or_else(|| anyhow::anyhow!("Driver '{}' not found", driver_name))?;
            
        let location = config.parameters.get("location")
            .ok_or_else(|| anyhow::anyhow!("'location' parameter required"))?;

        // RAW FETCH
        let ids = driver.fetch(&HashMap::new(), location).map_err(|e| anyhow::anyhow!("{}", e))?;
        
        let mut dictionary = DataDictionary::new();
        
        for id in ids {
             let record_map = driver.restore(location, &id).map_err(|e| anyhow::anyhow!("{}", e))?;
             
             if let Some((json_str, _, _)) = record_map.get("definition_json") {
                 let def: DataObjectDefinition = serde_json::from_str(json_str)?;
                 dictionary.add_object(def);
             } else {
                 return Err(anyhow::anyhow!("Dictionary store must provide 'definition_json' field"));
             }
        }
        
        Ok(dictionary)
    }
}

// --- Plugin Interface Implementation ---

#[allow(dead_code)]
fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

#[allow(dead_code)]
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

struct ModuleContext {
    #[allow(dead_code)]
    api: CoreHostApi,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    _plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };
    let ctx = Box::new(ModuleContext { api });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) -> FlowControl {
    // Placeholder: bootstrap logic not yet implemented
    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = unsafe { Box::from_raw(plugin_config_ctx as *mut ModuleContext) };
    }
}
