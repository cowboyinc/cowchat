use cowchat_client::CowchatClient;
use cowchat_core::ChatMessage;
use serde::{Deserialize, Serialize};

pub(crate) const HANDOFF_READY_KIND: &str = "handoff.ready";
pub(crate) const HANDOFF_ACCEPTED_KIND: &str = "handoff.accepted";
const HANDOFF_VERSION: u8 = 1;
const MAX_TEXT_LENGTH: usize = 2_000;
const MAX_ITEMS: usize = 10;
const MAX_ITEM_LENGTH: usize = 500;

#[derive(Debug, Deserialize, Serialize)]
struct HandoffPacket {
    version: u8,
    summary: String,
    next: String,
    risks: Vec<String>,
    refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HandoffAcceptancePacket {
    version: u8,
    accepted_handoff_id: String,
    note: Option<String>,
}

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
}

pub(crate) async fn send(
    client: &CowchatClient,
    room_id: &str,
    summary: &str,
    next: &str,
    risks: &[String],
    refs: &[String],
) -> Result<ChatMessage, Box<dyn std::error::Error>> {
    let packet = HandoffPacket {
        version: HANDOFF_VERSION,
        summary: bounded_required("summary", summary)?,
        next: bounded_required("next", next)?,
        risks: bounded_items("risk", risks)?,
        refs: bounded_items("ref", refs)?,
    };
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
) -> Result<String, Box<dyn std::error::Error>> {
    let messages = client.get_history(room_id, limit, None).await?;
    let output = HandoffListOutput {
        handoffs: messages.iter().filter_map(parse_list_item).collect(),
    };
    if json {
        return Ok(format!("{}\n", serde_json::to_string(&output)?));
    }

    if output.handoffs.is_empty() {
        return Ok("No handoffs found.\n".to_string());
    }

    let mut lines = Vec::with_capacity(output.handoffs.len());
    for item in output.handoffs {
        let detail = serde_json::from_value::<HandoffPacket>(item.handoff.clone())
            .map(|packet| format!("{} → {}", packet.summary, packet.next))
            .unwrap_or_else(|_| "accepted".to_string());
        lines.push(format!(
            "#{} {} {}: {} ({})",
            item.seq, item.kind, item.agent_name, detail, item.message_id
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
    let messages = client.get_history(room_id, 500, None).await?;
    let handoff = messages
        .iter()
        .find(|message| message.message_id == handoff_message_id)
        .ok_or_else(|| {
            format!("handoff {handoff_message_id} was not found in the latest 500 messages")
        })?;
    if !is_valid_ready_handoff(handoff) {
        return Err(
            format!("message {handoff_message_id} is not a valid handoff.ready event").into(),
        );
    }

    let note = note
        .map(|value| bounded_required("note", value))
        .transpose()?;
    let content = match note.as_deref() {
        Some(note) => format!("Handoff accepted: {handoff_message_id}\n\nNote: {note}"),
        None => format!("Handoff accepted: {handoff_message_id}"),
    };
    Ok(client
        .send_message_with_metadata(
            room_id,
            &content,
            Some(handoff_message_id),
            vec![],
            serde_json::json!({
                "kind": HANDOFF_ACCEPTED_KIND,
                "handoff": {
                    "version": HANDOFF_VERSION,
                    "accepted_handoff_id": handoff_message_id,
                    "note": note,
                }
            }),
        )
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
    })
}

fn is_valid_ready_handoff(message: &ChatMessage) -> bool {
    message
        .metadata
        .get("kind")
        .and_then(|value| value.as_str())
        == Some(HANDOFF_READY_KIND)
        && message
            .metadata
            .get("handoff")
            .is_some_and(is_valid_ready_packet)
}

fn is_valid_ready_packet(value: &serde_json::Value) -> bool {
    serde_json::from_value::<HandoffPacket>(value.clone())
        .ok()
        .is_some_and(|packet| {
            packet.version == HANDOFF_VERSION
                && bounded_required("summary", &packet.summary).is_ok()
                && bounded_required("next", &packet.next).is_ok()
                && bounded_items("risk", &packet.risks).is_ok()
                && bounded_items("ref", &packet.refs).is_ok()
        })
}

fn is_valid_acceptance_packet(message: &ChatMessage, value: &serde_json::Value) -> bool {
    serde_json::from_value::<HandoffAcceptancePacket>(value.clone())
        .ok()
        .is_some_and(|packet| {
            packet.version == HANDOFF_VERSION
                && !packet.accepted_handoff_id.trim().is_empty()
                && message.reply_to_message.as_deref() == Some(&packet.accepted_handoff_id)
                && packet
                    .note
                    .as_deref()
                    .map(|note| bounded_required("note", note).is_ok())
                    .unwrap_or(true)
        })
}

fn bounded_required(name: &str, value: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("--{name} must not be empty").into());
    }
    if value.chars().count() > MAX_TEXT_LENGTH {
        return Err(format!("--{name} must be at most {MAX_TEXT_LENGTH} characters").into());
    }
    Ok(value.to_string())
}

fn bounded_items(name: &str, values: &[String]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if values.len() > MAX_ITEMS {
        return Err(format!("at most {MAX_ITEMS} --{name} values are allowed").into());
    }
    values
        .iter()
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("--{name} must not be empty").into());
            }
            if value.chars().count() > MAX_ITEM_LENGTH {
                return Err(
                    format!("--{name} must be at most {MAX_ITEM_LENGTH} characters").into(),
                );
            }
            Ok(value.to_string())
        })
        .collect()
}

fn render_ready(packet: &HandoffPacket) -> String {
    let mut lines = vec![
        "Handoff ready".to_string(),
        String::new(),
        format!("Summary: {}", packet.summary),
        format!("Next: {}", packet.next),
    ];
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
    use super::{
        bounded_items, bounded_required, is_valid_ready_packet, render_ready, HandoffPacket,
    };

    #[test]
    fn handoff_body_is_readable_without_metadata() {
        let body = render_ready(&HandoffPacket {
            version: 1,
            summary: "Auth change complete".to_string(),
            next: "Review expiry tests".to_string(),
            risks: vec!["Expiry coverage is incomplete".to_string()],
            refs: vec!["git:abc123".to_string()],
        });
        assert!(body.contains("Summary: Auth change complete"));
        assert!(body.contains("Next: Review expiry tests"));
        assert!(body.contains("- git:abc123"));
    }

    #[test]
    fn handoff_fields_are_bounded_and_trimmed() {
        assert_eq!(bounded_required("summary", "  useful  ").unwrap(), "useful");
        assert!(bounded_required("summary", "   ").is_err());
        assert!(bounded_items("ref", &[" ".to_string()]).is_err());
        assert!(bounded_items("ref", &vec!["x".to_string(); 11]).is_err());
    }

    #[test]
    fn only_a_complete_bounded_packet_is_a_structured_handoff() {
        let valid = serde_json::json!({
            "version": 1,
            "summary": "Ready for review",
            "next": "Review the change",
            "risks": [],
            "refs": ["git:abc123"]
        });
        assert!(is_valid_ready_packet(&valid));

        let malformed = serde_json::json!({
            "version": 1,
            "summary": "Missing required fields"
        });
        assert!(!is_valid_ready_packet(&malformed));
    }
}
