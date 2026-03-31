use crate::{ox_plugin_init, ox_plugin_process};
use ox_webservice_test_utils::{
    create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field, PluginHandle,
};
use ox_workflow_abi::FLOW_CONTROL_CONTINUE;
use std::io::Write;
use tempfile::Builder;

#[test]
fn test_errorhandler_basic_flow() {
    let content_dir = tempfile::tempdir().unwrap();
    let root_path = content_dir.path();

    let mut template_404 = std::fs::File::create(root_path.join("404.jinja2")).unwrap();
    write!(template_404, "Custom 404 Error: {{ status_text }}").unwrap();

    let mut config_file = Builder::new().suffix(".yaml").tempfile().unwrap();
    let config_content = format!("content_root: \"{}\"", root_path.to_str().unwrap());
    writeln!(config_file, "{}", config_content).unwrap();

    let api = create_mock_api();
    let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
    let handle = PluginHandle::init(ox_plugin_init, &params_json, &api).expect("Failed to load module");

    // Test 1: Status < 400 (Should be ignored)
    {
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "response.status", "200");
        let flow = handle.process(ox_plugin_process, task_ctx);
        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        assert!(get_mock_field(task_ctx, "response.body").unwrap_or_default().is_empty());
        unsafe { drop_task_state(task_ctx); }
    }

    // Test 2: Status 404 (Should render 404.jinja2)
    {
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "response.status", "404");
        let flow = handle.process(ox_plugin_process, task_ctx);
        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert_eq!(body, "Custom 404 Error: Not Found");
        assert_eq!(
            get_mock_field(task_ctx, "response.header.Content-Type").unwrap_or_default(),
            "text/html"
        );
        unsafe { drop_task_state(task_ctx); }
    }

    // Test 3: Status 500 with index.jinja2 fallback
    {
        let mut template_index = std::fs::File::create(root_path.join("index.jinja2")).unwrap();
        write!(template_index, "Generic Error: {{ status_code }}").unwrap();

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "response.status", "500");
        let flow = handle.process(ox_plugin_process, task_ctx);
        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert_eq!(body, "Generic Error: 500");
        unsafe { drop_task_state(task_ctx); }
    }
}

#[test]
fn test_errorhandler_malformed_config() {
    let api = create_mock_api();
    let result = PluginHandle::init(ox_plugin_init, "{ bad json }", &api);
    assert!(result.is_err());
}

#[test]
fn test_errorhandler_reproduce_crash_with_bad_structure() {
    let api = create_mock_api();
    let mut config_file = Builder::new().suffix(".yaml").tempfile().unwrap();
    let bad_config_content = r#"
modules:
  - id: "errorhandler_jinja2"
    name: "ox_webservice_errorhandler_jinja2"
    params:
      config_file: "/some/path/to/self.yaml"
    "#;
    writeln!(config_file, "{}", bad_config_content).unwrap();
    let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
    let result = PluginHandle::init(ox_plugin_init, &params_json, &api);
    assert!(result.is_err(), "Module should fail to initialize with bad config structure");
}
