//! Leader Election & Decision
//!
//! Spawns 3 agents, runs a leader election (one agent declines),
//! then the elected leader issues a binding decision.
//!
//! Elections have a 2-second opt-out window. After that, the server
//! picks randomly from remaining candidates.
//!
//! ```bash
//! # Start the server first:
//! cargo run -p cowchat-server -- serve
//!
//! # Then run this example:
//! cargo run -p cowchat-client --example leader_election
//! ```

use cowchat_client::CowchatClient;
use cowchat_core::FrameType;
use std::path::PathBuf;
use std::time::Duration;

fn read_api_key() -> String {
    let home = std::env::var("HOME").expect("HOME not set");
    let key_path = PathBuf::from(home).join(".cowchat/auth.key");
    std::fs::read_to_string(&key_path)
        .unwrap_or_else(|_| panic!("Could not read API key from {}", key_path.display()))
        .trim()
        .to_string()
}

async fn connect(key: &str, name: &str) -> CowchatClient {
    CowchatClient::connect_tcp("127.0.0.1:9229", key, name, None, vec![])
        .await
        .unwrap_or_else(|e| panic!("Failed to connect as {name}: {e}"))
}

/// Wait for a specific event type from a subscription.
async fn wait_for(
    events: &mut tokio::sync::broadcast::Receiver<cowchat_client::Event>,
    target: FrameType,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = events.recv().await {
                if event.frame.frame_type == target {
                    return event.frame.payload;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Timed out waiting for {target:?}"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = read_api_key();

    // Connect 3 agents
    println!("Connecting agents...");
    let lead = connect(&key, "lead").await;
    let dev1 = connect(&key, "dev-1").await;
    let dev2 = connect(&key, "dev-2").await;
    println!("  lead:  {}", lead.agent_id);
    println!("  dev-1: {}", dev1.agent_id);
    println!("  dev-2: {}", dev2.agent_id);

    // Create a room and join (unique name for re-runnability)
    let run_id = &uuid::Uuid::new_v4().to_string()[..6];
    let room = lead
        .create_room(
            &format!("sprint-{run_id}"),
            Some("Sprint planning session"),
            None,
            false,
        )
        .await?;
    let room_id = &room.room_id;
    println!("\nCreated room: {} ({})", room.name, room_id);

    lead.join_room(room_id).await?;
    dev1.join_room(room_id).await?;
    dev2.join_room(room_id).await?;
    println!("All agents joined\n");

    // Subscribe to events
    let mut lead_events = lead.subscribe();
    let mut dev1_events = dev1.subscribe();

    // Start the election
    println!("Starting leader election...");
    lead.elect_leader(room_id).await?;

    // Wait for ElectionStarted
    let started = wait_for(&mut lead_events, FrameType::ElectionStarted).await;
    if let Some(candidates) = started["candidates"].as_array() {
        println!(
            "  Candidates: {}",
            candidates
                .iter()
                .filter_map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "  Opt-out window: {}s",
        started["opt_out_seconds"].as_u64().unwrap_or(2)
    );

    // dev-2 declines
    dev2.decline_election(room_id).await?;
    println!("\n  dev-2 declined candidacy");
    println!("  Waiting for election to complete...\n");

    // Wait for LeaderElected
    let elected = wait_for(&mut dev1_events, FrameType::LeaderElected).await;
    let leader_id = elected["leader_id"].as_str().unwrap_or("?");
    let leader_name = elected["leader_name"].as_str().unwrap_or("?");
    println!("=== LEADER ELECTED ===");
    println!("  {leader_name} ({leader_id})\n");

    // The elected leader issues a decision
    let decision_text = "We ship the auth service this sprint using Rust";
    println!("Leader issuing decision...");

    // Figure out which client is the leader
    if leader_id == lead.agent_id {
        lead.send_decision(room_id, decision_text, serde_json::json!({}))
            .await?;
    } else if leader_id == dev1.agent_id {
        dev1.send_decision(room_id, decision_text, serde_json::json!({}))
            .await?;
    } else {
        // dev2 declined, so this shouldn't happen, but handle it
        dev2.send_decision(room_id, decision_text, serde_json::json!({}))
            .await?;
    }

    // Wait for DecisionMade event on a non-leader subscriber
    let decision = wait_for(&mut lead_events, FrameType::DecisionMade).await;
    println!("\n=== DECISION MADE ===");
    println!(
        "  By:      {}",
        decision["leader_name"].as_str().unwrap_or("?")
    );
    println!(
        "  Content: \"{}\"",
        decision["content"].as_str().unwrap_or("?")
    );

    // Show that a non-leader gets rejected
    println!("\nVerifying non-leader cannot issue decisions...");
    let non_leader = if leader_id == lead.agent_id {
        &dev1
    } else {
        &lead
    };
    match non_leader
        .send_decision(room_id, "rogue decision", serde_json::json!({}))
        .await
    {
        Err(e) => println!("  Correctly rejected: {e}"),
        Ok(_) => println!("  ERROR: non-leader decision was accepted!"),
    }

    println!("\nDone!");
    Ok(())
}
