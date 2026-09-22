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
            key_epoch: None,
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

pub(super) fn key_cutover(room: &str, epoch: u64, previous: Option<[u8; 32]>) -> Command {
    let transition_id = [epoch as u8 + 1; 32];
    Command {
        owner_id: "owner-a".into(),
        command_id: transition_id.iter().map(|b| format!("{b:02x}")).collect(),
        timestamp: "2026-09-21T14:02:00Z".parse().unwrap(),
        body: CommandBody::CommitKeyEpoch {
            room_id: room.into(),
            expected_policy_hash: previous,
            state: RoomKeyState {
                transition_id,
                policy_epoch: epoch,
                key_epoch: epoch,
                policy_hash: [epoch as u8 + 10; 32],
                control_root: [epoch as u8 + 20; 32],
            },
        },
    }
}

pub(super) fn keyed_message(id: &str, room: &str, epoch: u64) -> Command {
    let mut command = message(id, room);
    if let CommandBody::AppendMessage { key_epoch, .. } = &mut command.body {
        *key_epoch = Some(epoch);
    }
    command
}

#[test]
fn key_cutover_orders_old_receipts_fresh_sends_and_replay() {
    let entries = vec![
        (0, create("one", 7)),
        (0, key_cutover("one", 0, None)),
        (7, keyed_message("before", "one", 0)),
        (0, key_cutover("one", 1, Some([10; 32]))),
    ];
    let mut state = OwnerState::new("owner-a".into());
    for (lane, command) in &entries {
        apply(&mut state, *lane, command);
    }
    assert_eq!(
        state
            .room("one")
            .unwrap()
            .key_state
            .as_ref()
            .unwrap()
            .key_epoch,
        1
    );
    // A previously accepted message keeps its receipt after removal/rotation.
    assert!(matches!(
        apply(&mut state, 7, &entries[2].1),
        Outcome::MessageAppended { sequence: 1, .. }
    ));
    for command in [
        keyed_message("fresh-old", "one", 0),
        message("missing-epoch", "one"),
        keyed_message("future", "one", 2),
    ] {
        assert_eq!(
            apply(&mut state, 7, &command),
            Outcome::Rejected {
                reason: Rejection::KeyEpochMismatch
            }
        );
    }
    assert!(matches!(
        apply(&mut state, 7, &keyed_message("after", "one", 1)),
        Outcome::MessageAppended { sequence: 2, .. }
    ));
    assert_eq!(
        state.room("one").unwrap().messages[0]
            .message
            .key_epoch
            .as_deref(),
        Some("0")
    );
    assert_eq!(
        state.room("one").unwrap().messages[1]
            .message
            .key_epoch
            .as_deref(),
        Some("1")
    );
    // Relabelling the same ciphertext/id is a conflict, not a retry.
    assert_eq!(
        apply(&mut state, 7, &keyed_message("before", "one", 1)),
        Outcome::Rejected {
            reason: Rejection::CommandConflict
        }
    );
    let mut rebuilt = OwnerState::new("owner-a".into());
    for (lane, command) in entries {
        let command: Command =
            serde_json::from_slice(&serde_json::to_vec(&command).unwrap()).unwrap();
        apply(&mut rebuilt, lane, &command);
    }
    assert_eq!(
        rebuilt.room("one").unwrap().key_state,
        state.room("one").unwrap().key_state
    );
    assert!(matches!(
        apply(&mut rebuilt, 7, &keyed_message("before", "one", 0)),
        Outcome::MessageAppended { sequence: 1, .. }
    ));
}

#[test]
fn key_cutover_requires_monotone_exact_predecessor_and_stable_transition_id() {
    let mut state = OwnerState::new("owner-a".into());
    apply(&mut state, 0, &create("one", 7));
    let initial = key_cutover("one", 0, None);
    assert_eq!(state.apply(2, 7, &initial), Err(ReplayError::Lane));
    let mut bad = initial.clone();
    bad.command_id = "different-transition".into();
    assert_eq!(
        apply(&mut state, 0, &bad),
        Outcome::Rejected {
            reason: Rejection::InvalidKeyTransition
        }
    );
    assert!(matches!(
        apply(&mut state, 0, &initial),
        Outcome::KeyEpochCommitted { key_epoch: 0, .. }
    ));
    let wrong = key_cutover("one", 1, Some([99; 32]));
    assert_eq!(
        apply(&mut state, 0, &wrong),
        Outcome::Rejected {
            reason: Rejection::InvalidKeyTransition
        }
    );
    // A rejected stable command ID cannot be recycled with a different body.
    assert_eq!(
        apply(&mut state, 0, &key_cutover("one", 1, Some([10; 32]))),
        Outcome::Rejected {
            reason: Rejection::CommandConflict
        }
    );
    assert_eq!(
        state
            .room("one")
            .unwrap()
            .key_state
            .as_ref()
            .unwrap()
            .key_epoch,
        0
    );
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
