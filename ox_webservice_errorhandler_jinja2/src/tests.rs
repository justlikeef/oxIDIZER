
use ox_webservice_api::{HandlerResult, LogLevel};
use ox_webservice_test_utils::{create_mock_api, ModuleLoader, mock_log, mock_alloc_raw};
use std::io::Write;
use tempfile::Builder;
use crate::initialize_module;

#[test]
fn test_errorhandler_basic_flow() {
    // --- Setup Config ---
    // Error Handler needs a content root with jinja2 templates (e.g. 404.jinja2, index.jinja2)
    let content_dir = tempfile::tempdir().unwrap();
    let root_path = content_dir.path();

    // Create 404.jinja2
    let mut template_404 = std::fs::File::create(root_path.join("404.jinja2")).unwrap();
    writeln!(template_404, "Custom 404 Error: {{{{ status_text }}}}").unwrap();

    // Config file
    let mut config_file = Builder::new().suffix(".yaml").tempfile().unwrap();
    // ErrorHandlerConfig has just content_root
    let config_content = format!(
        "content_root: \"{}\"",
        root_path.to_str().unwrap()
    );
    writeln!(config_file, "{}", config_content).unwrap();

    // --- Initialize ---
    let api = create_mock_api();
    let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
    
    let loader = ModuleLoader::load(initialize_module, &params_json, &api).expect("Failed to load module");

    // --- Test 1: Status < 400 (Should be ignored) ---
    {
        let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
        ps.status_code = 200;
        let result = loader.process_request(&mut ps, mock_log, mock_alloc_raw);
        assert_eq!(result, HandlerResult::UnmodifiedContinue);
        assert!(ps.response_body.is_empty());
    }

    // --- Test 2: Status 404 (Should render 404.jinja2) ---
    {
        let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
        ps.status_code = 404;
        let result = loader.process_request(&mut ps, mock_log, mock_alloc_raw);
        assert_eq!(result, HandlerResult::ModifiedContinue);
        assert_eq!(ps.response_body, b"Custom 404 Error: Not Found");
        assert_eq!(ps.response_headers.get("Content-Type").unwrap(), "text/html");
    }

    // --- Test 3: Status 500 (No specific template, fallback to index.jinja2 or text) ---
    // Let's create index.jinja2 first to test fallback
    {
        let mut template_index = std::fs::File::create(root_path.join("index.jinja2")).unwrap();
        writeln!(template_index, "Generic Error: {{{{ status_code }}}}").unwrap();

        let mut ps = ox_webservice_test_utils::create_stub_pipeline_state();
        ps.status_code = 500;
        let result = loader.process_request(&mut ps, mock_log, mock_alloc_raw);
        assert_eq!(result, HandlerResult::ModifiedContinue);
        assert_eq!(ps.response_body, b"Generic Error: 500");
    }
}

#[test]
fn test_errorhandler_malformed_config() {
     let api = create_mock_api();
     // Bad JSON
     let params_json = "{ bad json }";
     let result = ModuleLoader::load(initialize_module, params_json, &api);
     assert!(result.is_err());
}
