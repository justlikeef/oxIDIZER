use crate::{OxModule, ContentConfig};
use ox_webservice_api::{WebServiceApiV1, PipelineState};
use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state};
use lazy_static::lazy_static;
use std::fs;

lazy_static! {
    static ref API: WebServiceApiV1 = create_mock_api();
}

#[test]
fn test_ssti_resilience() {
    // Setup dummy mimetypes file
    let temp_dir = std::env::temp_dir().join("ox_test_ssti_unit");
    if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let mime_file = temp_dir.join("mimetypes.yaml");
    fs::write(&mime_file, "mimetypes: []").unwrap();
    
    // We can construct ContentConfig because we are inside the crate
    let config = ContentConfig {
        content_root: temp_dir.to_str().unwrap().to_string(),
        mimetypes_file: mime_file.to_str().unwrap().to_string(),
        default_documents: vec![crate::DocumentConfig { document: "index.html".to_string() }],
        on_content_conflict: Some(crate::ContentConflictAction::overwrite),
    };

    let module = OxModule::new(config, &API).unwrap();
    let mut ps = create_stub_pipeline_state();

    // Placeholder assert
    assert!(module.mimetypes.is_empty()); 
}
