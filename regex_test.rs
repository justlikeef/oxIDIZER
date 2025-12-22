use regex::Regex;

fn main() {
    let re = Regex::new(r"^/drivermanager(/.*)?$").unwrap();
    let text = "/drivermanager/wasm/ox_persistence_driver_manager_wasm_bg.wasm";
    
    match re.captures(text) {
        Some(caps) => {
            println!("Matched: {}", caps.get(0).unwrap().as_str());
            if let Some(m) = caps.get(1) {
                println!("Capture 1: '{}'", m.as_str());
            } else {
                println!("Capture 1: None");
            }
        },
        None => println!("No match"),
    }
}
