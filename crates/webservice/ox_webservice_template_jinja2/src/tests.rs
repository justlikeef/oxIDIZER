#[cfg(test)]
mod tests {
    use crate::{ox_plugin_init, ox_plugin_process};
    use ox_webservice_test_utils::{
        create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field, PluginHandle,
    };
    use ox_workflow_abi::FLOW_CONTROL_CONTINUE;
    use std::io::Write;
    use tempfile::Builder;

    #[test]
    fn test_template_basic_flow_with_hacker_checks() {
        let mut mimetypes_file = Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(
            mimetypes_file,
            "mimetypes:\n  - extension: html\n    mimetype: text/html\n    handler: template\n    url: \".*\\\\.html$\""
        )
        .unwrap();

        let mut template_file = Builder::new().suffix(".html").tempfile().unwrap();
        writeln!(template_file, "Hello Jinja").unwrap();

        let mut config_file = Builder::new().suffix(".yaml").tempfile().unwrap();
        let config_content = format!(
            "content_root: \"{}\"\nmimetypes_file: \"{}\"\ndefault_documents: []",
            template_file.path().parent().unwrap().to_str().unwrap(),
            mimetypes_file.path().to_str().unwrap()
        );
        writeln!(config_file, "{}", config_content).unwrap();

        let api = create_mock_api();
        let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
        let handle = PluginHandle::init(ox_plugin_init, &params_json, &api).expect("Module load failed");

        // Test 1: Happy Path
        {
            let task_ctx = create_task_state();
            let request_path = format!("/{}", template_file.path().file_name().unwrap().to_str().unwrap());
            set_mock_field(task_ctx, "request.path", &request_path);
            let flow = handle.process(ox_plugin_process, task_ctx);
            assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
            let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
            assert_eq!(body, "Hello Jinja\n");
            let ct = get_mock_field(task_ctx, "response.header.Content-Type").unwrap_or_default();
            assert_eq!(ct, "text/html");
            unsafe { drop_task_state(task_ctx); }
        }

        // Test 2: Path Traversal → 404
        {
            let task_ctx = create_task_state();
            set_mock_field(task_ctx, "request.path", "/../../../../../../../../etc/passwd");
            let flow = handle.process(ox_plugin_process, task_ctx);
            assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
            assert_eq!(
                get_mock_field(task_ctx, "response.status").unwrap_or_default(),
                "404"
            );
            unsafe { drop_task_state(task_ctx); }
        }
    }

    #[test]
    fn test_template_malformed_config() {
        let api = create_mock_api();

        // 1. Missing config file
        {
            let params_json = r#"{"config_file": "/non/existent/path.yaml"}"#;
            let result = PluginHandle::init(ox_plugin_init, params_json, &api);
            assert!(result.is_err(), "Should fail on missing config file");
        }

        // 2. Bad JSON
        {
            let result = PluginHandle::init(ox_plugin_init, "{ bad json }", &api);
            assert!(result.is_err(), "Should fail on bad json");
        }
    }
}
