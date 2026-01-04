use std::path::PathBuf;
use std::fs;
use ox_webservice::{ServerConfig, pipeline::Pipeline, ModuleConfig};
use tempfile::tempdir;

#[test]
fn test_dynamic_module_loading_custom_path() {
    // 1. Locate a valid .so to test with (e.g., ox_pipeline_router)
    let debug_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug");
    let source_lib = debug_dir.join("libox_pipeline_router.so");
    
    if !source_lib.exists() {
        // Skip test if target not built (shouldn't happen in explicit test run)
        eprintln!("SKIPPING: Source library not found at {:?}", source_lib);
        return;
    }

    // 2. Create temp dir and copy lib there with unique name
    let dir = tempdir().unwrap();
    let custom_lib_path = dir.path().join("libcustom_router_test.so");
    fs::copy(&source_lib, &custom_lib_path).expect("Failed to copy test lib");

    // 3. Define Config with Custom Path
    let module_config = ModuleConfig {
        name: "ox_pipeline_router".to_string(), // Must match internal name expected by lib? No, libname usually matches.
        // Actually, ox_pipeline_router's initialize might not care about name, 
        // but Pipeline::new loop for routers looks for "ox_pipeline_router" or whatever is in router_map.
        // Let's test "Module" loading first, then "Router" loading.
        
        // Testing generic Module loading
        id: Some("CustomLoadedModule".to_string()),
        path: Some(custom_lib_path.to_str().unwrap().to_string()),
        ..Default::default()
    };

    let server_config = ServerConfig {
        routes: vec![],
        modules: vec![module_config],
        log4rs_config: "log4rs.yaml".to_string(),
        enable_metrics: Some(false),
        pipeline: Some(ox_webservice::PipelineConfig {
             phases: Some(vec![]) // Empty phases
        }),
        servers: vec![],
    };

    // 4. Initialize Pipeline
    // This should succeed if it finds the lib at the custom path
    let result = Pipeline::new(&server_config, "{}".to_string());
    assert!(result.is_ok(), "Pipeline failed to initialize with custom module path: {:?}", result.err());
}

#[test]
fn test_dynamic_router_loading_custom_path() {
    // 1. Locate source lib
    let debug_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug");
    let source_lib = debug_dir.join("libox_pipeline_router.so");

    if !source_lib.exists() {
        return;
    }

    // 2. Temp dir copy
    let dir = tempdir().unwrap();
    let custom_lib_path = dir.path().join("libcustom_router_dl.so");
    fs::copy(&source_lib, &custom_lib_path).expect("Failed to copy test lib");

    // 3. Config
    // We define a router mapped phase "TestPhase" -> "CustomRouter"
    // And "CustomRouter" is defined in 'modules' with a custom path.
    
    // Define Module with ID "CustomRouter"
    let module_config = ModuleConfig {
        name: "CustomRouter".to_string(),
        id: Some("CustomRouter".to_string()),
        path: Some(custom_lib_path.to_str().unwrap().to_string()),
        ..Default::default()
    };
    
    // Define Pipeline Phase mapping
    let mut phase_map = std::collections::HashMap::new();
    phase_map.insert("TestPhase".to_string(), "CustomRouter".to_string());

    let server_config = ServerConfig {
        routes: vec![],
        modules: vec![module_config],
        log4rs_config: "log4rs.yaml".to_string(), // won't be used/checked really by Pipeline::new
        enable_metrics: Some(false),
        pipeline: Some(ox_webservice::PipelineConfig {
             phases: Some(vec![phase_map])
        }),
        servers: vec![],
    };
    
    // 4. Initialize
    // Pipeline::new logic:
    // - Iterates phases (TestPhase)
    // - Gets router_id = "CustomRouter"
    // - Looks for "CustomRouter" in modules list
    // - Finds it, sees custom path
    // - Loads from custom path
    let result = Pipeline::new(&server_config, "{}".to_string());
    assert!(result.is_ok(), "Pipeline failed to load dynamic router: {:?}", result.err());
}

#[test]
fn test_dynamic_loading_failure() {
    // 1. Point to non-existent file
    let module_config = ModuleConfig {
        name: "FailModule".to_string(),
        id: Some("FailModule".to_string()),
        path: Some("/non/existent/path/libfail.so".to_string()),
        ..Default::default()
    };

    let server_config = ServerConfig {
        routes: vec![],
        modules: vec![module_config],
        log4rs_config: "log4rs.yaml".to_string(),
        enable_metrics: Some(false),
        pipeline: Some(ox_webservice::PipelineConfig {
             phases: Some(vec![])
        }),
        servers: vec![],
    };

    let result = Pipeline::new(&server_config, "{}".to_string());
    // Should fail
    assert!(result.is_err(), "Pipeline should have failed loading missing module");
}
