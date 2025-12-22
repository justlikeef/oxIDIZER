use super::*;
use ox_webservice_api::{PipelineState, ModuleStatus, FlowControl};
use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state};

use lazy_static::lazy_static;

lazy_static! {
    static ref API: WebServiceApiV1 = create_mock_api();
}

#[test]
fn test_ping_basic() {
    let module = OxModule::new(&API);
    
    let mut ps = create_stub_pipeline_state();

    let result = module.process_request(&mut ps as *mut _);

    // Should modify state (return pong)
    assert_eq!(result.status, ModuleStatus::Modified);
    assert_eq!(result.flow_control, FlowControl::Continue);
    
    // Verify response
    assert_eq!(ps.status_code, 200);
}
