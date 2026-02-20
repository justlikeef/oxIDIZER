// use ox_fileproc::process_file;
use std::fs::File;
use std::io::Write;

use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    // Setup temporary environment for demonstration
    let dir = tempdir()?;
    let config_path = dir.path().join("config.yaml");
    
    // Create a main config file
    let mut f = File::create(&config_path)?;
    writeln!(f, "app_name: MyApp")?;
    writeln!(f, "version: 1.0.0")?;
    writeln!(f, "substitutions:")?;
    writeln!(f, "  ENV: Production")?;
    writeln!(f, "environment: \"${{{{ENV}}}}\"")?;

    println!("Reading config from: {:?}", config_path);

    // Load and process the file
    // Use ProcessorBuilder for modern usage
    use ox_fileproc::processor::Processor;
    let config = Processor::new().process(&config_path)?;
    
    println!("Loaded Configuration:");
    println!("{}", serde_json::to_string_pretty(&config)?);
    
    // Cleanup happens automatically when `dir` goes out of scope
    Ok(())
}
