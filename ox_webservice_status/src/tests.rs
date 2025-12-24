
use crate::initialize_module;
use ox_webservice_api::{
    PipelineState, HandlerResult, WebServiceApiV1, ModuleInterface, ModuleStatus, FlowControl, ReturnParameters,
};
use ox_webservice_test_utils::{create_mock_api, ModuleLoader};

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
#[cfg_attr(miri, ignore)]
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
