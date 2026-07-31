//! LANTERN: an optional structured-reasoning overlay carried entirely inside
//! Cowchat message content (no server changes). Each LANTERN message is a JSON
//! envelope stored as the message `content` — encrypted like any content in an
//! encrypted room — tagged with `metadata.kind = "lantern"` so it can be found
//! without inspecting (encrypted) content. State (threads, shared model,
//! calibration) is reconstructed client-side from decrypted history.
//!
//! Thread ids are sequence-based: an opening PROBE/ASSERT carries no `thread`,
//! and is identified by its own message seq; replies set `thread` to that seq.
//! This avoids any server-side id scheme and matches the seq-based `reply_to`.

use cowchat_core::ChatMessage;
use serde::{Deserialize, Serialize};

pub const LANTERN_VERSION: &str = "lantern/0.1.1";
/// `metadata.kind` marker on every LANTERN message — the only thing written to
/// (plaintext) metadata, so encrypted rooms don't leak thread shape or verb.
pub const LANTERN_KIND: &str = "lantern";

/// The nine thread verbs (HELLO is handled separately — it has its own shape).
pub const VERBS: &[&str] = &[
    "PROBE",
    "ASSERT",
    "CHALLENGE",
    "RESOLVE",
    "FUSE",
    "SYNC",
    "SPARK",
    "HARVEST",
    "BURY",
];

/// `basis` values eligible for calibration scoring.
pub const SCORABLE_BASIS: &[&str] = &["tool", "artifact", "human"];
pub const ALL_BASIS: &[&str] = &["tool", "artifact", "human", "consensus", "stale"];

/// A thread verb envelope. `body` is verb-specific and validated per verb.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: String,
    #[serde(rename = "type")]
    pub verb: String,
    /// Root thread seq. Absent on an opening verb (then the thread is its own seq).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<i64>,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default)]
    pub body: serde_json::Value,
}

impl Envelope {
    pub fn new(
        verb: &str,
        from: &str,
        thread: Option<i64>,
        reply_to: Option<i64>,
        intent: Option<String>,
        body: serde_json::Value,
    ) -> Self {
        Self {
            v: LANTERN_VERSION.to_string(),
            verb: verb.to_string(),
            thread,
            reply_to,
            from: from.to_string(),
            intent,
            body,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub falsifiable_by: Option<String>,
}

/// The HELLO session preamble — provenance + capability claims. Distinct shape
/// from the thread verbs (top-level identity fields, not the {thread,body} form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub v: String,
    #[serde(rename = "type")]
    pub verb: String,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// Per-field attestation level: every field is `self_attested` until a host
    /// signature / runtime attestation / human vouch promotes it to `verified`.
    pub attestation: serde_json::Value,
}

impl Hello {
    pub fn new(
        agent_name: &str,
        provider: Option<String>,
        model: Option<String>,
        role: Option<String>,
        capabilities: Vec<Capability>,
    ) -> Self {
        // Everything a HELLO asserts about itself is self_attested by definition;
        // only an external party can promote a field to "verified".
        let attestation = serde_json::json!({
            "agent_name": "self_attested",
            "provider": "self_attested",
            "model": "self_attested",
            "role": "self_attested",
            "capabilities": "self_attested",
        });
        Self {
            v: LANTERN_VERSION.to_string(),
            verb: "HELLO".to_string(),
            agent_name: agent_name.to_string(),
            provider,
            model,
            role,
            capabilities,
            attestation,
        }
    }
}

/// A LANTERN message parsed from a Cowchat message's content.
#[derive(Debug, Clone)]
pub enum Parsed {
    Hello(Hello),
    Verb(Envelope),
}

/// Parse decrypted message content as a LANTERN message, or None if it isn't one.
pub fn parse(content: &str) -> Option<Parsed> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let v = value.get("v")?.as_str()?;
    if !v.starts_with("lantern/") {
        return None;
    }
    match value.get("type").and_then(|t| t.as_str()) {
        Some("HELLO") => serde_json::from_value::<Hello>(value)
            .ok()
            .map(Parsed::Hello),
        Some(_) => serde_json::from_value::<Envelope>(value)
            .ok()
            .map(Parsed::Verb),
        None => None,
    }
}

/// Validate a LANTERN message (parsed from a JSON value). Returns the list of
/// problems; empty means valid. Enforced on send so malformed claims can't go out.
// Keep every verb arm in the same if-in-body shape: multi-check arms (ASSERT,
// CHALLENGE, …) can't be a match guard, so collapsing the single-check arms into
// guards would make the per-verb checks inconsistent and harder to scan.
#[allow(clippy::collapsible_match)]
pub fn validate(value: &serde_json::Value) -> Vec<String> {
    let mut errs = Vec::new();
    let v = value.get("v").and_then(|x| x.as_str()).unwrap_or("");
    if !v.starts_with("lantern/") {
        errs.push("missing or invalid `v` (must start with \"lantern/\")".into());
    }
    let verb = match value.get("type").and_then(|x| x.as_str()) {
        Some(t) => t,
        None => {
            errs.push("missing `type`".into());
            return errs;
        }
    };
    let body = value
        .get("body")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let nonempty = |b: &serde_json::Value, k: &str| {
        b.get(k)
            .and_then(|x| x.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    };

    if verb == "HELLO" {
        if value
            .get("agent_name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .is_empty()
        {
            errs.push("HELLO requires `agent_name`".into());
        }
        return errs;
    }

    if !VERBS.contains(&verb) {
        errs.push(format!("unknown verb `{verb}`"));
    }
    if value
        .get("from")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .is_empty()
    {
        errs.push("missing `from`".into());
    }

    match verb {
        "PROBE" => {
            if !nonempty(&body, "question") {
                errs.push("PROBE requires `body.question`".into());
            }
        }
        "ASSERT" => {
            if !nonempty(&body, "claim") {
                errs.push("ASSERT requires `body.claim`".into());
            }
            // The defining rule: an ASSERT without falsifiable_by is malformed.
            if !nonempty(&body, "falsifiable_by") {
                errs.push(
                    "ASSERT requires `body.falsifiable_by` (what observation would retract it)"
                        .into(),
                );
            }
            check_confidence(&body, &mut errs, false);
        }
        "CHALLENGE" => {
            if body.get("target_seq").and_then(|x| x.as_i64()).is_none() {
                errs.push("CHALLENGE requires `body.target_seq`".into());
            }
            if !nonempty(&body, "counter_claim") {
                errs.push("CHALLENGE requires `body.counter_claim`".into());
            }
            if !nonempty(&body, "test") {
                errs.push("CHALLENGE must name a `body.test`".into());
            }
            check_confidence(&body, &mut errs, true);
        }
        "RESOLVE" => {
            if !nonempty(&body, "observation") {
                errs.push("RESOLVE requires `body.observation`".into());
            }
            match body.get("basis").and_then(|x| x.as_str()) {
                Some(b) if ALL_BASIS.contains(&b) => {}
                _ => errs.push(format!("RESOLVE `body.basis` must be one of {ALL_BASIS:?}")),
            }
        }
        "FUSE" => {
            if !nonempty(&body, "synthesis") {
                errs.push("FUSE requires `body.synthesis`".into());
            }
        }
        "SPARK" => {
            for k in ["seed", "why_now", "smallest_test"] {
                if !nonempty(&body, k) {
                    errs.push(format!("SPARK requires `body.{k}`"));
                }
            }
        }
        "HARVEST" => {
            if body.get("spark_seq").and_then(|x| x.as_i64()).is_none() {
                errs.push("HARVEST requires `body.spark_seq`".into());
            }
        }
        "BURY" => {
            if body.get("spark_seq").and_then(|x| x.as_i64()).is_none() {
                errs.push("BURY requires `body.spark_seq`".into());
            }
            if !nonempty(&body, "reason") {
                errs.push("BURY requires `body.reason`".into());
            }
        }
        _ => {}
    }
    errs
}

fn check_confidence(body: &serde_json::Value, errs: &mut Vec<String>, required: bool) {
    match body.get("confidence").and_then(|x| x.as_f64()) {
        Some(c) if (0.0..=1.0).contains(&c) => {}
        Some(_) => errs.push("`body.confidence` must be in [0, 1]".into()),
        None if required => errs.push("a CHALLENGE must stake `body.confidence`".into()),
        None => {}
    }
}

// --- Reconstruction ---

/// One LANTERN message located in room history (decrypted), with its resolved
/// thread id (own seq for an opening verb).
#[derive(Debug, Clone)]
pub struct ThreadMessage {
    pub seq: i64,
    pub from: String,
    pub verb: String,
    pub body: serde_json::Value,
    pub intent: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreadState {
    Open,
    Resolved,
    Fused,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: i64,
    pub messages: Vec<ThreadMessage>,
    pub state: ThreadState,
    pub resolve_basis: Option<String>,
}

impl Thread {
    /// One-line headline: the opening claim/question, else the first intent.
    pub fn headline(&self) -> String {
        let first = &self.messages[0];
        let pick = |k: &str| {
            first
                .body
                .get(k)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        pick("claim")
            .or_else(|| pick("question"))
            .or_else(|| first.intent.clone())
            .unwrap_or_else(|| format!("{} thread", first.verb))
    }
}

/// Collect HELLOs and threads from decrypted room history.
pub struct Reconstructed {
    pub hellos: Vec<(i64, Hello)>,
    pub threads: Vec<Thread>,
    /// Committed shared-state deltas, in FUSE order.
    pub shared_state: Vec<serde_json::Value>,
    /// Count of FUSE messages across the room (for the REFRACTION-every-3rd rule).
    pub fuse_count: usize,
}

pub fn reconstruct(history: &[ChatMessage]) -> Reconstructed {
    let mut hellos = Vec::new();
    let mut by_thread: std::collections::BTreeMap<i64, Vec<ThreadMessage>> = Default::default();
    let mut shared_state = Vec::new();
    let mut fuse_count = 0;

    for m in history {
        match parse(&m.content) {
            Some(Parsed::Hello(h)) => hellos.push((m.seq, h)),
            Some(Parsed::Verb(e)) => {
                // Opening verbs anchor a thread at their own seq; replies use `thread`.
                let tid = e.thread.unwrap_or(m.seq);
                if e.verb == "FUSE" {
                    fuse_count += 1;
                    if let Some(delta) = e.body.get("shared_state_delta") {
                        if !delta.is_null() {
                            shared_state.push(delta.clone());
                        }
                    }
                }
                by_thread.entry(tid).or_default().push(ThreadMessage {
                    seq: m.seq,
                    from: e.from,
                    verb: e.verb,
                    body: e.body,
                    intent: e.intent,
                });
            }
            None => {}
        }
    }

    let threads = by_thread
        .into_iter()
        .map(|(id, mut msgs)| {
            msgs.sort_by_key(|m| m.seq);
            let resolve_basis = msgs.iter().find(|m| m.verb == "RESOLVE").and_then(|m| {
                m.body
                    .get("basis")
                    .and_then(|b| b.as_str())
                    .map(str::to_string)
            });
            let state = if msgs.iter().any(|m| m.verb == "FUSE") {
                ThreadState::Fused
            } else if msgs.iter().any(|m| m.verb == "RESOLVE") {
                ThreadState::Resolved
            } else {
                ThreadState::Open
            };
            Thread {
                id,
                messages: msgs,
                state,
                resolve_basis,
            }
        })
        .collect();

    Reconstructed {
        hellos,
        threads,
        shared_state,
        fuse_count,
    }
}

// --- Calibration ---

/// Per-agent calibration loss. Lower is better; only FUSEd threads whose RESOLVE
/// basis is tool/artifact/human and whose FUSE records an `outcomes` verdict for
/// a staked claim are scored. Loss = -ln(confidence) if the claim held, else
/// -ln(1 - confidence), clamped to [0, ln(10)].
#[derive(Debug, Clone, Default)]
pub struct Calibration {
    pub per_agent: std::collections::BTreeMap<String, (f64, usize)>, // (sum_loss, n)
}

impl Calibration {
    pub fn mean(&self, agent: &str) -> Option<f64> {
        self.per_agent.get(agent).map(|(s, n)| s / *n as f64)
    }
}

pub fn calibration(rec: &Reconstructed) -> Calibration {
    let max = (10.0f64).ln();
    let mut cal = Calibration::default();
    for t in &rec.threads {
        if t.state != ThreadState::Fused {
            continue;
        }
        let basis = match &t.resolve_basis {
            Some(b) if SCORABLE_BASIS.contains(&b.as_str()) => b,
            _ => continue,
        };
        let _ = basis;
        // Gather outcomes recorded on the FUSE: { "<claim_seq>": true|false }.
        let outcomes: std::collections::HashMap<i64, bool> = t
            .messages
            .iter()
            .filter(|m| m.verb == "FUSE")
            .filter_map(|m| m.body.get("outcomes").and_then(|o| o.as_object()))
            .flat_map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_bool()?)))
            })
            .collect();
        if outcomes.is_empty() {
            continue;
        }
        // Score each staked ASSERT/CHALLENGE that has an outcome.
        for m in &t.messages {
            if m.verb != "ASSERT" && m.verb != "CHALLENGE" {
                continue;
            }
            let Some(held) = outcomes.get(&m.seq) else {
                continue;
            };
            let Some(conf) = m.body.get("confidence").and_then(|c| c.as_f64()) else {
                continue;
            };
            let conf = conf.clamp(1e-6, 1.0 - 1e-6);
            let loss = if *held {
                -conf.ln()
            } else {
                -(1.0 - conf).ln()
            };
            let loss = loss.clamp(0.0, max);
            let entry = cal.per_agent.entry(m.from.clone()).or_insert((0.0, 0));
            entry.0 += loss;
            entry.1 += 1;
        }
    }
    cal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: i64, from: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: format!("m{seq}"),
            room_id: "r".into(),
            agent_id: format!("id-{from}"),
            agent_name: from.into(),
            content: content.into(),
            reply_to_message: None,
            metadata: serde_json::json!({"kind": "lantern"}),
            timestamp: chrono::Utc::now(),
            seq,
        }
    }

    #[test]
    fn assert_without_falsifiable_by_is_invalid() {
        let env = Envelope::new(
            "ASSERT",
            "a",
            None,
            None,
            None,
            serde_json::json!({"claim": "x"}),
        );
        let errs = validate(&serde_json::to_value(&env).unwrap());
        assert!(errs.iter().any(|e| e.contains("falsifiable_by")));

        let ok = Envelope::new(
            "ASSERT",
            "a",
            None,
            None,
            None,
            serde_json::json!({"claim": "x", "falsifiable_by": "a test"}),
        );
        assert!(validate(&serde_json::to_value(&ok).unwrap()).is_empty());
    }

    #[test]
    fn challenge_must_stake_confidence_and_test() {
        let env = Envelope::new(
            "CHALLENGE",
            "b",
            Some(1),
            Some(1),
            None,
            serde_json::json!({"target_seq": 1, "counter_claim": "no"}),
        );
        let errs = validate(&serde_json::to_value(&env).unwrap());
        assert!(errs.iter().any(|e| e.contains("confidence")));
        assert!(errs.iter().any(|e| e.contains("test")));
    }

    #[test]
    fn reconstruct_groups_thread_by_opening_seq() {
        let a = Envelope::new(
            "ASSERT",
            "a",
            None,
            None,
            None,
            serde_json::json!({"claim": "needs rollback gate", "falsifiable_by": "x", "confidence": 0.7}),
        );
        let c = Envelope::new(
            "CHALLENGE",
            "b",
            Some(10),
            Some(10),
            None,
            serde_json::json!({"target_seq": 10, "counter_claim": "exists as recovery", "confidence": 0.6, "test": "grep"}),
        );
        let r = Envelope::new(
            "RESOLVE",
            "a",
            Some(10),
            Some(11),
            None,
            serde_json::json!({"observation": "no rollback gate", "basis": "artifact"}),
        );
        let f = Envelope::new(
            "FUSE",
            "a",
            Some(10),
            Some(12),
            None,
            serde_json::json!({"synthesis": "add gate", "shared_state_delta": {"missing_work": ["gate"]},
                               "outcomes": {"10": true, "11": false}}),
        );
        let history = vec![
            msg(10, "a", &serde_json::to_string(&a).unwrap()),
            msg(11, "b", &serde_json::to_string(&c).unwrap()),
            msg(12, "a", &serde_json::to_string(&r).unwrap()),
            msg(13, "a", &serde_json::to_string(&f).unwrap()),
            msg(14, "a", "just plain chat, not lantern"),
        ];
        let rec = reconstruct(&history);
        assert_eq!(rec.threads.len(), 1);
        let t = &rec.threads[0];
        assert_eq!(t.id, 10);
        assert_eq!(t.messages.len(), 4);
        assert_eq!(t.state, ThreadState::Fused);
        assert_eq!(t.resolve_basis.as_deref(), Some("artifact"));
        assert_eq!(rec.fuse_count, 1);
        assert_eq!(rec.shared_state.len(), 1);

        // Calibration: assert(10) held (loss=-ln .7), challenge(11) at seq11 false (loss=-ln(1-.6)).
        let cal = calibration(&rec);
        let a_loss = cal.mean("a").unwrap();
        assert!((a_loss - (-0.7f64.ln())).abs() < 1e-9, "a loss {a_loss}");
        let b_loss = cal.mean("b").unwrap();
        assert!(
            (b_loss - (-(1.0f64 - 0.6).ln())).abs() < 1e-9,
            "b loss {b_loss}"
        );
    }
}
