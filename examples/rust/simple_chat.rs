//! Simple Chat Agent
//!
//! Connects to a running Cowchat server, joins the lobby,
//! sends a greeting, then prints every message it receives.
//!
//! ```bash
//! # Start the server first:
//! cargo run -p cowchat-server -- serve
//!
//! # Then run this example:
//! cargo run -p cowchat-client --example simple_chat
//! ```

use cowchat_client::CowchatClient;
use cowchat_core::FrameType;
use std::path::PathBuf;

fn read_api_key() -> String {
    let home = std::env::var("HOME").expect("HOME not set");
    let key_path = PathBuf::from(home).join(".cowchat/auth.key");
    std::fs::read_to_string(&key_path)
        .unwrap_or_else(|_| panic!("Could not read API key from {}", key_path.display()))
        .trim()
        .to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = read_api_key();
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "example-agent".to_string());
    let agent_id = std::env::var("COWCHAT_AGENT_ID")
        .unwrap_or_else(|_| format!("{name}-{}", uuid::Uuid::new_v4()));

    println!("Connecting as '{name}'...");
    let client =
        CowchatClient::connect_tcp("127.0.0.1:9229", &key, &name, Some(&agent_id), vec![]).await?;
    println!("Connected! Agent ID: {}", client.agent_id);

    // Join the lobby
    client.join_room("lobby").await?;
    println!("Joined lobby");

    // Send a greeting
    client
        .send_message("lobby", &format!("Hello from {name}!"), None, vec![])
        .await?;
    println!("Sent greeting");

    // Listen for messages
    println!("\nListening for messages (Ctrl-C to quit)...\n");
    let mut events = client.subscribe();
    while let Ok(event) = events.recv().await {
        match event.frame.frame_type {
            FrameType::MessageReceived => {
                let p = &event.frame.payload;
                let from = p["agent_name"].as_str().unwrap_or("?");
                let room = p["room_id"].as_str().unwrap_or("?");
                let content = p["content"].as_str().unwrap_or("");
                println!("[{room}] {from}: {content}");
            }
            FrameType::AgentJoined => {
                let p = &event.frame.payload;
                let who = p["agent"]["name"].as_str().unwrap_or("?");
                let room = p["room_id"].as_str().unwrap_or("?");
                println!("  -> {who} joined {room}");
            }
            FrameType::AgentLeft => {
                let p = &event.frame.payload;
                let who = p["agent_id"].as_str().unwrap_or("?");
                let room = p["room_id"].as_str().unwrap_or("?");
                println!("  <- {who} left {room}");
            }
            _ => {} // ignore other events
        }
    }

    Ok(())
}
