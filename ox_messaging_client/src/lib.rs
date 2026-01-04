use ox_event_bus::EventBus;
use ox_event_bus_mqtt::MqttBus;
use serde::Deserialize;
use std::sync::Arc;
use std::fs::File;
use std::path::Path;

#[derive(Deserialize)]
pub struct ClientConfig {
    pub provider: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub client_id: Option<String>,
}

pub struct EventBusFactory;

impl EventBusFactory {
    pub async fn create_from_config<P: AsRef<Path>>(path: P) -> Result<Arc<dyn EventBus>, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open config: {}", e))?;
        let config: ClientConfig = serde_yaml::from_reader(file).map_err(|e| format!("Invalid config: {}", e))?;
        
        Self::create(config).await
    }
    
    pub async fn create(config: ClientConfig) -> Result<Arc<dyn EventBus>, String> {
        match config.provider.as_str() {
            "mqtt" => {
                let host = config.host.unwrap_or_else(|| "127.0.0.1".to_string());
                let port = config.port.unwrap_or(1883);
                let client_id = config.client_id.unwrap_or_else(|| format!("ox_client_{}", uuid::Uuid::new_v4()));
                
                Ok(MqttBus::new(&client_id, &host, port).await)
            }
            _ => Err(format!("Unknown provider: {}", config.provider)),
        }
    }
}
