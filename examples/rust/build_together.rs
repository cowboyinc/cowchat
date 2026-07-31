//! Build Together — 3 Agents Coordinate to Build Tic-Tac-Toe
//!
//! The capstone Cowchat example: a coordinator, a server-dev, and a
//! client-dev agent work together to "build" a tic-tac-toe game.
//!
//! Demonstrates ALL coordination features: rooms, sub-rooms, messaging,
//! sealed-ballot voting, leader election, and decisions.
//!
//! ```bash
//! # Start the server first:
//! cargo run -p cowchat-server -- serve
//!
//! # Then run this example:
//! cargo run -p cowchat-client --example build_together
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

    // ========================================================
    println!("\n=== PHASE 1: SETUP & PLANNING ===\n");
    // ========================================================

    // Connect 3 agents
    let coordinator = connect(&key, "coordinator").await;
    let server_dev = connect(&key, "server-dev").await;
    let client_dev = connect(&key, "client-dev").await;
    println!("  Connected 3 agents:");
    println!("    coordinator: {}", coordinator.agent_id);
    println!("    server-dev:  {}", server_dev.agent_id);
    println!("    client-dev:  {}", client_dev.agent_id);

    // Coordinator creates the project room (unique name for re-runnability)
    let run_id = &uuid::Uuid::new_v4().to_string()[..6];
    let room_name = format!("tictactoe-{run_id}");
    let project = coordinator
        .create_room(&room_name, Some("Tic-tac-toe game project"), None, false)
        .await?;
    let project_id = project.room_id.clone();
    println!("\n  Created project room: {room_name} ({project_id})");

    // Everyone joins
    coordinator.join_room(&project_id).await?;
    server_dev.join_room(&project_id).await?;
    client_dev.join_room(&project_id).await?;
    println!("  All agents joined\n");

    // Coordinator kicks things off
    coordinator
        .send_message(
            &project_id,
            "Let's build a tic-tac-toe game. We need a TCP server and a client.",
            None,
            vec![],
        )
        .await?;
    println!("  coordinator: Let's build a tic-tac-toe game.");

    // Create ephemeral sub-rooms for focused work
    let server_room = coordinator
        .create_room(
            &format!("ttt-server-{run_id}"),
            None,
            Some(&project_id),
            true,
        )
        .await?;
    let client_room = coordinator
        .create_room(
            &format!("ttt-client-{run_id}"),
            None,
            Some(&project_id),
            true,
        )
        .await?;
    println!(
        "  Created sub-rooms: ttt-server-{run_id} ({}), ttt-client-{run_id} ({})",
        server_room.room_id, client_room.room_id
    );

    // Devs join their respective sub-rooms
    server_dev.join_room(&server_room.room_id).await?;
    client_dev.join_room(&client_room.room_id).await?;
    println!("  server-dev -> ttt-server, client-dev -> ttt-client");

    // ========================================================
    println!("\n=== PHASE 2: VOTE ON PROTOCOL ===\n");
    // ========================================================

    // Subscribe to events before creating the vote
    let mut coord_events = coordinator.subscribe();

    // Coordinator creates a sealed-ballot vote
    let vote = coordinator
        .create_vote(
            &project_id,
            "Wire protocol format?",
            Some("How should the server and client talk to each other?"),
            vec![
                "JSON".to_string(),
                "plain text".to_string(),
                "binary".to_string(),
            ],
            None,
        )
        .await?;
    println!("  Vote: \"Wire protocol format?\"");
    println!("  Options: JSON | plain text | binary");
    println!("  Eligible: {}\n", vote.eligible_voters);

    // Cast sealed ballots
    println!("  Casting sealed ballots...");
    coordinator.cast_vote(&vote.vote_id, 0).await?; // JSON
    println!("    coordinator -> (sealed)");
    server_dev.cast_vote(&vote.vote_id, 0).await?; // JSON
    println!("    server-dev  -> (sealed)");
    client_dev.cast_vote(&vote.vote_id, 1).await?; // plain text
    println!("    client-dev  -> (sealed)");

    // Wait for results
    let result = wait_for(&mut coord_events, FrameType::VoteResult).await;
    println!("\n  Results revealed:");
    if let Some(tally) = result["tally"].as_array() {
        for entry in tally {
            let option = entry["option_text"].as_str().unwrap_or("?");
            let count = entry["count"].as_u64().unwrap_or(0);
            let bar = "#".repeat(count as usize * 4);
            println!("    {option:<12} {bar} ({count})");
        }
    }
    if let Some(ballots) = result["ballots"].as_array() {
        println!();
        for ballot in ballots {
            let name = ballot["agent_name"].as_str().unwrap_or("?");
            let idx = ballot["option_index"].as_u64().unwrap_or(0);
            let options = ["JSON", "plain text", "binary"];
            let choice = options.get(idx as usize).unwrap_or(&"?");
            println!("    {name:<14} voted {choice}");
        }
    }

    // Post the result to the room
    coordinator
        .send_message(
            &project_id,
            "Vote result: JSON wins! We'll use JSON.",
            None,
            vec![],
        )
        .await?;

    // ========================================================
    println!("\n=== PHASE 3: ELECT TECH LEAD ===\n");
    // ========================================================

    let mut sdev_events = server_dev.subscribe();

    coordinator.elect_leader(&project_id).await?;
    println!("  Election started (2s opt-out window)...");

    // Wait for election result
    let elected = wait_for(&mut sdev_events, FrameType::LeaderElected).await;
    let leader_id = elected["leader_id"].as_str().unwrap_or("?").to_string();
    let leader_name = elected["leader_name"].as_str().unwrap_or("?");
    println!("  Leader elected: {leader_name}");

    // Leader issues the protocol decision
    let protocol_spec = r#"JSON protocol decided: {"action":"move","pos":0-8} for moves, {"state":"board","cells":["X","O"," ",...]} for state updates"#;

    // Find which client is the leader and issue the decision
    let leader_client = if leader_id == coordinator.agent_id {
        &coordinator
    } else if leader_id == server_dev.agent_id {
        &server_dev
    } else {
        &client_dev
    };
    leader_client
        .send_decision(&project_id, protocol_spec, serde_json::json!({}))
        .await?;

    let decision = wait_for(&mut coord_events, FrameType::DecisionMade).await;
    println!(
        "\n  Decision by {}: \"{}\"",
        decision["leader_name"].as_str().unwrap_or("?"),
        &decision["content"].as_str().unwrap_or("?")[..60],
    );

    // ========================================================
    println!("\n=== PHASE 4: BUILD ===\n");
    // ========================================================

    // Server dev works in their sub-room
    server_dev
        .send_message(
            &server_room.room_id,
            "Building game server... TCP listener, move validation, win detection",
            None,
            vec![],
        )
        .await?;
    println!("  [ttt-server] server-dev: Building game server...");

    // Client dev works in their sub-room
    client_dev
        .send_message(
            &client_room.room_id,
            "Building game client... board renderer, input parser, TCP connector",
            None,
            vec![],
        )
        .await?;
    println!("  [ttt-client] client-dev: Building game client...");

    // Simulate build time
    tokio::time::sleep(Duration::from_millis(500)).await;
    print!("  Building");
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        print!(".");
    }
    println!();

    // Server done
    server_dev
        .send_message(
            &server_room.room_id,
            "Server done! Listening on port 3000. Supports 2-player matches.",
            None,
            vec![],
        )
        .await?;
    println!("\n  [ttt-server] server-dev: Server done! Listening on port 3000");

    // Client done
    client_dev
        .send_message(
            &client_room.room_id,
            "Client done! Connects to server:3000, renders board in terminal.",
            None,
            vec![],
        )
        .await?;
    println!("  [ttt-client] client-dev: Client done! Connects to server:3000");

    // Report back to main room
    server_dev
        .send_message(&project_id, "Server component ready!", None, vec![])
        .await?;
    client_dev
        .send_message(&project_id, "Client component ready!", None, vec![])
        .await?;
    println!("\n  Both devs reported ready in main room");

    // ========================================================
    println!("\n=== PHASE 5: INTEGRATION ===\n");
    // ========================================================

    coordinator
        .send_message(
            &project_id,
            "All components ready. Tic-tac-toe is shipped! Great work team.",
            None,
            vec![],
        )
        .await?;
    println!("  coordinator: All components ready. Ship it!");

    // Fetch and print the full project history
    let history = coordinator.get_history(&project_id, 50, None).await?;

    println!("\n=== PROJECT HISTORY ({} messages) ===\n", history.len());
    for msg in &history {
        let name = &msg.agent_name;
        let content = &msg.content;
        // Check if it's a decision (has metadata with type)
        let is_decision = msg
            .metadata
            .get("type")
            .and_then(|v| v.as_str())
            .map(|t| t == "decision")
            .unwrap_or(false);

        if is_decision {
            println!("  [DECISION] {name}: {content}");
        } else {
            println!("  {name}: {content}");
        }
    }

    // Clean up — leave ephemeral sub-rooms (they auto-destruct)
    server_dev.leave_room(&server_room.room_id).await?;
    client_dev.leave_room(&client_room.room_id).await?;
    coordinator.leave_room(&server_room.room_id).await.ok(); // coordinator may not have joined
    coordinator.leave_room(&client_room.room_id).await.ok();
    println!("\n  Ephemeral sub-rooms cleaned up");

    println!("\nDone!");
    Ok(())
}
