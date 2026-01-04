use ox_messaging_client::{EventBusFactory, ClientConfig};
use ox_event_bus::EventBus;
use std::time::Duration;
use tokio::time::sleep;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();
    println!("[INFO] Starting Verification...");

    // 1. Create Config
    let port_str = std::env::var("MQTT_PORT").unwrap_or_else(|_| "1883".to_string());
    let port = port_str.parse::<u16>().expect("Invalid MQTT_PORT");

    let config = ClientConfig {
        provider: "mqtt".to_string(),
        host: Some("127.0.0.1".to_string()),
        port: Some(port),
        client_id: Some("verifier".to_string()),
    };

    // 2. Instantiate Bus
    let bus = EventBusFactory::create(config).await.map_err(|e| format!("Factory error: {}", e))?;
    // Verification: Give simulated network time to establish (local rumqttd <-> rumqttc)
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("[INFO] Connected to Broker");

    // 3. Test Subscribe
    let topic = "test/topic";
    let mut stream = bus.as_ref().subscribe(topic).await.map_err(|e| format!("Subscribe error: {}", e))?;
    println!("[INFO] Subscribed to {}", topic);
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 4. Test Publish (Async)
    let payload = b"Hello World".to_vec();
    bus.as_ref().publish(topic, &payload).await.map_err(|e| format!("Publish error: {}", e))?;
    println!("[INFO] Published message");

    // 5. Verify Receipt
    if let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(60), stream.next()).await {
        println!("[INFO] Received: {:?}", String::from_utf8_lossy(&msg.payload));
        assert_eq!(msg.payload, payload);
    } else {
        panic!("[ERROR] Timeout waiting for message");
    }

    // 6. Test Sync Request/Reply
    // We need a responder task
    let bus_clone = bus.clone(); // Arc clone
    tokio::spawn(async move {
        // Subscribe to request topic
        let mut req_stream = bus_clone.as_ref().subscribe("test/request").await.unwrap();
        while let Some(msg) = req_stream.next().await {
            if let Some(reply_to) = msg.reply_to.clone() {
                 println!("[RESPONDER] Got request, replying to {}", reply_to);
                 // Use reply method
                 let _ = bus_clone.as_ref().reply(&msg, b"ResponsePayload").await;
            } else {
                 println!("[RESPONDER] Got request but NO reply_to");
            }
        }
    });
    
    // Give responder time to sub
    sleep(Duration::from_secs(1)).await;
    
    println!("[INFO] Sending Request...");
    let response = bus.as_ref().request("test/request", b"RequestPayload", Duration::from_secs(10)).await;
    match response {
        Ok(msg) => {
             println!("[INFO] Got Response: {:?}", String::from_utf8_lossy(&msg.payload));
             assert_eq!(msg.payload, b"ResponsePayload");
        },
        Err(e) => panic!("[ERROR] Request failed: {:?}", e),
    }

    println!("[INFO] Verification Complete");
    Ok(())
}
