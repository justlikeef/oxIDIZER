use ox_event_bus::{EventBus, QueueConfig as BusQueueConfig};
use ox_event_bus_mqtt::MqttBus;
use ox_workflow_config::{load_config_from_file, QueuesManifest};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ClientConfig {
    pub provider: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub client_id: Option<String>,
}

pub struct EventBusFactory;

impl EventBusFactory {
    pub async fn create_from_config<P: AsRef<Path>>(
        path: P, 
        queues_path: P
    ) -> Result<Arc<dyn EventBus>, String> {
        let file = std::fs::File::open(&path).map_err(|e| format!("Failed to open config: {}", e))?;
        let config: ClientConfig = serde_yaml::from_reader(file).map_err(|e| format!("Invalid config: {}", e))?;
        
        let bus = Self::create(config).await?;

        // Initialize queues from config if provided
        if let Ok(manifest) = load_config_from_file::<QueuesManifest>(queues_path.as_ref()) {
            for q in manifest.queues {
                let bus_cfg = BusQueueConfig {
                    priority_levels: q.priority_levels,
                    max_messages: q.max_messages,
                    max_throughput_per_sec: q.max_throughput_per_sec,
                };
                if let Err(e) = bus.create_queue(&q.name, bus_cfg).await {
                    log::warn!("Failed to initialize queue {}: {}", q.name, e);
                } else {
                    log::info!("Initialized queue: {}", q.name);
                }
            }
        } else {
            log::warn!("queues.yaml not found or invalid at {:?}", queues_path.as_ref());
        }
        
        Ok(bus)
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
