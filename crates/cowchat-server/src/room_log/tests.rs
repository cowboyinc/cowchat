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

// Deliberately structural bytes: reducer tests do not claim signature verification.
pub(super) fn key_prepare(room: &str, epoch: u64, previous: Option<[u8; 32]>) -> Command {
    let preparation = Box::new(RoomKeyPreparation {
        transition_id: [epoch as u8 + 1; 32],
        expected_policy_hash: previous,
        policy_epoch: epoch,
        key_epoch: epoch,
        policy_hash: [epoch as u8 + 10; 32],
        expected_control_root: [epoch as u8 + 19; 32],
        signed_setup: "01".repeat(200),
        previous_policy: previous.map(|_| "06".repeat(400)),
        signed_policy: "02".repeat(400),
        custody: format!("{:02x}", epoch + 3).repeat(157),
        grants: vec!["04".repeat(178)],
    });
    Command {
        owner_id: "owner-a".into(),
        command_id: preparation.command_id(),
        timestamp: "2026-09-21T14:02:00Z".parse().unwrap(),
        body: CommandBody::PrepareKeyEpoch {
            room_id: room.into(),
            preparation,
        },
    }
}

#[test]
fn committed_custody_history_survives_serialization_replay_and_exact_retry() {
    let entries = vec![
        (0, create("one", 7)),
        (0, key_prepare("one", 0, None)),
        (0, key_cutover("one", 0, None)),
        (0, key_prepare("one", 1, Some([10; 32]))),
        (0, key_cutover("one", 1, Some([10; 32]))),
        (0, key_prepare("one", 2, Some([11; 32]))),
        (0, key_cutover("one", 2, Some([11; 32]))),
    ];
    let retry = entries.last().unwrap().clone();
    let mut live = OwnerState::new("owner-a".into());
    for (lane, command) in entries.iter().chain(std::iter::once(&retry)) {
        assert!(matches!(
            apply(&mut live, *lane, command),
            Outcome::RoomCreated { .. }
                | Outcome::KeyEpochPrepared { .. }
                | Outcome::KeyEpochCommitted { .. }
        ));
    }
    let expected = (0..3)
        .map(|epoch| RoomKeyCustody {
            key_epoch: epoch,
            custody: format!("{:02x}", epoch + 3).repeat(157),
        })
        .collect::<Vec<_>>();
    assert_eq!(live.room("one").unwrap().key_custodies, expected);

    let live: OwnerState = serde_json::from_slice(&serde_json::to_vec(&live).unwrap()).unwrap();
    assert_eq!(live.room("one").unwrap().key_custodies, expected);

    let mut rebuilt = OwnerState::new("owner-a".into());
    for (lane, command) in entries.into_iter().chain(std::iter::once(retry)) {
        let command: Command =
            serde_json::from_slice(&serde_json::to_vec(&command).unwrap()).unwrap();
        apply(&mut rebuilt, lane, &command);
    }
    assert_eq!(rebuilt.room("one").unwrap().key_custodies, expected);
    assert_eq!(
        serde_json::to_value(rebuilt).unwrap(),
        serde_json::to_value(live).unwrap()
    );
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
        (0, key_prepare("one", 0, None)),
        (0, key_cutover("one", 0, None)),
        (7, keyed_message("before", "one", 0)),
        (0, key_prepare("one", 1, Some([10; 32]))),
        (0, key_cutover("one", 1, Some([10; 32]))),
    ];
    let mut state = OwnerState::new("owner-a".into());
    for (lane, command) in &entries[..5] {
        apply(&mut state, *lane, command);
    }
    assert_eq!(
        apply(&mut state, 7, &keyed_message("during", "one", 0)),
        Outcome::Rejected {
            reason: Rejection::KeyTransitionPending
        }
    );
    apply(&mut state, entries[5].0, &entries[5].1);
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
        apply(&mut state, 7, &entries[3].1),
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
    apply(&mut state, 0, &key_prepare("one", 0, None));
    assert!(matches!(
        apply(&mut state, 0, &initial),
        Outcome::KeyEpochCommitted { key_epoch: 0, .. }
    ));
    assert!(state.room("one").unwrap().key_preparation.is_none());
    assert_eq!(
        state
            .room("one")
            .unwrap()
            .key_publication
            .as_ref()
            .unwrap()
            .key_epoch,
        0
    );
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
fn preparation_blocks_only_fresh_sends_and_cannot_be_replaced() {
    let mut state = OwnerState::new("owner-a".into());
    apply(&mut state, 0, &create("one", 7));
    apply(&mut state, 0, &create("two", 8));
    let accepted = message("before-setup", "one");
    apply(&mut state, 7, &accepted);
    let preparation = key_prepare("one", 0, None);
    assert_eq!(state.apply(4, 7, &preparation), Err(ReplayError::Lane));
    assert!(matches!(
        apply(&mut state, 0, &preparation),
        Outcome::KeyEpochPrepared { .. }
    ));
    assert!(state.room("one").unwrap().key_state.is_none());
    assert!(matches!(
        apply(&mut state, 7, &accepted),
        Outcome::MessageAppended { sequence: 1, .. }
    ));
    assert_eq!(
        apply(&mut state, 7, &message("during-setup", "one")),
        Outcome::Rejected {
            reason: Rejection::KeyTransitionPending
        }
    );
    assert!(matches!(
        apply(&mut state, 8, &message("other-room", "two")),
        Outcome::MessageAppended { .. }
    ));
    assert!(matches!(
        apply(&mut state, 0, &preparation),
        Outcome::KeyEpochPrepared { .. }
    ));
    let mut changed = preparation.clone();
    if let CommandBody::PrepareKeyEpoch { preparation, .. } = &mut changed.body {
        preparation.custody = "05".repeat(157);
    }
    assert_eq!(
        apply(&mut state, 0, &changed),
        Outcome::Rejected {
            reason: Rejection::CommandConflict
        }
    );
    assert_eq!(
        apply(&mut state, 0, &key_prepare("one", 1, Some([10; 32]))),
        Outcome::Rejected {
            reason: Rejection::KeyTransitionPending
        }
    );
    assert!(matches!(
        apply(&mut state, 0, &key_cutover("one", 0, None)),
        Outcome::KeyEpochCommitted { .. }
    ));
    assert!(state.room("one").unwrap().key_preparation.is_none());
    assert!(matches!(
        apply(&mut state, 7, &keyed_message("after-setup", "one", 0)),
        Outcome::MessageAppended { sequence: 2, .. }
    ));
}

#[test]
fn older_preparation_without_predecessor_bytes_still_pauses_on_replay() {
    let mut state = OwnerState::new("owner-a".into());
    apply(&mut state, 0, &create("one", 7));
    apply(&mut state, 0, &key_prepare("one", 0, None));
    apply(&mut state, 0, &key_cutover("one", 0, None));
    let mut encoded = serde_json::to_value(key_prepare("one", 1, Some([10; 32]))).unwrap();
    encoded["body"]["preparation"]
        .as_object_mut()
        .unwrap()
        .remove("previous_policy");
    let old: Command = serde_json::from_value(encoded).unwrap();
    assert!(matches!(
        apply(&mut state, 0, &old),
        Outcome::KeyEpochPrepared { .. }
    ));
    assert_eq!(
        apply(&mut state, 7, &keyed_message("during", "one", 0)),
        Outcome::Rejected {
            reason: Rejection::KeyTransitionPending
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
fn preparation_rejects_invalid_predecessors_bytes_and_oversized_records() {
    for case in 0..10 {
        let mut state = OwnerState::new("owner-a".into());
        apply(&mut state, 0, &create("one", 7));
        let mut command = key_prepare("one", 0, None);
        if let CommandBody::PrepareKeyEpoch { preparation, .. } = &mut command.body {
            match case {
                0 => preparation.expected_policy_hash = Some([1; 32]),
                1 => preparation.key_epoch = 1,
                2 => preparation.transition_id = [0; 32],
                3 => preparation.expected_control_root = [0; 32],
                4 => preparation.signed_setup = "0A".into(),
                5 => preparation.signed_policy = "0".into(),
                6 => preparation.custody.pop().map(|_| ()).unwrap(),
                7 => preparation.grants.push("ff".into()),
                8 => preparation.grants = vec!["04".repeat(178); 1024],
                9 => preparation.signed_setup = "01".repeat(32 * 1024 + 1),
                _ => unreachable!(),
            }
            command.command_id = preparation.command_id();
        }
        assert_eq!(
            apply(&mut state, 0, &command),
            Outcome::Rejected {
                reason: Rejection::InvalidKeyTransition
            },
            "case {case}"
        );
        assert!(state.room("one").unwrap().key_preparation.is_none());
    }
}

#[test]
fn cutover_requires_preparation_and_exact_prepared_fields() {
    for case in 0..7 {
        let mut state = OwnerState::new("owner-a".into());
        apply(&mut state, 0, &create("one", 7));
        if case != 0 {
            apply(&mut state, 0, &key_prepare("one", 0, None));
        }
        let mut command = key_cutover("one", 0, None);
        if let CommandBody::CommitKeyEpoch {
            expected_policy_hash,
            state,
            ..
        } = &mut command.body
        {
            match case {
                0 => {}
                1 => *expected_policy_hash = Some([1; 32]),
                2 => state.policy_hash = [99; 32],
                3 => state.policy_epoch = 1,
                4 => state.key_epoch = 1,
                5 => state.control_root = [19; 32],
                6 => state.control_root = [0; 32],
                _ => unreachable!(),
            }
        }
        assert_eq!(
            apply(&mut state, 0, &command),
            Outcome::Rejected {
                reason: Rejection::InvalidKeyTransition
            },
            "case {case}"
        );
        assert!(state.room("one").unwrap().key_state.is_none());
        assert_eq!(
            state.room("one").unwrap().key_preparation.is_some(),
            case != 0
        );
    }
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
