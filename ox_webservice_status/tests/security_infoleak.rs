use ox_webservice_status::OxModule;
use ox_webservice_api::{WebServiceApiV1, PipelineState};
use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state};
use lazy_static::lazy_static;
use serde_json::Value;

lazy_static! {
    static ref API: WebServiceApiV1 = create_mock_api();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_info_leak() {
    let module = OxModule::new(&API, None);
    let mut ps = create_stub_pipeline_state();
    
    // Assuming process_request fills response_body with JSON status
    let _ = module.process_request(&mut ps as *mut _);
    
    if !ps.response_body.is_empty() {
        let json: Value = serde_json::from_slice(&ps.response_body).unwrap();
        
        // Check for sensitive keys
        if let Some(env) = json.get("environment") {
             // Ensure no "AWS_SECRET" or "KEY" in environment dump if it exists
             let env_str = env.to_string();
             assert!(!env_str.contains("SECRET"), "Status module leaked SECRET env var");
             assert!(!env_str.contains("KEY"), "Status module leaked KEY env var");
        }
    }
}
