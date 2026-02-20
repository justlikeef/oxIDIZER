use ox_fileproc::{RawFile, Format};

#[test]
fn test_integration_yaml_editing() {
    // 1. Setup a virtual file content
    let initial_content = r#"
app:
  name: "MyApp"
  version: "1.0.0"
  features:
    - name: "auth"
      enabled: true
    - name: "logging"
      enabled: false
"#;

    let mut raw = RawFile {
        path: "config.yaml".into(),
        content: initial_content.to_string(),
        format: Format::Yaml,
    };

    // 2. Find and update version
    let version_span = {
        let cursor = raw.find("app/version").next().expect("Should find version");
        assert_eq!(cursor.value(), "\"1.0.0\"");
        cursor.span.clone()
    };
    
    // Update version to 1.1.0
    raw.update(version_span, "\"1.1.0\"");
    assert!(raw.content.contains("version: \"1.1.0\""));
    
    // 3. Enable logging
    let logging_span = {
        let cursor = raw.find("app/features[name=\"logging\"]/enabled")
            .next()
            .expect("Should find logging enabled flag");
        assert_eq!(cursor.value(), "false");
        cursor.span.clone()
    };
    
    raw.update(logging_span, "true");
    assert!(raw.content.contains("enabled: true"));
    
    // 4. Verify structural integrity
    // (Simple check via string matching, in real integration we might re-parse as YAML)
    assert!(raw.content.contains("- name: \"logging\""));
    assert!(raw.content.contains("enabled: true")); // now true
}
