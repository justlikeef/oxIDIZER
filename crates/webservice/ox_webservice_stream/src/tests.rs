#[cfg(test)]
mod tests {
    use crate::{ox_plugin_init, ox_plugin_process};
    use ox_webservice_test_utils::{
        create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field, PluginHandle,
    };
    use ox_workflow_abi::{FLOW_CONTROL_CONTINUE, FLOW_CONTROL_STREAM_FILE};
    use std::io::Write;

    #[test]
    fn test_stream_basic_flow_with_hacker_checks() {
        let mut mimetypes_file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(
            mimetypes_file,
            "mimetypes:\n  - extension: txt\n    mimetype: text/plain\n    handler: stream\n    url: \".*\\\\.txt$\""
        )
        .unwrap();

        let mut content_file = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
        write!(content_file, "Safe Content").unwrap();

        let mut config_file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        let config_content = format!(
            "content_root: \"{}\"\nmimetypes_file: \"{}\"\ndefault_documents: []",
            content_file.path().parent().unwrap().to_str().unwrap(),
            mimetypes_file.path().to_str().unwrap()
        );
        writeln!(config_file, "{}", config_content).unwrap();

        let api = create_mock_api();
        let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
        let handle = PluginHandle::init(ox_plugin_init, &params_json, &api).expect("Module load failed");

        // 1. Happy Path: file found → FLOW_CONTROL_STREAM_FILE with path in payload
        {
            let task_ctx = create_task_state();
            let request_path = format!("/{}", content_file.path().file_name().unwrap().to_str().unwrap());
            set_mock_field(task_ctx, "request.path", &request_path);
            let flow = handle.process(ox_plugin_process, task_ctx);
            assert_eq!(flow.code, FLOW_CONTROL_STREAM_FILE);
            assert!(!flow.payload.is_null());
            let path_str = unsafe { std::ffi::CStr::from_ptr(flow.payload).to_str().unwrap() };
            assert_eq!(path_str, content_file.path().to_str().unwrap());
            assert!(get_mock_field(task_ctx, "response.body").unwrap_or_default().is_empty());
            unsafe { drop_task_state(task_ctx); }
        }

        // 2. Path Traversal → 404
        {
            let task_ctx = create_task_state();
            set_mock_field(task_ctx, "request.path", "/../../../../../../etc/passwd");
            let flow = handle.process(ox_plugin_process, task_ctx);
            assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
            assert_eq!(get_mock_field(task_ctx, "response.status").unwrap_or_default(), "404");
            unsafe { drop_task_state(task_ctx); }
        }

        // 3. Large URL → graceful handling
        {
            let task_ctx = create_task_state();
            let huge_path = "/".to_string() + &"a".repeat(10000);
            set_mock_field(task_ctx, "request.path", &huge_path);
            let flow = handle.process(ox_plugin_process, task_ctx);
            assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
            unsafe { drop_task_state(task_ctx); }
        }

        // 4. Bad config → init fails
        {
            let bad_json = r#"{"config_file": "/nonexistent"}"#;
            let bad_handle = PluginHandle::init(ox_plugin_init, bad_json, &api);
            assert!(bad_handle.is_err(), "Should fail on missing config file");
        }
    }
}
