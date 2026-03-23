use ox_fileproc::{RawFile, Format};

fn main() -> anyhow::Result<()> {
    // Simulate content in memory for advanced manual manipulation
    let content = r#"
data:
  items:
    - id: A
      value: 10
    - id: B
      value: 20
"#;

    let raw = RawFile {
        path: "memory.yaml".into(),
        content: content.to_string(),
        format: Format::Yaml,
    };
    
    // Manual iteration over find results
    println!("Walking through items...");
    
    // Find the 'items' array content specifically
    // Note: Our find returns the value cursor.
    for cursor in raw.find("data/items") {
        println!("Items block found:\n{}", cursor.value());
    }
    
    // Specific targeted search
    let target_id = "B";
    let query = format!("data/items[id=\"{}\"]/value", target_id);
    
    if let Some(val_cursor) = raw.find(&query).next() {
        println!("Value for ID {}: {}", target_id, val_cursor.value());
    } else {
        println!("ID {} not found", target_id);
    }

    Ok(())
}
