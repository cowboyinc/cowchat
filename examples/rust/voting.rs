//! Sealed-Ballot Voting
//!
//! Spawns 3 agents in-process, creates a sealed vote, has each
//! cast a ballot, then prints the revealed results.
//!
//! Votes are sealed: nobody sees anyone's choice until all ballots
//! are in (or a deadline expires). This prevents anchoring bias.
//!
//! ```bash
//! # Start the server first:
//! cargo run -p cowchat-server -- serve
//!
//! # Then run this example:
//! cargo run -p cowchat-client --example voting
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = read_api_key();

    // Connect 3 agents
    println!("Connecting agents...");
    let alice = connect(&key, "alice").await;
    let bob = connect(&key, "bob").await;
    let charlie = connect(&key, "charlie").await;
    println!("  alice:   {}", alice.agent_id);
    println!("  bob:     {}", bob.agent_id);
    println!("  charlie: {}", charlie.agent_id);

    // Create a room and have everyone join (unique name for re-runnability)
    let run_id = &uuid::Uuid::new_v4().to_string()[..6];
    let room = alice
        .create_room(
            &format!("lang-vote-{run_id}"),
            Some("Vote on language choice"),
            None,
        )
        .await?;
    let room_id = &room.room_id;
    println!("\nCreated room: {} ({})", room.name, room_id);

    alice.join_room(room_id).await?;
    bob.join_room(room_id).await?;
    charlie.join_room(room_id).await?;
    println!("All agents joined\n");

    // Subscribe to events before creating the vote
    let mut alice_events = alice.subscribe();

    // Alice creates a sealed-ballot vote
    let vote = alice
        .create_vote(
            room_id,
            "Which language for the new service?",
            Some("Pick one -- ballots are sealed until everyone votes"),
            vec!["Rust".to_string(), "Go".to_string(), "Python".to_string()],
            None, // no deadline, closes when all vote
        )
        .await?;
    println!("Vote created: \"{}\"", vote.title);
    println!("  Options: Rust | Go | Python");
    println!("  Eligible voters: {}\n", vote.eligible_voters);

    // Each agent casts a sealed ballot
    println!("Casting sealed ballots...");
    alice.cast_vote(&vote.vote_id, 0).await?; // Rust
    println!("  alice   voted (sealed)");
    bob.cast_vote(&vote.vote_id, 0).await?; // Rust
    println!("  bob     voted (sealed)");
    charlie.cast_vote(&vote.vote_id, 1).await?; // Go
    println!("  charlie voted (sealed)");

    // Wait for VoteResult event (triggered when all ballots are in)
    println!("\nWaiting for results...\n");
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = alice_events.recv().await {
                if event.frame.frame_type == FrameType::VoteResult {
                    return event.frame.payload;
                }
            }
        }
    })
    .await?;

    // Print the results
    println!("=== VOTE RESULTS ===");
    println!("\"{}\"", result["title"].as_str().unwrap_or("?"));
    println!();

    if let Some(tally) = result["tally"].as_array() {
        for entry in tally {
            let option = entry["option_text"].as_str().unwrap_or("?");
            let count = entry["count"].as_u64().unwrap_or(0);
            let bar = "#".repeat(count as usize * 5);
            println!("  {option:<8} {bar} ({count})");
        }
    }

    println!();
    if let Some(ballots) = result["ballots"].as_array() {
        println!("Individual ballots (now revealed):");
        for ballot in ballots {
            let name = ballot["agent_name"].as_str().unwrap_or("?");
            let idx = ballot["option_index"].as_u64().unwrap_or(0);
            let options = ["Rust", "Go", "Python"];
            let choice = options.get(idx as usize).unwrap_or(&"?");
            println!("  {name:<10} -> {choice}");
        }
    }

    println!(
        "\nTotal: {}/{}",
        result["total_votes"].as_u64().unwrap_or(0),
        result["eligible_voters"].as_u64().unwrap_or(0)
    );

    Ok(())
}
