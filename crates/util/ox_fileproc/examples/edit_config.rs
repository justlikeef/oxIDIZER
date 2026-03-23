use ox_fileproc::RawFile;
use std::fs::File;
use std::io::Write;
use tempfile::NamedTempFile;

fn main() -> anyhow::Result<()> {
    // 1. Create a sample YAML file
    let file = NamedTempFile::new()?;
    // We don't use the extension logic here, just the file
    // Persist temp file to work with path extension logic if needed, 
    // or just pass path to RawFile::open if it exists.
    // NamedTempFile path might not have extension.
    // Let's create a real file for clarity in example
    let path = "example_config.yaml";
    {
        let mut f = File::create(path)?;
        writeln!(f, r#"
server:
  host: "localhost"
  port: 8080
  features:
    - name: "fast-mode"
      enabled: false
"#)?;
    }

    // 2. Open as RawFile
    let mut raw = RawFile::open(path)?;
    println!("Original Content:\n{}", raw.content);

    // 3. Find and Update Port
    let port_span = {
        let cursor = raw.find("server/port")
            .next()
            .expect("Should find port");
        println!("Found port: {}", cursor.value());
        cursor.span.clone()
    };
    raw.update(port_span, "9090");


    // 4. Find Feature and Enable it
    // Query: features list -> item with name="fast-mode" -> enabled key
    let query = "server/features[name=\"fast-mode\"]/enabled";
    let feature_span = {
        let cursor = raw.find(query)
            .next()
            .expect("Should find feature");
         println!("Found feature enabled status: {}", cursor.value());
         cursor.span.clone()
    };
    raw.update(feature_span, "true");

    println!("\nUpdated Content:\n{}", raw.content);
    
    // Clean up
    std::fs::remove_file(path)?;
    
    // Keep temp file alive until end?
    drop(file);
    Ok(())
}
