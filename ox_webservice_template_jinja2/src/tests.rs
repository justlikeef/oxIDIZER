
#[cfg(test)]
mod tests {
    use crate::initialize_module;
    use ox_webservice_api::{HandlerResult, LogLevel};
    use ox_webservice_test_utils::{create_mock_api, ModuleLoader, mock_log, mock_alloc_raw};
    use std::io::Write;
    use tempfile::Builder;

    #[test]
    fn test_template_basic_flow_with_hacker_checks() {
        // --- Setup Configuration ---
        let mut mimetypes_file = Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(
            mimetypes_file,
            "mimetypes:\n  - extension: html\n    mimetype: text/html\n    handler: template"
        )
        .unwrap();

        // Valid template
        let mut template_file = Builder::new().suffix(".html").tempfile().unwrap();
        writeln!(template_file, "Hello {{{{ \"Jinja\" }}}}").unwrap();

        // Config file
        let mut config_file = Builder::new().suffix(".yaml").tempfile().unwrap();
        let config_content = format!(
            "content_root: \"{}\"\nmimetypes_file: \"{}\"\ndefault_documents: []",
            template_file.path().parent().unwrap().to_str().unwrap(),
            mimetypes_file.path().to_str().unwrap()
        );
        writeln!(config_file, "{}", config_content).unwrap();

        // --- Initialize Module ---
        let api = create_mock_api();
        let params_json = format!(
            r#"{{"config_file": "{}"}}"#,
            config_file.path().to_str().unwrap()
        );

        let mut loader = ModuleLoader::load(initialize_module, &params_json, &api).expect("Module load failed");
        assert!(!loader.interface_ptr.is_null(), "Module initialization failed");

        // --- Test 1: Happy Path ---
        {
            let request_path = format!(
                "/{}",
                template_file.path().file_name().unwrap().to_str().unwrap()
            );
            let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
            ps.request_path = request_path;
            ps.request_method = "GET".to_string();

            let result = loader.process_request(&mut ps, mock_log, mock_alloc_raw);

            assert_eq!(result, HandlerResult::ModifiedContinue);
            assert_eq!(ps.response_body, b"Hello Jinja");
            let content_type = ps.response_headers.get("Content-Type").unwrap();
            assert_eq!(content_type, "text/html");
        }

        // --- Test 2: Path Traversal Attempt ---
        {
            // Try to access a file outside content root (e.g. /etc/passwd equivalent)
            // We'll create a file in a sibling directory to mock this safely
            let sibling_dir = tempfile::tempdir().unwrap();
            let secret_file_path = sibling_dir.path().join("passwd");
            std::fs::write(&secret_file_path, "secret_data").unwrap();

            // Construct a relative path: ../sibling_dir/passwd
            let relative_path = format!(
                "/../{}/passwd",
                sibling_dir.path().file_name().unwrap().to_str().unwrap()
            );

            // Note: The module logic trims start matches of '/', so we might need to be careful with how we construct it.
            // But if we pass "/../../foo", it effectively tests traversal from the content root.
            
            // Actually, let's try a direct traversal up from the temp dir
            let traversal_path = "/../../../../../../../../etc/passwd"; 
            
            let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
            ps.request_path = traversal_path.to_string();
            ps.request_method = "GET".to_string();
            let result = loader.process_request(&mut ps, mock_log, mock_alloc_raw);

            // Should NOT serve the file. It might return UnmodifiedContinue (not found/ignored) or ModifiedJumpToError.
            // Based on logic: resolve_and_find_file checks canonical path vs content root.
            assert_eq!(result, HandlerResult::ModifiedContinue); // It returns ModifiedContinue if not found/processed (default fallback in logic is ModifiedContinue at the end of function)
            
            // Wait, looking at the code: 
            // if resolve_and_find_file returns None, it drops to end and returns HandlerResult::ModifiedContinue 
            // BUT response body would be empty/untouched.
            assert!(ps.response_body.is_empty(), "Pathtraversal should not yield content");
        }
        
        // --- Test 3: Null Pipeline Spec check ---
        // The ModuleLoader takes a reference, so we can't easily pass null safe-ly here without unsafe raw pointer manipulation,
        // but the ModuleLoader handles the valid case. The C-wrapper inside lib.rs checks for nulls.
    }

    #[test]
    fn test_template_malformed_config() {
        let api = create_mock_api();
        
        // 1. Missing config file
        {
             let params_json = r#"{"config_file": "/non/existent/path.yaml"}"#;
             // initialize_module returns null on failure
             // We can't use ModuleLoader::new directly because it might panic or we want to assert failure.
             // But ModuleLoader doesn't enforce non-null instance_ptr in new immediately? 
             // unique_ptr in ModuleLoader: `instance_ptr: *mut c_void,`
             // Let's use raw initialize_module calls for failure cases as before.
             
             let api_ptr = &api as *const ox_webservice_api::WebServiceApiV1;
             let c_params = std::ffi::CString::new(params_json).unwrap();
             unsafe {
                 let res = initialize_module(c_params.as_ptr(), api_ptr);
                 assert!(res.is_null(), "Should fail on missing config file");
             }
        }
    
        // 2. Bad JSON
        {
             let params_json = r#"{ bad json }"#;
             let api_ptr = &api as *const ox_webservice_api::WebServiceApiV1;
             let c_params = std::ffi::CString::new(params_json).unwrap();
             unsafe {
                 let res = initialize_module(c_params.as_ptr(), api_ptr);
                 assert!(res.is_null(), "Should fail on bad json");
             }
        }
    }
}
