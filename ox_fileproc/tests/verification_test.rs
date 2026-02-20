use ox_fileproc::processor::{process_file, Processor};
use ox_fileproc::cursor::{RawFile, Format};
use std::fs;
use std::io::Write;
use tempfile::NamedTempFile;


#[test]
fn test_security_root_dir_violation() {
    let root_dir = tempfile::tempdir().unwrap();
    let safe_file = root_dir.path().join("safe.json");
    
    // Create a file outside root
    let outside_file = NamedTempFile::new().unwrap();
    let outside_path = outside_file.path().to_str().unwrap();

    fs::write(&safe_file, format!(r#"{{"include": "{}"}}"#, outside_path)).unwrap();

    let processor = Processor::new().with_root_dir(root_dir.path());
    let res = processor.process(&safe_file);
    
    assert!(res.is_err(), "Expected error, got Ok");
    let err_msg = format!("{:?}", res.err().unwrap());
    assert!(err_msg.contains("Security violation"));
}



#[test]
fn test_merge_array_numeric_ids() {
    let mut base_file = NamedTempFile::new().unwrap();
    write!(base_file, r#"{{
        "items": [
            {{ "id": 1, "val": "original" }},
            {{ "id": 2, "val": "original" }}
        ]
    }}"#).unwrap();
    
    // Overlay with numeric ID
    let mut overlay_file = NamedTempFile::new().unwrap();
    write!(overlay_file, r#"{{
        "items": [
            {{ "id": 1, "val": "updated" }}
        ]
    }}"#).unwrap();
    
    // Since we can't easily mock include with NamedTempFile without knowing paths relative to each other easily...
    // Actually we can use the `smart_merge_arrays` within library if exposed? It's private.
    // So we test via `process_file` logic or similar.
    // Instead of full file processing which requires `include`, let's check `process_includes` logic via `merge`.
    // Wait, `process_file` supports `merge` key locally.
    
    // Let's create a main file that merges the overlay.
    // We need both in same dir or absolute paths (if allowed).
    // Let's use `tempfile::Builder` to put them in same dir.
    
    let dir = tempfile::tempdir().unwrap();
    let base_path = dir.path().join("base.json");
    let overlay_path = dir.path().join("overlay.json");
    
    fs::write(&base_path, r#"{
        "items": [
            { "id": 1, "val": "original" },
            { "id": 2, "val": "original" }
        ]
    }"#).unwrap();
    
    fs::write(&overlay_path, r#"{
        "merge": "base.json",
        "items": [
            { "id": 1, "val": "updated" }
        ]
    }
    "#).unwrap();
    
    let val = process_file(&overlay_path, 5).unwrap();
    let items = val["items"].as_array().unwrap();
    
    // Item 1 should be updated
    let item1 = items.iter().find(|i| i["id"] == 1).unwrap();
    assert_eq!(item1["val"], "updated");
    
    // Item 2 should remain
    let item2 = items.iter().find(|i| i["id"] == 2).unwrap();
    assert_eq!(item2["val"], "original");
}



#[test]
fn test_substitution_parent_scope() {
    let dir = tempfile::tempdir().unwrap();
    let grandparent_path = dir.path().join("grandparent.json");
    let parent_path = dir.path().join("parent.json");
    let child_path = dir.path().join("child.json");
    
    fs::write(&grandparent_path, r#"{
        "substitutions": { "WHO": "Universe" },
        "include": "parent.json"
    }"#).unwrap();
    
    fs::write(&parent_path, r#"{
        "substitutions": "child.json", 
        "result": "${{MSG}}"
    }"#).unwrap();
    
    fs::write(&child_path, r#"{
        "MSG": "Hello ${{WHO}}"
    }"#).unwrap();
    
    let val = process_file(&grandparent_path, 5).unwrap();
    assert_eq!(val["result"], "Hello Universe");
}



#[test]
fn test_json_scanner_depth_and_regex_safety() {
    // We need to test RawFile finding with JSON format
    let content = r#"{
        "nested": { "key": "ignore_me" },
        "key": "correct_value",
        "tricky": "key",
        "regex_char": { "foo.bar": 1 }
    }"#;
    
    let raw = RawFile {
        path: std::path::PathBuf::from("test.json"),
        content: content.to_string(),
        format: Format::Json,
    };
    
    let cursor = raw.find("key").next().expect("Should find key");
    assert_eq!(cursor.value().trim_matches('"'), "correct_value");
    
    // Test regex char safety
    // Construct a JSON where the key has regex meta chars
    let content_regex = r#"{
        "foo.bar": "value"
    }"#;
     let raw_regex = RawFile {
        path: std::path::PathBuf::from("regex.json"),
        content: content_regex.to_string(),
        format: Format::Json,
    };
    let cursor_regex = raw_regex.find("foo.bar").next().expect("Should find foo.bar");
    assert_eq!(cursor_regex.value().trim_matches('"'), "value");
}


#[test]
fn test_unsupported_format_fallback() {
    // TOML file
    let content = r#"key = "value""#;
    let raw = RawFile {
        path: std::path::PathBuf::from("test.toml"),
        content: content.to_string(),
        format: Format::Toml,
    };
    
    // Should return None/Empty
    let mut iter = raw.find("key");
    assert!(iter.next().is_none());
}
