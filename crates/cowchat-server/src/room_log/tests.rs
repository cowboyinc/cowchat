use super::*;

pub(super) fn create(id: &str, lane: u64) -> Command {
    Command {
        owner_id: "owner-a".into(),
        command_id: format!("create-{id}"),
        timestamp: "2026-09-21T14:00:00Z".parse().unwrap(),
        body: CommandBody::CreateRoom {
            room_id: id.into(),
            lane_id: lane,
            name: id.into(),
            created_by: "alice".into(),
        },
    }
}
pub(super) fn message(id: &str, room: &str) -> Command {
    Command {
        owner_id: "owner-a".into(),
        command_id: id.into(),
        timestamp: "2026-09-21T14:01:00Z".parse().unwrap(),
        body: CommandBody::AppendMessage {
            room_id: room.into(),
            agent_id: "alice".into(),
            agent_name: "Alice".into(),
            ciphertext: "cow1:opaque-ciphertext".into(),
            reply_to: None,
            metadata: serde_json::json!({}),
            mentions: vec!["actor-b".into()],
        },
    }
}
fn apply(state: &mut OwnerState, lane: u64, command: &Command) -> Outcome {
    state
        .apply(state.applied_through() + 1, lane, command)
        .unwrap()
}

#[test]
fn replay_rebuilds_two_rooms_and_exact_retry_results_without_time_or_randomness() {
    let entries = vec![
        (0, create("one", 7)),
        (0, create("two", 8)),
        (7, message("a", "one")),
        (8, message("b", "two")),
    ];
    let mut live = OwnerState::new("owner-a".into());
    for (lane, command) in &entries {
        apply(&mut live, *lane, command);
    }
    let mut retry = entries[2].1.clone();
    retry.timestamp = "2026-10-01T00:00:00Z".parse().unwrap();
    if let CommandBody::AppendMessage { agent_name, .. } = &mut retry.body {
        *agent_name = "Renamed Alice".into();
    }
    assert_eq!(
        apply(&mut live, 7, &retry),
        Outcome::MessageAppended {
            room_id: "one".into(),
            message_id: "a".into(),
            sequence: 1
        }
    );
    assert_eq!(live.room("one").unwrap().messages.len(), 1);
    assert_eq!(
        live.room("one").unwrap().messages[0].message.agent_name,
        "Alice"
    );
    let mut rebuilt = OwnerState::new("owner-a".into());
    for (lane, command) in entries.into_iter().chain([(7, retry)]) {
        let wire = serde_json::to_vec(&command).unwrap();
        apply(&mut rebuilt, lane, &serde_json::from_slice(&wire).unwrap());
    }
    assert_eq!(
        serde_json::to_value(live).unwrap(),
        serde_json::to_value(rebuilt).unwrap()
    );
}

#[test]
fn gaps_wrong_owner_and_wrong_lane_stop_without_advancing_projection() {
    let mut state = OwnerState::new("owner-a".into());
    assert_eq!(
        state.apply(2, 0, &create("one", 7)),
        Err(ReplayError::Sequence)
    );
    let mut other = create("one", 7);
    other.owner_id = "owner-b".into();
    assert_eq!(state.apply(1, 0, &other), Err(ReplayError::Owner));
    apply(&mut state, 0, &create("one", 7));
    assert_eq!(
        state.apply(2, 8, &message("a", "one")),
        Err(ReplayError::Lane)
    );
    assert_eq!(state.applied_through(), 1);
    assert!(state.room("one").unwrap().messages.is_empty());
}

#[test]
fn changed_ciphertext_or_routing_conflicts_and_never_adds_another_message() {
    let mut state = OwnerState::new("owner-a".into());
    apply(&mut state, 0, &create("one", 7));
    apply(&mut state, 0, &create("two", 8));
    apply(&mut state, 7, &message("a", "one"));
    let mut changed = message("a", "one");
    if let CommandBody::AppendMessage { ciphertext, .. } = &mut changed.body {
        *ciphertext = "cow1:different".into();
    }
    assert_eq!(
        apply(&mut state, 7, &changed),
        Outcome::Rejected {
            reason: Rejection::CommandConflict
        }
    );
    assert_eq!(
        apply(&mut state, 8, &message("a", "two")),
        Outcome::Rejected {
            reason: Rejection::CommandConflict
        }
    );
    assert_eq!(state.room("one").unwrap().messages.len(), 1);
    assert!(state.room("two").unwrap().messages.is_empty());
}

#[test]
fn plaintext_and_cross_room_replies_are_rejected_in_log_order() {
    let mut state = OwnerState::new("owner-a".into());
    apply(&mut state, 0, &create("one", 7));
    apply(&mut state, 0, &create("two", 8));
    let mut plain = message("plain", "one");
    if let CommandBody::AppendMessage { ciphertext, .. } = &mut plain.body {
        *ciphertext = "secret plaintext".into();
    }
    assert_eq!(
        apply(&mut state, 7, &plain),
        Outcome::Rejected {
            reason: Rejection::Plaintext
        }
    );
    apply(&mut state, 7, &message("a", "one"));
    let mut reply = message("b", "two");
    if let CommandBody::AppendMessage { reply_to, .. } = &mut reply.body {
        *reply_to = Some("a".into());
    }
    assert_eq!(
        apply(&mut state, 8, &reply),
        Outcome::Rejected {
            reason: Rejection::UnknownReply
        }
    );
    assert_eq!(state.applied_through(), 5);
    assert_eq!(state.room("one").unwrap().messages[0].message.seq, 1);
    assert!(state.room("two").unwrap().messages.is_empty());
}

#[test]
fn competing_room_names_and_lane_bindings_have_one_winner() {
    let mut state = OwnerState::new("owner-a".into());
    apply(&mut state, 0, &create("one", 7));
    assert_eq!(
        apply(&mut state, 0, &create("two", 7)),
        Outcome::Rejected {
            reason: Rejection::LaneAlreadyAssigned
        }
    );
    let mut same_name = create("three", 9);
    if let CommandBody::CreateRoom { name, .. } = &mut same_name.body {
        *name = "one".into();
    }
    assert_eq!(
        apply(&mut state, 0, &same_name),
        Outcome::Rejected {
            reason: Rejection::RoomNameTaken
        }
    );
    assert_eq!(state.rooms().len(), 1);
}
