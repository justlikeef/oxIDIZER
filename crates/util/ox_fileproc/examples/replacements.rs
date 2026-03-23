use ox_fileproc::{RawFile, process_file};
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("config.yaml");

    // 1. Create Initial Config
    {
        let mut f = File::create(&path)?;
        writeln!(f, "app:")?;
        writeln!(f, "  name: MyApp")?;
        writeln!(f, "  version: \"1.0.0\"")?;
        writeln!(f, "  # This block will be replaced")?;
        writeln!(f, "  meta:")?;
        writeln!(f, "    author: Alice")?;
        writeln!(f, "    year: 2023")?;
        writeln!(f, "substitutions:")?;
        writeln!(f, "  REGION: \"us-east\"")?;
        writeln!(f, "location: \"${{{{REGION}}}}\"")?;
    }

    // --- CASE A: Surgical Text Replacement (RawFile) ---
    let mut raw = RawFile::open(&path)?;
    println!("--- ORIGINAL RAW ---\n{}", raw.content);

    // Replace a simple value
    let version_span = {
        let cur = raw.find("app/version").next().expect("Find version");
        cur.span.clone()
    };
    raw.update(version_span, "\"2.0.0\"");

    // Replace an entire block
    let meta_span = {
        let cur = raw.find("app/meta").next().expect("Find meta block");
        cur.span.clone()
    };
    // Note: We are replacing everything UNDER 'meta:' or including 'meta:'?
    // In YAML, find_child returns the value block.
    raw.update(meta_span, "\n    author: Bob\n    year: 2024\n    status: beta\n");

    println!("\n--- UPDATED RAW (Surgical) ---\n{}", raw.content);
    raw.save()?;

    // --- CASE B: Variable Substitution Replacement (process_file) ---
    // Now we load it and see the ${{REGION}} replacement
    let config = process_file(&path, 5)?;
    println!("\n--- RESOLVED JSON (with substitutions) ---");
    println!("{}", serde_json::to_string_pretty(&config)?);
    
    assert_eq!(config["location"], "us-east");
    assert_eq!(config["app"]["version"], "2.0.0");
    assert_eq!(config["app"]["meta"]["author"], "Bob");

    Ok(())
}
