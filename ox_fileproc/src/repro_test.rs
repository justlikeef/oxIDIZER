
#[cfg(test)]
mod tests {

    use std::io::Write;
    use std::fs::File;
    use crate::RawFile;

    #[test]
    fn test_toggle_driver_repro() {
        // Create a temp file simulating drivers.yaml
        let content = r#"drivers:
  - id: "delimited"
    name: "ox_persistence_driver_file_delimited"
    state: "disabled"
  - id: "mysql"
    name: "ox_persistence_driver_db_mysql"
    state: "enabled"
"#;
        let mut file = File::create("repro_drivers.yaml").unwrap();
        file.write_all(content.as_bytes()).unwrap();

        // Simulate logic from DriverManager::toggle_driver_status
        let id = "mysql";
        let config_file = "repro_drivers.yaml";
        
        let mut raw = RawFile::open(config_file).unwrap();
         
         // Logic from lib.rs
         let query = format!("drivers[id=\"{}\"]/state", id);
         println!("Query: {}", query);
         
         let span_and_quoted = raw.find(&query).next().map(|c| {
             let val = c.value().trim();
             (c.span.clone(), val.starts_with('"'), val.trim_matches('"').to_string())
         });
         
         if let Some((span, is_quoted, current_val)) = span_and_quoted {
              println!("Found: val={}, quoted={}", current_val, is_quoted);
              let new_status_val = if current_val == "enabled" { "disabled" } else { "enabled" };
              let replacement = if is_quoted { format!("\"{}\"", new_status_val) } else { new_status_val.to_string() };
              
              raw.update(span, &replacement);
              // raw.save().unwrap(); // verify update only
              println!("Update successful to {}", new_status_val);
         } else {
             panic!("Driver with ID '{}' not found or has no state field", id);
         }
         
         std::fs::remove_file("repro_drivers.yaml").unwrap();
    }
}
