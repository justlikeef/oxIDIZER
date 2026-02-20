// use ox_fileproc::process_file;
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let root_path = dir.path();

    // 1. Setup "Shared" config file
    let shared_path = root_path.join("shared.yaml");
    {
        let mut f = File::create(&shared_path)?;
        writeln!(f, "common_setting: true")?;
        writeln!(f, "log_level: info")?;
    }

    // 2. Setup "Overlay" config file
    let overlay_path = root_path.join("overlay.yaml");
    {
        let mut f = File::create(&overlay_path)?;
        writeln!(f, "log_level: debug")?; // Overrides shared
        writeln!(f, "new_setting: 123")?;
    }
    
    // 3. Setup a Directory for recursive merge
    let parts_dir = root_path.join("parts");
    fs::create_dir(&parts_dir)?;
    {
        let mut f = File::create(parts_dir.join("part1.json"))?;
        writeln!(f, r#"{{"feature_a": "enabled"}}"#)?;
    }
    {
        let mut f = File::create(parts_dir.join("part2.json"))?;
        writeln!(f, r#"{{"feature_b": "disabled"}}"#)?;
    }

    // 4. Main Config using directives
    let main_path = root_path.join("main.yaml");
    {
        let mut f = File::create(&main_path)?;
        writeln!(f, "# Include: merges shared content into root")?;
        writeln!(f, "include: shared.yaml")?; 
        
        writeln!(f, "# Local override")?;
        writeln!(f, "app_name: My App")?;
        
        writeln!(f, "modules:")?;
        writeln!(f, "  # Merge: explicit merge of overlay into 'modules'")?;
        writeln!(f, "  merge: overlay.yaml")?;
        
        writeln!(f, "features:")?;
        writeln!(f, "  # Merge Recursive: loads all files in directory")?;
        writeln!(f, "  merge_recursive: parts")?;
    }

    println!("Processing config at: {:?}", main_path);
    use ox_fileproc::processor::Processor;
    let config = Processor::new().process(&main_path)?;
    
    println!("Final Resolved Config:\n{}", serde_json::to_string_pretty(&config)?);
    
    // Explanation of expected output:
    // root should have: common_setting=true, log_level=info, app_name="My App"
    // modules should have: log_level=debug, new_setting=123
    // features should have: feature_a=enabled, feature_b=disabled
    
    Ok(())
}
