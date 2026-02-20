use ox_webservice_api::{
    InitializeModuleFn, ModuleStatus
};
use std::ffi::{c_void, CString};
use libloading::{Library, Symbol};
use std::path::PathBuf;
use cargo_metadata::MetadataCommand;
use std::fs;
use bumpalo::Bump;
use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state, mock_alloc_raw, mock_log};


fn get_dynamic_library_path() -> PathBuf {
    let metadata = MetadataCommand::new().exec().unwrap();
    let mut path = PathBuf::from(metadata.target_directory.as_str());
    path.push("debug");
    path.push("libox_webservice_errorhandler_jinja2.so");
    path
}

#[test]
fn test_module_loading_and_execution() {
    let temp_dir = tempfile::tempdir().unwrap();
    let content_root = temp_dir.path().to_path_buf();
    fs::create_dir_all(&content_root).unwrap();
    let error_500_template_path = content_root.join("500.jinja2");
    fs::write(&error_500_template_path, "Error page: {{ status_code }}").unwrap();
    let index_template_path = content_root.join("index.jinja2");
    fs::write(&index_template_path, "Generic error page: {{ status_code }}").unwrap();

    let config_file_path = temp_dir.path().join("error_handler_config.yaml");
    fs::write(&config_file_path, format!(r#"
content_root: "{}"
"#, content_root.to_str().unwrap())).unwrap();

    let api = create_mock_api();
    let params_json = serde_json::json!({
        "config_file": config_file_path.to_str().unwrap(),
    }).to_string();

    let params_cstring = CString::new(params_json).unwrap();

    unsafe {
        let lib_path = get_dynamic_library_path();
        // Skip if dylib doesn't exist (e.g. running cargo test without full build)
        if !lib_path.exists() {
            println!("Skipping FFI test: dylib not found at {:?}", lib_path);
            return;
        }

        let lib = Library::new(lib_path).unwrap();
        let init_func: Symbol<InitializeModuleFn> = lib.get(b"initialize_module").unwrap();
        let module_id = CString::new("test_module").unwrap();
        let module_interface_ptr = init_func(params_cstring.as_ptr(), module_id.as_ptr(), &api);
        assert!(!module_interface_ptr.is_null());
        let module_interface = Box::from_raw(module_interface_ptr);
        
        // Test with a specific error template (500.jinja2)
        let mut state_err_500 = create_stub_pipeline_state();
        state_err_500.status_code = 500;
        state_err_500.request_method = "GET".to_string();
        state_err_500.request_path = "/error".to_string();

        let result_err_500 = (module_interface.handler_fn)(
            module_interface.instance_ptr, 
            &mut state_err_500, 
            mock_log, 
            mock_alloc_raw, 
            &state_err_500.arena as *const Bump as *const c_void
        );
        
        assert_eq!(result_err_500.status, ModuleStatus::Modified);
        let body = String::from_utf8(state_err_500.response_body).unwrap();
        assert!(body.contains("Error page: 500"));

        // Test with a generic index.jinja2 template
        let mut state_err_404 = create_stub_pipeline_state();
        state_err_404.status_code = 404;
        state_err_404.request_path = "/notfound".to_string();

        let result_err_404 = (module_interface.handler_fn)(
            module_interface.instance_ptr, 
            &mut state_err_404, 
            mock_log, 
            mock_alloc_raw, 
            &state_err_404.arena as *const Bump as *const c_void
        );
        assert_eq!(result_err_404.status, ModuleStatus::Modified);
        let body = String::from_utf8(state_err_404.response_body).unwrap();
        assert!(body.contains("Generic error page: 404"));

        // Test OK status (should be unmodified)
        let mut state_ok = create_stub_pipeline_state();
        state_ok.status_code = 200;
        let result_ok = (module_interface.handler_fn)(
            module_interface.instance_ptr, 
            &mut state_ok, 
            mock_log, 
            mock_alloc_raw, 
            &state_ok.arena as *const Bump as *const c_void
        );
        assert_eq!(result_ok.status, ModuleStatus::Unmodified);

         // Clean up (Box takes ownership so it drops when out of scope, but since we constructed from raw, we need to be careful. 
         // In test scope it's fine, typically we'd have a destroy function).
    }
}
