use std::collections::HashMap;
use std::sync::RwLock;
use serde::{Deserialize, Serialize};
use lazy_static::lazy_static;

#[derive(Serialize, Deserialize, Clone)]
#[derive(prost::Message)]
pub struct DictionaryConfig {
    #[prost(string, tag = "1")]
    pub driver: String,
    #[prost(map = "string, string", tag = "2")]
    pub parameters: HashMap<String, String>,
}

lazy_static! {
    static ref GLOBAL_DICTIONARY_CONFIG: RwLock<Option<DictionaryConfig>> = RwLock::new(None);
}

pub fn set_dictionary_config(config: DictionaryConfig) {
    let mut writer = GLOBAL_DICTIONARY_CONFIG.write().unwrap();
    *writer = Some(config);
}

pub fn get_dictionary_config() -> Option<DictionaryConfig> {
    let reader = GLOBAL_DICTIONARY_CONFIG.read().unwrap();
    reader.clone()
}

