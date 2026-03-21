use async_trait::async_trait;
use futures::{Stream, StreamExt};
use rumqttc::{AsyncClient, MqttOptions, QoS, Event, Packet};
use std::collections::{HashMap, BinaryHeap};
use std::sync::Arc;
use std::time::Duration;
use std::cmp::Ordering;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;
use ox_event_bus::{EventBus, EventMessage, BusError, QueueConfig};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
struct OxEnvelope {
    payload: Vec<u8>, 
    headers: HashMap<String, String>,
    correlation_id: Option<String>,
    reply_to: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PrioritizedMessage {
    pub priority: u8,
    pub message: EventMessage,
}

impl PartialEq for PrioritizedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for PrioritizedMessage {}
impl PartialOrd for PrioritizedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PrioritizedMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        // High priority first
        self.priority.cmp(&other.priority)
    }
}

pub struct AsyncPriorityQueue {
    heap: Mutex<BinaryHeap<PrioritizedMessage>>,
    notify: Notify,
}

impl AsyncPriorityQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            heap: Mutex::new(BinaryHeap::new()),
            notify: Notify::new(),
        })
    }
    
    pub async fn push(&self, msg: EventMessage, priority: u8) {
        self.heap.lock().await.push(PrioritizedMessage { priority, message: msg });
        self.notify.notify_waiters();
    }
    
    pub async fn pop(&self) -> EventMessage {
        loop {
            let mut heap = self.heap.lock().await;
            if let Some(p_msg) = heap.pop() {
                return p_msg.message;
            }
            drop(heap);
            self.notify.notified().await;
        }
    }
}

pub struct MqttBus {
    client: AsyncClient,
    _eventloop_handle: tokio::task::JoinHandle<()>,
    subscribers: Arc<Mutex<HashMap<String, Arc<AsyncPriorityQueue>>>>,
}

impl MqttBus {
    pub async fn new(client_id: &str, host: &str, port: u16) -> Arc<Self> {
        let mut mqttoptions = MqttOptions::new(client_id, host, port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));
        
        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
        let subscribers = Arc::new(Mutex::new(HashMap::<String, Arc<AsyncPriorityQueue>>::new()));
        
        let subscribers_clone = subscribers.clone();
        let handle = tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(event) => {
                         if let Event::Incoming(Packet::Publish(p)) = event {
                             let topic = p.topic;
                             let raw_payload = p.payload.to_vec();
                             
                             let (payload, headers, correlation_id, reply_to, priority) = match serde_json::from_slice::<OxEnvelope>(&raw_payload) {
                                 Ok(env) => {
                                     let prio = env.headers.get("x-priority").and_then(|s| s.parse().ok()).unwrap_or(0);
                                     (env.payload, env.headers, env.correlation_id, env.reply_to, prio)
                                 },
                                 Err(_) => {
                                     (raw_payload, HashMap::new(), None, None, 0)
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
                             if let Some(q) = subs.get(&topic) {
                                 q.push(msg, priority).await;
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
             let pq = AsyncPriorityQueue::new();
             subs.insert(topic.to_string(), pq);
             self.client.subscribe(topic, QoS::AtLeastOnce).await.map_err(|e| BusError::SubscriptionError(e.to_string()))?;
         }
         
         let q = subs.get(topic).unwrap().clone();
         
         let stream = async_stream::stream! {
             loop {
                 yield q.pop().await;
             }
         };
            
         Ok(Box::pin(stream))
    }

    async fn request(&self, topic: &str, payload: &[u8], timeout_duration: Duration) -> Result<EventMessage, BusError> {
        let correlation_id = Uuid::new_v4().to_string();
        let reply_topic = format!("replies/{}", correlation_id);
        
        let mut stream = self.subscribe(&reply_topic).await?;
        
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

    // --- Queue API Extensions ---

    async fn create_queue(&self, name: &str, _config: QueueConfig) -> Result<String, BusError> {
        let mut subs = self.subscribers.lock().await;
        if !subs.contains_key(name) {
             let pq = AsyncPriorityQueue::new();
             subs.insert(name.to_string(), pq);
             self.client.subscribe(name, QoS::AtLeastOnce).await.map_err(|e| BusError::SubscriptionError(e.to_string()))?;
        }
        // Save config rules somewhere or apply quotas here in the future
        Ok(name.to_string())
    }

    async fn publish_to_queue(&self, queue_id: &str, priority: u8, payload: &[u8]) -> Result<(), BusError> {
        let mut headers = HashMap::new();
        headers.insert("x-priority".to_string(), priority.to_string());
        
        let env = OxEnvelope {
            payload: payload.to_vec(),
            headers,
            correlation_id: None,
            reply_to: None,
        };
        self.publish_envelope(queue_id, env).await
    }

    async fn subscribe_to_queue(&self, queue_id: &str) -> Result<std::pin::Pin<Box<dyn Stream<Item = EventMessage> + Send>>, BusError> {
        self.subscribe(queue_id).await
    }
}
