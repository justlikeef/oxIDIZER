use ox_fileproc::process_file;
use std::fs;
use tempfile::Builder;

#[test]
fn test_path_traversal_attempt() {
    // Attempt to read a file outside the directory
    // We create a "secret" file in a separate directory
    let secret_dir = Builder::new().prefix("secret").tempdir().unwrap();
    let secret_file = secret_dir.path().join("secret.txt");
    fs::write(&secret_file, "super_secret_data").unwrap();
    let secret_path_str = secret_file.to_str().unwrap();

    // Now create a config in a public directory that attempts to include the secret
    let public_dir = Builder::new().prefix("public").tempdir().unwrap();
    let _malicious_config = public_dir.path().join("evil.yaml");
    
    // Using absolute path - this generally SHOULD work if the library allows absolute paths. 
    // The security policy might be "allow" or "deny". Currently oxFileproc likely allows it.
    // We serve data, so if we can read /etc/passwd via a config file, that is a risk if the config file content is user-controlled.
    let _content = format!("secret: !include {}", secret_path_str);
    // Note: oxFileproc generally uses "include" or "merge" keys in JSON/map structures, not YAML tags currently.
    // Let's use the supported JSON include syntax.
    
    let malicious_json = public_dir.path().join("evil.json");
    let json_content = format!(r#"{{ "merged_secret": {{ "include": "{}" }} }}"#, secret_path_str);
    fs::write(&malicious_json, json_content).unwrap();

    let result = process_file(&malicious_json, 5);
    
    // For now, we expect this might succeed (as we haven't blocked it). 
    // The TEST is to document expected behavior. 
    // If we want to block it, this test should fail or assert error.
    if let Ok(val) = result {
        println!("Path traversal succeeded (as currently implemented). Value: {:?}", val);
    } else {
        println!("Path traversal failed.");
    }
}

#[test]
fn test_infinite_recursion_hard() {
    // A -> B -> C -> A
    let dir = Builder::new().prefix("cycle").tempdir().unwrap();
    let file_a = dir.path().join("a.json");
    let file_b = dir.path().join("b.json");
    let file_c = dir.path().join("c.json");

    let path_a = file_a.file_name().unwrap().to_str().unwrap();
    let path_b = file_b.file_name().unwrap().to_str().unwrap();
    let path_c = file_c.file_name().unwrap().to_str().unwrap();

    fs::write(&file_a, format!(r#"{{ "next": {{ "include": "{}" }} }}"#, path_b)).unwrap();
    fs::write(&file_b, format!(r#"{{ "next": {{ "include": "{}" }} }}"#, path_c)).unwrap();
    fs::write(&file_c, format!(r#"{{ "next": {{ "include": "{}" }} }}"#, path_a)).unwrap();

    // Depth limit should catch this
    let result = process_file(&file_a, 5);
    assert!(result.is_err(), "Should detect cycle or hit depth limit");
}

#[test]
fn test_billion_laughs_expansion() {
    // Simulate exponential growth via nested variable substitution? 
    // oxFileproc substitute doesn't do recursive substitution automatically unless we code it that way.
    // But let's try deep nesting of includes.
    
    // Root -> [A, A]
    // A -> [B, B]
    // B -> [C, C] ...
    // This scales as 2^N.
    
    let dir = Builder::new().prefix("expansion").tempdir().unwrap();
    
    // Leaf
    let leaf = dir.path().join("leaf.json");
    fs::write(&leaf, r#"{"data": "x"}"#).unwrap();
    let leaf_name = leaf.file_name().unwrap().to_str().unwrap();

    // Layer 1
    let layer1 = dir.path().join("layer1.json");
    fs::write(&layer1, format!(r#"{{ "a": {{ "include": "{}" }}, "b": {{ "include": "{}" }} }}"#, leaf_name, leaf_name)).unwrap();
    let l1_name = layer1.file_name().unwrap().to_str().unwrap();

    // Layer 2
    let layer2 = dir.path().join("layer2.json");
    fs::write(&layer2, format!(r#"{{ "a": {{ "include": "{}" }}, "b": {{ "include": "{}" }} }}"#, l1_name, l1_name)).unwrap();
    let l2_name = layer2.file_name().unwrap().to_str().unwrap();

    // Layer 3
    let layer3 = dir.path().join("layer3.json");
    fs::write(&layer3, format!(r#"{{ "a": {{ "include": "{}" }}, "b": {{ "include": "{}" }} }}"#, l2_name, l2_name)).unwrap();
    
    // If we process layer 3, we expect 2^3 = 8 copies of leaf data.
    // This isn't huge, but verification that the library handles it.
    
    let start = std::time::Instant::now();
    let result = process_file(&layer3, 10);
    let duration = start.elapsed();
    
    assert!(result.is_ok());
    println!("Expansion took: {:?}", duration);
}


#[test]
fn test_secure_processing_blocks_traversal() {
    // Setup:
    // /tmp/root/config.json
    // /tmp/outside_secret.txt
    
    // 1. Create secret outside root
    let outside_dir = Builder::new().prefix("outside").tempdir().unwrap();
    let secret_file = outside_dir.path().join("secret.txt");
    fs::write(&secret_file, "super_secret").unwrap();
    let secret_path_str = secret_file.to_str().unwrap();

    // 2. Create root dir
    let root_dir = Builder::new().prefix("root").tempdir().unwrap();
    let config_file = root_dir.path().join("config.json");
    
    // Case A: Relative Path Traversal
    // "include": "../../../outside/secret.txt" 
    // We can't easily guess the relative path to the temp dir random name, so we use absolute path.
    // If we use absolute path, it should ALSO fail if it's not starting with root.
    
    let content_abs = format!(r#"{{ "secret": {{ "include": "{}" }} }}"#, secret_path_str);
    fs::write(&config_file, &content_abs).unwrap();

    let processor = ox_fileproc::processor::Processor::new()
        .with_root_dir(root_dir.path());
        
    let result = processor.process(&config_file);
    assert!(result.is_err(), "Should block absolute path outside root");
    let err_msg = format!("{:?}", result.err().unwrap());
    assert!(err_msg.contains("Security violation") || err_msg.contains("outside the allowed root directory"), "Unexpected error: {}", err_msg);


    // Case B: Symlink Attack
    // Create a symlink inside root that points to outside secret
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link_path = root_dir.path().join("link_to_secret");
        symlink(&secret_file, &link_path).unwrap();
        
        let config_symlink = root_dir.path().join("symlink_config.json");
        // Include the SYMLINK (which is inside root)
        fs::write(&config_symlink, r#"{ "secret": { "include": "link_to_secret" } }"#).unwrap();
        
        // This should fail because canonicalize() resolves the link to outside path, which does not start with root.
        let result_sym = processor.process(&config_symlink);
        assert!(result_sym.is_err(), "Should block symlink to outside");
        let err_sym = format!("{:?}", result_sym.err().unwrap());
        assert!(err_sym.contains("Security violation") || err_sym.contains("outside the allowed root directory"));
    }
}

