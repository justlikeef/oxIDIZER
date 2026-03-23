#[cfg(test)]
mod tests {
    use crate::initialize_module;
    use ox_webservice_api::{HandlerResult, PipelineState, ModuleStatus, FlowControl, ReturnParameters};
    use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state, mock_alloc_raw, mock_log, ModuleLoader};
    use std::io::Write;
    use tempfile::NamedTempFile;
    
    #[test]
    fn test_stream_basic_flow_with_hacker_checks() {
        // Setup temporary config
        let mut mimetypes_file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(mimetypes_file, "mimetypes:\n  - extension: txt\n    mimetype: text/plain\n    handler: stream\n    url: \".*\\\\.txt$\"").unwrap();
        
        let mut content_file = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
        write!(content_file, "Safe Content").unwrap();

        let mut config_file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        let config_content = format!(
            "content_root: \"{}\"\nmimetypes_file: \"{}\"\ndefault_documents: []",
            content_file.path().parent().unwrap().to_str().unwrap(),
            mimetypes_file.path().to_str().unwrap()
        );
        writeln!(config_file, "{}", config_content).unwrap();

        // Initialize Module using Loader
        let api = create_mock_api();
        let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
        
        let loader = ModuleLoader::load(initialize_module, &params_json, "stream_module", &api).expect("Module load failed");

        // 1. Happy Path
        let mut ps = create_stub_pipeline_state();
        ps.request_path = format!("/{}", content_file.path().file_name().unwrap().to_str().unwrap());
        
        // This validates "set_response_body" and "set_response_header" bounds implicitly via the Mock's slice construction
        // This validates "set_response_body" and "set_response_header" bounds implicitly via the Mock's slice construction
        let result = loader.process_request(&mut ps, mock_log, mock_alloc_raw);
        
        assert_eq!(result.status, ModuleStatus::Modified);
        assert_eq!(result.flow_control, FlowControl::StreamFile);
        assert!(!result.return_parameters.return_data.is_null());
        
        // Verify path string
        let ptr = result.return_parameters.return_data as *mut i8;
        let c_str = unsafe { std::ffi::CStr::from_ptr(ptr) };
        let str_slice = c_str.to_str().unwrap();
        assert_eq!(str_slice, content_file.path().to_str().unwrap());
        
        // Cleanup memory allocated by module
        unsafe { let _ = std::ffi::CString::from_raw(ptr); }

        // Body should NOT be populated in memory
        assert_eq!(ps.response_body, b"");
        
        // 2. Hacker Test: Null/Invalid Pointers (Simulated via Loader checks?) 
        // Note: We can't pass actual garbage pointers to the module without crashing test runner (UB).
        // But ModuleLoader checks for null return from init.
        
        // 3. Hacker Test: Path Traversal
        // Try to access the config file itself via traversal if it's outside content root (it usually is in tmp)
        // Or create a secret file outside content root.
        let mut secret_file = tempfile::NamedTempFile::new().unwrap();
        write!(secret_file, "SECRET").unwrap();
        let secret_path = secret_file.path().to_str().unwrap();

        // Construct a path that tries to traverse out of content root to hit secret file
        // ../../../tmp/secret
        let mut traversing_ps = create_stub_pipeline_state();
        // Just send a blatantly traversing path
        traversing_ps.request_path = "/../../../../../../etc/passwd".to_string(); 
        let t_result = loader.process_request(&mut traversing_ps, mock_log, mock_alloc_raw);
        // Should NOT return ModifiedContinue with body populated (e.g. should 404/Ignore or 403)
        // Since it relies on `resolve_and_find_file` which uses `canonicalize` and `starts_with`, it should return UnmodifiedContinue safely.
        assert_eq!(t_result, HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        }); // It returns ModifiedContinue because it likely falls through? Wait, if file not found, it returns ModifiedContinue.
        // We need to verify BODY is EMPTY or 404 status?
        // Actually `process_request` in this module returns `ModifiedContinue` at end if no file found, without setting body/status (Default 404 handling later).
        assert_eq!(traversing_ps.status_code, 404, "Traversing request should return 404");
        assert_eq!(traversing_ps.response_body, b"404 Not Found");

        // 4. Hacker Test: Large Payload (Buffer Overflow Check)
        // Send a huge request body (though stream handler ignores request body usually)
        // Send a huge URL
        let mut huge_ps = create_stub_pipeline_state();
        huge_ps.request_path = "/".to_string() + &"a".repeat(10000); // 10KB URL
        let h_result = loader.process_request(&mut huge_ps, mock_log, mock_alloc_raw);
        // Should handle gracefully (UnmodifiedContinue presumably)
        assert_eq!(h_result, HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        });

        // 5. Malformed Config
        let bad_json = "{ \"config_file\": \"/nonexistent\" }";
        let bad_loader = ModuleLoader::load(initialize_module, bad_json, "stream_module", &api);
        assert!(bad_loader.is_err(), "Should fail on missing config file");
    }
}
