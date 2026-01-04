use async_trait::async_trait;
use futures::{Stream, StreamExt};
use rumqttc::{AsyncClient, MqttOptions, QoS, Event, Packet};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;
use ox_event_bus::{EventBus, EventMessage, BusError};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
struct OxEnvelope {
    payload: Vec<u8>, 
    headers: HashMap<String, String>,
    correlation_id: Option<String>,
    reply_to: Option<String>,
}

pub struct MqttBus {
    client: AsyncClient,
    _eventloop_handle: tokio::task::JoinHandle<()>,
    subscribers: Arc<Mutex<HashMap<String, tokio::sync::broadcast::Sender<EventMessage>>>>,
}

impl MqttBus {
    pub async fn new(client_id: &str, host: &str, port: u16) -> Arc<Self> {
        let mut mqttoptions = MqttOptions::new(client_id, host, port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));
        
        // Disable clean session to prevent dropped messages on intermittent disco? 
        // Or Keep it clean for RPC? 
        // Default is clean=true.
        
        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
        let subscribers = Arc::new(Mutex::new(HashMap::<String, tokio::sync::broadcast::Sender<EventMessage>>::new()));
        
        let subscribers_clone = subscribers.clone();
        let handle = tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(event) => {
                         log::debug!("DEBUG EVENT: {:?}", event); 
                         if let Event::Incoming(Packet::Publish(p)) = event {
                             log::debug!("DEBUG PUBLISH: {:?}", p);
                             let topic = p.topic;
                             let raw_payload = p.payload.to_vec();
                             
                             // Try to deserialize envelope
                             let (payload, headers, correlation_id, reply_to) = match serde_json::from_slice::<OxEnvelope>(&raw_payload) {
                                 Ok(env) => (env.payload, env.headers, env.correlation_id, env.reply_to),
                                 Err(_) => {
                                     // Fallback: Raw payload
                                     (raw_payload, HashMap::new(), None, None)
                                 }
                             };
                             
                             let msg = EventMessage {
                                 topic: topic.clone(),
                                 payload,
                                 headers,
                                 correlation_id, 
                                 reply_to, 
                             };

                             let subs = subscribers_clone.lock().await;
                             if let Some(tx) = subs.get(&topic) {
                                 let _ = tx.send(msg);
                             }
                         }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Arc::new(Self {
            client,
            _eventloop_handle: handle,
            subscribers: subscribers.clone(),
        })
    }
    
    async fn publish_envelope(&self, topic: &str, envelope: OxEnvelope) -> Result<(), BusError> {
        let payload = serde_json::to_vec(&envelope).map_err(|e| BusError::SerializationError(e.to_string()))?;
        self.client.publish(topic, QoS::AtLeastOnce, false, payload)
            .await
            .map_err(|e| BusError::PublishError(e.to_string()))
    }
}

#[async_trait]
impl EventBus for MqttBus {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), BusError> {
        // Wrap
        let env = OxEnvelope {
            payload: payload.to_vec(),
            headers: HashMap::new(),
            correlation_id: None,
            reply_to: None,
        };
        self.publish_envelope(topic, env).await
    }

    async fn subscribe(&self, topic: &str) -> Result<std::pin::Pin<Box<dyn Stream<Item = EventMessage> + Send>>, BusError> {
         let mut subs = self.subscribers.lock().await;
         
         if !subs.contains_key(topic) {
             let (tx, _) = tokio::sync::broadcast::channel(100);
             subs.insert(topic.to_string(), tx);
             self.client.subscribe(topic, QoS::AtLeastOnce).await.map_err(|e| BusError::SubscriptionError(e.to_string()))?;
         }
         
         let rx = subs.get(topic).unwrap().subscribe();
         let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
            .filter_map(|res| async move { res.ok() }); 
            
         Ok(Box::pin(stream))
    }

    async fn request(&self, topic: &str, payload: &[u8], timeout_duration: Duration) -> Result<EventMessage, BusError> {
        let correlation_id = Uuid::new_v4().to_string();
        let reply_topic = format!("replies/{}", correlation_id);
        
        let mut stream = self.subscribe(&reply_topic).await?;
        
        // Wrap with metadata
        let env = OxEnvelope {
            payload: payload.to_vec(),
            headers: HashMap::new(),
            correlation_id: Some(correlation_id),
            reply_to: Some(reply_topic),
        };
        
        self.publish_envelope(topic, env).await?; 
        
        match tokio::time::timeout(timeout_duration, stream.next()).await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => Err(BusError::ConnectionError("Stream closed".to_string())),
            Err(_) => Err(BusError::RequestTimeout),
        }
    }
    
    async fn reply(&self, original: &EventMessage, payload: &[u8]) -> Result<(), BusError> {
        if let Some(reply_to) = &original.reply_to {
             let env = OxEnvelope {
                 payload: payload.to_vec(),
                 headers: HashMap::new(),
                 correlation_id: original.correlation_id.clone(),
                 reply_to: None,
             };
             self.publish_envelope(reply_to, env).await
        } else {
             Err(BusError::PublishError("No Reply-To topic found".to_string()))
        }
    }
}
