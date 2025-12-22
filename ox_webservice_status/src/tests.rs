
use crate::initialize_module;
use ox_webservice_api::{
    PipelineState, HandlerResult, WebServiceApiV1, ModuleInterface, ModuleStatus, FlowControl, ReturnParameters,
};
use ox_webservice_test_utils::{create_mock_api, ModuleLoader};

#[test]
fn test_status_html_stream() {
    let api = create_mock_api();
    let params_json = "{}";
    let module_id = "test_status";

    let loader = ModuleLoader::load(initialize_module, params_json, module_id, &api).expect("Failed to load status module");
    let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
    
    // Default request (HTML)
    let result = loader.process_request(&mut ps, api.log_callback, api.alloc_raw);

    assert_eq!(result.status, ModuleStatus::Modified);
    assert_eq!(result.flow_control, FlowControl::StreamFile);
    
    // Check return_data points to index.html
    assert!(!result.return_parameters.return_data.is_null());
    let path_cstr = unsafe { std::ffi::CStr::from_ptr(result.return_parameters.return_data as *const i8) };
    let path = path_cstr.to_str().unwrap();
    assert!(path.ends_with("index.html"));
}

#[test]
fn test_status_static_assets() {
    let api = create_mock_api();
    let params_json = "{}";
    let module_id = "test_status";

    let loader = ModuleLoader::load(initialize_module, params_json, module_id, &api).expect("Failed to load status module");
    
    // CSS
    let mut ps_css = ox_webservice_test_utils::create_stub_pipeline_state();
    ps_css.request_path = "/status/css/status.css".to_string();
    let result_css = loader.process_request(&mut ps_css, api.log_callback, api.alloc_raw);
    assert_eq!(result_css.flow_control, FlowControl::StreamFile);
    let path_cstr = unsafe { std::ffi::CStr::from_ptr(result_css.return_parameters.return_data as *const i8) };
    let path = path_cstr.to_str().unwrap();
    assert!(path.ends_with("status.css"), "Expected path ending in status.css, got {}", path);
    assert_eq!(ps_css.response_headers.get("Content-Type").unwrap(), "text/css");

    // JS
    let mut ps_js = ox_webservice_test_utils::create_stub_pipeline_state();
    ps_js.request_path = "/status/js/status.js".to_string();
    let result_js = loader.process_request(&mut ps_js, api.log_callback, api.alloc_raw);
    assert_eq!(result_js.flow_control, FlowControl::StreamFile);
    let path_cstr_js = unsafe { std::ffi::CStr::from_ptr(result_js.return_parameters.return_data as *const i8) };
    assert!(path_cstr_js.to_str().unwrap().ends_with("status.js"));
    assert_eq!(ps_js.response_headers.get("Content-Type").unwrap(), "application/javascript");
}

#[test]
fn test_status_json_accept() {
    let api = create_mock_api();
    let params_json = "{}";
    let module_id = "test_status";

    let loader = ModuleLoader::load(initialize_module, params_json, module_id, &api).expect("Failed to load status module");
    let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
    
    // Set Accept header
    ps.request_headers.insert("Accept", "application/json".parse().unwrap());
    
    let result = loader.process_request(&mut ps, api.log_callback, api.alloc_raw);

    assert_eq!(result.status, ModuleStatus::Modified);
    assert_eq!(result.flow_control, FlowControl::Continue);

    let body_str = String::from_utf8_lossy(&ps.response_body);
    assert!(body_str.starts_with("{"));
    assert!(body_str.contains("\"uptime\":"));
    
    // Check Content-Type header
    assert_eq!(ps.response_headers.get("Content-Type").unwrap(), "application/json");
}

#[test]
fn test_status_json_query() {
    let api = create_mock_api();
    let params_json = "{}";
    let module_id = "test_status";

    let loader = ModuleLoader::load(initialize_module, params_json, module_id, &api).expect("Failed to load status module");
    let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
    
    // Set Query
    ps.request_query = "format=json".to_string();
    
    let result = loader.process_request(&mut ps, api.log_callback, api.alloc_raw);

    assert_eq!(result.status, ModuleStatus::Modified);
    assert_eq!(result.flow_control, FlowControl::Continue);

    let body_str = String::from_utf8_lossy(&ps.response_body);
    assert!(body_str.starts_with("{"));
}
