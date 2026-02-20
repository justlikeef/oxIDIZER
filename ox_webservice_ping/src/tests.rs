use super::*;
use ox_webservice_api::{PipelineState, ModuleStatus, FlowControl, CoreHostApi};
use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state};

use lazy_static::lazy_static;

lazy_static! {
    static ref API: CoreHostApi = create_mock_api();
}

#[test]
fn test_ping_basic() {
    let api_ptr: *const _ = &*API;
    let core_api = unsafe { &*(api_ptr as *const ox_webservice_api::CoreHostApi) };
    let module = OxModule::new(core_api, "test_ping".to_string());
    
    let mut ps = create_stub_pipeline_state();

    let result = module.process_request(&mut ps as *mut _);

    // Should modify state (return pong)
    assert_eq!(result.status, ModuleStatus::Modified);
    assert_eq!(result.flow_control, FlowControl::Continue);
    
    // Verify response
    assert_eq!(ps.status_code, 200);
}
