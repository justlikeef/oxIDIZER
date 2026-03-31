use std::ffi::{c_char, c_void, CString};
use libloading::{Library, Symbol};
use std::path::PathBuf;
use cargo_metadata::MetadataCommand;
use std::fs;
use ox_webservice_test_utils::{create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field};
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_WORKFLOW_ABI_VERSION};

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
    fs::write(content_root.join("500.jinja2"), "Error page: {{ status_code }}").unwrap();
    fs::write(content_root.join("index.jinja2"), "Generic error page: {{ status_code }}").unwrap();

    let config_file_path = temp_dir.path().join("error_handler_config.yaml");
    fs::write(
        &config_file_path,
        format!("content_root: \"{}\"", content_root.to_str().unwrap()),
    )
    .unwrap();

    let api = create_mock_api();
    let params_json = serde_json::json!({
        "config_file": config_file_path.to_str().unwrap(),
    })
    .to_string();
    let params_cstring = CString::new(params_json).unwrap();

    unsafe {
        let lib_path = get_dynamic_library_path();
        if !lib_path.exists() {
            println!("Skipping FFI test: dylib not found at {:?}", lib_path);
            return;
        }

        let lib = Library::new(lib_path).unwrap();

        type InitFn = unsafe extern "C" fn(*const c_char, *const CoreHostApi, u32) -> *mut c_void;
        type ProcessFn = unsafe extern "C" fn(*mut c_void, *mut c_void) -> FlowControl;
        type DestroyFn = unsafe extern "C" fn(*mut c_void);

        let init_fn: Symbol<InitFn> = lib.get(b"ox_plugin_init").unwrap();
        let process_fn: Symbol<ProcessFn> = lib.get(b"ox_plugin_process").unwrap();
        let destroy_fn: Symbol<DestroyFn> = lib.get(b"ox_plugin_destroy").unwrap();

        let plugin_ctx = init_fn(params_cstring.as_ptr(), &api as *const CoreHostApi, OX_WORKFLOW_ABI_VERSION);
        assert!(!plugin_ctx.is_null());

        // Test with 500 status
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "response.status", "500");
        set_mock_field(task_ctx, "request.method", "GET");
        set_mock_field(task_ctx, "request.path", "/error");
        let result = process_fn(plugin_ctx, task_ctx);
        assert_eq!(result.code, FLOW_CONTROL_CONTINUE);
        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("Error page: 500"), "body was: {}", body);
        drop_task_state(task_ctx);

        // Test with 404 status (falls back to index.jinja2)
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "response.status", "404");
        set_mock_field(task_ctx, "request.path", "/notfound");
        let result = process_fn(plugin_ctx, task_ctx);
        assert_eq!(result.code, FLOW_CONTROL_CONTINUE);
        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("Generic error page: 404"), "body was: {}", body);
        drop_task_state(task_ctx);

        // Test OK status (should not modify body)
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "response.status", "200");
        let result = process_fn(plugin_ctx, task_ctx);
        assert_eq!(result.code, FLOW_CONTROL_CONTINUE);
        assert!(get_mock_field(task_ctx, "response.body").unwrap_or_default().is_empty());
        drop_task_state(task_ctx);

        destroy_fn(plugin_ctx);
    }
}
