use cowchat_client::CowchatClient;
use cowchat_core::{
    ChatMessage, HandoffAcceptancePacket, HandoffPacket, HANDOFF_ACCEPTED_KIND, HANDOFF_READY_KIND,
    HANDOFF_SCHEMA_VERSION,
};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Serialize)]
struct HandoffListOutput {
    handoffs: Vec<HandoffListItem>,
}

#[derive(Debug, Serialize)]
struct HandoffListItem {
    message_id: String,
    seq: i64,
    agent_id: String,
    agent_name: String,
    timestamp: String,
    reply_to_message: Option<String>,
    kind: String,
    handoff: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
}

pub(crate) struct HandoffDraft<'a> {
    pub task_id: &'a str,
    pub revision: &'a str,
    pub supersedes: Option<&'a str>,
    pub summary: &'a str,
    pub next: &'a str,
    pub risks: &'a [String],
    pub refs: &'a [String],
}

pub(crate) async fn send(
    client: &CowchatClient,
    room_id: &str,
    draft: HandoffDraft<'_>,
) -> Result<ChatMessage, Box<dyn std::error::Error>> {
    let packet = HandoffPacket {
        version: HANDOFF_SCHEMA_VERSION,
        task_id: draft.task_id.trim().to_string(),
        revision: draft.revision.trim().to_string(),
        supersedes: draft.supersedes.map(str::trim).map(str::to_string),
        summary: draft.summary.trim().to_string(),
        next: draft.next.trim().to_string(),
        risks: draft
            .risks
            .iter()
            .map(|risk| risk.trim().to_string())
            .collect(),
        refs: draft
            .refs
            .iter()
            .map(|reference| reference.trim().to_string())
            .collect(),
    };
    packet.validate()?;
    let content = render_ready(&packet);
    Ok(client
        .send_message_with_metadata(
            room_id,
            &content,
            None,
            vec![],
            serde_json::json!({"kind": HANDOFF_READY_KIND, "handoff": packet}),
        )
        .await?)
}

pub(crate) async fn list(
    client: &CowchatClient,
    room_id: &str,
    limit: u32,
    json: bool,
    pending: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let messages = client.get_history(room_id, limit, None).await?;
    let mut handoffs: Vec<_> = messages.iter().filter_map(parse_list_item).collect();
    let accepted: HashSet<_> = handoffs
        .iter()
        .filter(|item| item.kind == HANDOFF_ACCEPTED_KIND)
        .filter_map(|item| {
            serde_json::from_value::<HandoffAcceptancePacket>(item.handoff.clone())
                .ok()
                .map(|packet| packet.accepted_handoff_id)
        })
        .collect();
    let superseded: HashSet<_> = handoffs
        .iter()
        .filter(|item| item.kind == HANDOFF_READY_KIND)
        .filter_map(|item| {
            serde_json::from_value::<HandoffPacket>(item.handoff.clone())
                .ok()
                .and_then(|packet| packet.supersedes)
        })
        .collect();
    for item in &mut handoffs {
        if item.kind == HANDOFF_READY_KIND {
            item.state = Some(
                if accepted.contains(&item.message_id) {
                    "accepted"
                } else if superseded.contains(&item.message_id) {
                    "superseded"
                } else {
                    "pending"
                }
                .to_string(),
            );
        }
    }
    if pending {
        handoffs.retain(|item| {
            item.kind == HANDOFF_READY_KIND && item.state.as_deref() == Some("pending")
        });
    }
    let output = HandoffListOutput { handoffs };
    if json {
        return Ok(format!("{}\n", serde_json::to_string(&output)?));
    }

    if output.handoffs.is_empty() {
        return Ok("No handoffs found.\n".to_string());
    }

    let mut lines = Vec::with_capacity(output.handoffs.len());
    for item in output.handoffs {
        let detail = serde_json::from_value::<HandoffPacket>(item.handoff.clone())
            .map(|packet| {
                format!(
                    "{}@{} {} → {}",
                    packet.task_id, packet.revision, packet.summary, packet.next
                )
            })
            .unwrap_or_else(|_| "accepted".to_string());
        lines.push(format!(
            "#{} {}{} {}: {} ({})",
            item.seq,
            item.kind,
            item.state
                .as_deref()
                .map(|state| format!(" [{state}]"))
                .unwrap_or_default(),
            item.agent_name,
            detail,
            item.message_id
        ));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

pub(crate) async fn accept(
    client: &CowchatClient,
    room_id: &str,
    handoff_message_id: &str,
    note: Option<&str>,
) -> Result<ChatMessage, Box<dyn std::error::Error>> {
    let note = note.map(str::trim);
    let acceptance = HandoffAcceptancePacket {
        version: HANDOFF_SCHEMA_VERSION,
        accepted_handoff_id: handoff_message_id.trim().to_string(),
        note: note.map(str::to_string),
    };
    acceptance.validate()?;
    Ok(client
        .accept_handoff(room_id, handoff_message_id, note)
        .await?)
}

fn parse_list_item(message: &ChatMessage) -> Option<HandoffListItem> {
    let kind = message
        .metadata
        .get("kind")
        .and_then(|value| value.as_str())?;
    let handoff = message
        .metadata
        .get("handoff")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let valid = match kind {
        HANDOFF_READY_KIND => is_valid_ready_packet(&handoff),
        HANDOFF_ACCEPTED_KIND => is_valid_acceptance_packet(message, &handoff),
        _ => false,
    };
    if !valid {
        return None;
    }
    Some(HandoffListItem {
        message_id: message.message_id.clone(),
        seq: message.seq,
        agent_id: message.agent_id.clone(),
        agent_name: message.agent_name.clone(),
        timestamp: message.timestamp.to_rfc3339(),
        reply_to_message: message.reply_to_message.clone(),
        kind: kind.to_string(),
        handoff,
        state: None,
    })
}

fn is_valid_ready_packet(value: &serde_json::Value) -> bool {
    serde_json::from_value::<HandoffPacket>(value.clone())
        .ok()
        .is_some_and(|packet| packet.validate().is_ok())
}

fn is_valid_acceptance_packet(message: &ChatMessage, value: &serde_json::Value) -> bool {
    serde_json::from_value::<HandoffAcceptancePacket>(value.clone())
        .ok()
        .is_some_and(|packet| {
            packet.validate().is_ok()
                && message.reply_to_message.as_deref() == Some(&packet.accepted_handoff_id)
        })
}

fn render_ready(packet: &HandoffPacket) -> String {
    let mut lines = vec![
        "Handoff ready".to_string(),
        String::new(),
        format!("Task: {}", packet.task_id),
        format!("Revision: {}", packet.revision),
        format!("Summary: {}", packet.summary),
        format!("Next: {}", packet.next),
    ];
    if let Some(supersedes) = packet.supersedes.as_deref() {
        lines.push(format!("Supersedes: {supersedes}"));
    }
    if !packet.risks.is_empty() {
        lines.push(String::new());
        lines.push("Risks:".to_string());
        lines.extend(packet.risks.iter().map(|risk| format!("- {risk}")));
    }
    if !packet.refs.is_empty() {
        lines.push(String::new());
        lines.push("References:".to_string());
        lines.extend(packet.refs.iter().map(|reference| format!("- {reference}")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{is_valid_ready_packet, render_ready, HandoffPacket, HANDOFF_SCHEMA_VERSION};

    fn packet() -> HandoffPacket {
        HandoffPacket {
            version: HANDOFF_SCHEMA_VERSION,
            task_id: "AUTH-118".to_string(),
            revision: "r2".to_string(),
            supersedes: Some("prior-message".to_string()),
            summary: "Auth change complete".to_string(),
            next: "Review expiry tests".to_string(),
            risks: vec!["Expiry coverage is incomplete".to_string()],
            refs: vec!["git:abc123".to_string()],
        }
    }

    #[test]
    fn handoff_body_is_readable_without_metadata() {
        let body = render_ready(&packet());
        assert!(body.contains("Task: AUTH-118"));
        assert!(body.contains("Revision: r2"));
        assert!(body.contains("Supersedes: prior-message"));
        assert!(body.contains("Summary: Auth change complete"));
        assert!(body.contains("Next: Review expiry tests"));
        assert!(body.contains("- git:abc123"));
    }

    #[test]
    fn handoff_fields_are_bounded() {
        let mut invalid = packet();
        invalid.task_id = " ".to_string();
        assert!(invalid.validate().is_err());
        let mut invalid = packet();
        invalid.refs = vec!["x".to_string(); 11];
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn only_a_complete_bounded_packet_is_a_structured_handoff() {
        let valid = serde_json::json!({
            "version": 2,
            "task_id": "AUTH-118",
            "revision": "r2",
            "summary": "Ready for review",
            "next": "Review the change",
            "risks": [],
            "refs": ["git:abc123"]
        });
        assert!(is_valid_ready_packet(&valid));

        let malformed = serde_json::json!({
            "version": 2,
            "task_id": "AUTH-118",
            "summary": "Missing required fields"
        });
        assert!(!is_valid_ready_packet(&malformed));
    }
}
