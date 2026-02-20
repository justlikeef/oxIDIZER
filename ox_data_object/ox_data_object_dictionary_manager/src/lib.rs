use ox_data_object_manager::{
    DataDictionary, DataObjectDefinition, DataStoreContainer, QueryNode
};
use ox_persistence_dictionary_manager::get_dictionary_config;
use std::collections::HashMap;
use anyhow::Result;
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use ox_webservice_api::{
    ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters, CoreHostApi,
};
use lazy_static::lazy_static;

pub struct BootstrapService;

impl BootstrapService {
    /// bootstrapping the dictionary from the configured datastore.
    pub fn load_dictionary() -> Result<DataDictionary> {
         let config = get_dictionary_config()
            .ok_or_else(|| anyhow::anyhow!("Dictionary configuration not set"))?;

        let meta_container = DataStoreContainer {
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
        let ids = driver.fetch(&HashMap::new(), location).map_err(|e: String| anyhow::anyhow!(e))?;
        
        let mut dictionary = DataDictionary::new();
        
        for id in ids {
             let record_map = driver.restore(location, &id).map_err(|e: String| anyhow::anyhow!(e))?;
             
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

// --- Module Interface Implementation ---

struct ModuleContext {
    api: &'static CoreHostApi,
    module_id: String,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    _module_params_json_ptr: *const c_char,
    module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { &*api_ptr };
    
    let module_id_str = if !module_id.is_null() {
        unsafe { CStr::from_ptr(module_id).to_string_lossy().to_string() }
    } else {
        "ox_data_object_dictionary_manager".to_string()
    };
    
    let ctx = Box::new(ModuleContext {
        api,
        module_id: module_id_str,
    });

    let interface = Box::new(ModuleInterface {
        instance_ptr: Box::into_raw(ctx) as *mut c_void,
        handler_fn: process_request,
        log_callback: unsafe { (*api_ptr).log_callback },
        get_config: get_config,
    });

    Box::into_raw(interface)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_request(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    alloc_fn: AllocFn,
    arena: *const c_void,
) -> HandlerResult {
    if instance_ptr.is_null() {
         return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }
    
    // Use PipelineContext helper if available, or manual raw pointer access.
    // For brevity, using raw access or creating context wrapper.
    let _context = unsafe { &*(instance_ptr as *mut ModuleContext) };
    let pipeline_state = unsafe { &mut *pipeline_state_ptr };
    
    // Check path for /bootstrap command
    // "request.path" or capture.
    // Simplifying: If request contains "bootstrap" in payload or query, do it.
    // Or check if capture == "bootstrap"
    
    // Just a placeholder implementation to acknowledge functionality
    // Real implementation would parse path.
    
    HandlerResult {
        status: ModuleStatus::Unmodified,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config(
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let json = "{}".to_string();
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}
