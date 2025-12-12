use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, PipelineState, AllocFn
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::PathBuf;
use regex::Regex;

const MODULE_NAME: &str = "ox_webservice_rewrite";

#[derive(Debug, Deserialize)]
pub struct RewriteRule {
    pub match_pattern: String,
    pub replace_string: String,
}

#[derive(Debug, Deserialize)]
pub struct RewriteConfig {
    pub rules: Vec<RewriteRule>,
}

pub struct RewriteModule<'a> {
    config: RewriteConfig,
    regexes: Vec<Regex>,
    api: &'a WebServiceApiV1,
}

impl<'a> RewriteModule<'a> {
    pub fn new(config: RewriteConfig, api: &'a WebServiceApiV1) -> anyhow::Result<Self> {
        let module_name = CString::new(MODULE_NAME).unwrap();
        let message = CString::new("ox_webservice_rewrite: Initializing...").unwrap();
        unsafe { (api.log_callback)(LogLevel::Debug, module_name.as_ptr(), message.as_ptr()); }

        let mut regexes = Vec::new();
        for rule in &config.rules {
            regexes.push(Regex::new(&rule.match_pattern)?);
        }

        Ok(Self {
            config,
            regexes,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        let pipeline_state = unsafe { &mut *pipeline_state_ptr };

        unsafe {
            let arena_ptr = &pipeline_state.arena as *const bumpalo::Bump as *const c_void;
            let c_str_path = (self.api.get_request_path)(pipeline_state, arena_ptr, self.api.alloc_str);
            let path = CStr::from_ptr(c_str_path).to_string_lossy();

            for (i, regex) in self.regexes.iter().enumerate() {
                if regex.is_match(&path) {
                    let rule = &self.config.rules[i];
                    let replacement = regex.replace(&path, rule.replace_string.as_str());
                    let new_path = replacement.to_string();

                    if path != new_path {
                        let module_name = CString::new(MODULE_NAME).unwrap();
                        let message = CString::new(format!("Rewrite match found. Rewriting path from '{}' to '{}'", path, new_path)).unwrap();
                        (self.api.log_callback)(LogLevel::Info, module_name.as_ptr(), message.as_ptr());

                        let c_new_path = CString::new(new_path).unwrap();
                        (self.api.set_request_path)(pipeline_state, c_new_path.as_ptr());
                        
                        return HandlerResult::ModifiedContinue;
                    }
                }
            }
        }

        HandlerResult::UnmodifiedContinue
    }
}

#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    api: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    let result = panic::catch_unwind(|| {
        let api_instance = unsafe { &*api };
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() };
        let params: Value =
            serde_json::from_str(module_params_json).expect("Failed to parse module params JSON");

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                let log_msg = CString::new("\"config_file\" parameter is missing or not a string.").unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let config_path = PathBuf::from(config_file_name);
        
        let config: RewriteConfig = match ox_fileproc::process_file(&config_path, 5) {
            Ok(value) => match serde_json::from_value(value) {
                Ok(c) => c,
                Err(e) => {
                     let log_msg = CString::new(format!("Failed to deserialize RewriteConfig: {}", e)).unwrap();
                     let module_name = CString::new(MODULE_NAME).unwrap();
                     unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                     return std::ptr::null_mut();
                }
            },
            Err(e) => {
                 let log_msg = CString::new(format!("Failed to process config file '{}': {}", config_file_name, e)).unwrap();
                 let module_name = CString::new(MODULE_NAME).unwrap();
                 unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                 return std::ptr::null_mut();
            }
        };

        let handler = match RewriteModule::new(config, api_instance) {
            Ok(h) => h,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to create RewriteModule: {}", e)).unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api_instance.log_callback,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            eprintln!("Panic during module initialization: {:?}", e);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_raw_c: AllocFn, 
    _arena: *const c_void, 
) -> HandlerResult {
    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut RewriteModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            let log_msg =
                format!("Panic occurred in process_request_c: {:?}.", e);
            let c_log_msg = CString::new(log_msg).unwrap();
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), c_log_msg.as_ptr()); } 
            HandlerResult::ModifiedJumpToError
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex_rewrite() {
        let rules = vec![
            RewriteRule {
                match_pattern: "^/api/v1/(.*)$".to_string(),
                replace_string: "/api/v2/$1".to_string(),
            },
            RewriteRule {
                match_pattern: "^/home$".to_string(),
                replace_string: "/index.html".to_string(),
            },
        ];

        let regexes: Vec<Regex> = rules.iter().map(|r| Regex::new(&r.match_pattern).unwrap()).collect();

        // Test case 1
        let path1 = "/api/v1/users";
        let mut rewritten = false;
        let mut new_path1 = String::new();
        for (i, regex) in regexes.iter().enumerate() {
            if regex.is_match(path1) {
                new_path1 = regex.replace(path1, rules[i].replace_string.as_str()).to_string();
                rewritten = true;
                break;
            }
        }
        assert!(rewritten);
        assert_eq!(new_path1, "/api/v2/users");

        // Test case 2
        let path2 = "/home";
        let mut rewritten2 = false;
        let mut new_path2 = String::new();
        for (i, regex) in regexes.iter().enumerate() {
            if regex.is_match(path2) {
                new_path2 = regex.replace(path2, rules[i].replace_string.as_str()).to_string();
                rewritten2 = true;
                break;
            }
        }
        assert!(rewritten2);
        assert_eq!(new_path2, "/index.html");

        // Test case 3 no match
        let path3 = "/about";
        let mut rewritten3 = false;
        for (i, regex) in regexes.iter().enumerate() {
            if regex.is_match(path3) {
                rewritten3 = true;
                break;
            }
        }
        assert!(!rewritten3);
    }
}
