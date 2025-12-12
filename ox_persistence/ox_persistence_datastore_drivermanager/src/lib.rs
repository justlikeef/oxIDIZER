use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DriverManagerConfig {
    pub drivers_file: String,
    pub driver_root: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfiguredDriver {
    pub name: String,
    pub library_path: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DriversList {
    #[serde(default)]
    pub drivers: Vec<ConfiguredDriver>,
}

pub struct DriverManager {
    config: DriverManagerConfig,
}

impl DriverManager {
    pub fn new(config: DriverManagerConfig) -> Self {
        DriverManager { config }
    }

    /// Lists all potential driver files in the driver_root directory
    pub fn list_available_driver_files(&self) -> Result<Vec<String>, String> {
        let root = Path::new(&self.config.driver_root);
        if !root.exists() {
            return Err(format!("Driver root directory does not exist: {}", self.config.driver_root));
        }

        let mut files = Vec::new();
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy();
                    if ext_str == "so" || ext_str == "dll" || ext_str == "dylib" {
                        if let Ok(stripped) = path.strip_prefix(root) {
                            files.push(stripped.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        Ok(files)
    }

    /// Loads the list of currently configured drivers
    pub fn load_configured_drivers(&self) -> Result<DriversList, String> {
        let path = Path::new(&self.config.drivers_file);
        if !path.exists() {
             return Ok(DriversList { drivers: Vec::new() });
        }
        
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }

    /// Saves the list of configured drivers
    pub fn save_configured_drivers(&self, list: &DriversList) -> Result<(), String> {
         let content = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
         // Ensure directory exists
         if let Some(parent) = Path::new(&self.config.drivers_file).parent() {
             fs::create_dir_all(parent).map_err(|e| e.to_string())?;
         }
         fs::write(&self.config.drivers_file, content).map_err(|e| e.to_string())
    }

    /// Attempts to load a driver file and retrieve its metadata
    /// This verifies it's a valid driver and gets its details
    pub fn get_driver_metadata(&self, relative_path: &str) -> Result<String, String> {
        let full_path = Path::new(&self.config.driver_root).join(relative_path);
        
        unsafe {
            let lib = libloading::Library::new(&full_path).map_err(|e| format!("Failed to load library: {}", e))?;
            
            // Check for metadata symbol
            let get_metadata: libloading::Symbol<unsafe extern "C" fn() -> *mut libc::c_char> = 
                lib.get(b"ox_driver_get_driver_metadata").map_err(|_| "Missing symbol: ox_driver_get_driver_metadata".to_string())?;
            
            let ptr = get_metadata();
            if ptr.is_null() {
                return Err("ox_driver_get_driver_metadata returned null".to_string());
            }
            
            let c_str = std::ffi::CStr::from_ptr(ptr);
             let meta_str = c_str.to_string_lossy().into_owned();
             
             Ok(meta_str)
        }
    }
}
