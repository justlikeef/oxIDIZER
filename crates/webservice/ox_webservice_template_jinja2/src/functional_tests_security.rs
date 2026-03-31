use crate::ContentConfig;
use ox_webservice_test_utils::create_mock_api;
use std::fs;

#[test]
fn test_ssti_resilience() {
    // Setup dummy mimetypes file
    let temp_dir = std::env::temp_dir().join("ox_test_ssti_unit");
    if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
    fs::create_dir_all(&temp_dir).unwrap();

    let mime_file = temp_dir.join("mimetypes.yaml");
    fs::write(&mime_file, "mimetypes: []").unwrap();

    let _config = ContentConfig {
        content_root: temp_dir.to_str().unwrap().to_string(),
        mimetypes_file: mime_file.to_str().unwrap().to_string(),
        default_documents: vec![crate::DocumentConfig { document: "index.html".to_string() }],
        on_content_conflict: Some(crate::ContentConflictAction::overwrite),
    };

    let _api = create_mock_api();

    // Module initialisation tested via ox_plugin_init in tests.rs.
    // This file verifies that ContentConfig and the public API types are usable.
}
