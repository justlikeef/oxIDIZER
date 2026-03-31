/// verify_messaging: Verifies MQTT broker connectivity via publish/subscribe round-trip.
/// Reads MQTT_PORT from environment (default: 1883), connects to 127.0.0.1.
/// Exits 0 on success, 1 on failure.

use ox_event_bus::EventBus;
use ox_event_bus_mqtt::MqttBus;
use std::time::Duration;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("MQTT_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1883);

    let client_id = format!("verify_{}", Uuid::new_v4());
    println!("Connecting to MQTT broker at 127.0.0.1:{}", port);

    let bus = MqttBus::new(&client_id, "127.0.0.1", port).await;

    let test_topic = format!("verify/{}", Uuid::new_v4());
    let test_payload = b"verify_ok";

    // Subscribe
    let mut stream = match bus.subscribe(&test_topic).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to subscribe: {}", e);
            std::process::exit(1);
        }
    };

    // Give subscription time to register
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Publish
    if let Err(e) = bus.publish(&test_topic, test_payload).await {
        eprintln!("Failed to publish: {}", e);
        std::process::exit(1);
    }

    println!("Published to {}", test_topic);

    // Wait for receipt with timeout
    let received = tokio::time::timeout(Duration::from_secs(10), async {
        use futures::StreamExt;
        stream.next().await
    })
    .await;

    match received {
        Ok(Some(msg)) if msg.payload == test_payload => {
            println!("Received matching message on {}. MQTT verification PASSED.", test_topic);
            std::process::exit(0);
        }
        Ok(Some(msg)) => {
            eprintln!(
                "Received unexpected payload: {:?}",
                String::from_utf8_lossy(&msg.payload)
            );
            std::process::exit(1);
        }
        Ok(None) => {
            eprintln!("Stream ended without receiving message.");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("Timeout waiting for MQTT message.");
            std::process::exit(1);
        }
    }
}
