//! HTTP-webhook delivery for external automations.
//!
//! Implements the **Standard Webhooks v1** signature (standardwebhooks.com) so
//! receivers can plug in `svix` or any compatible verifier. On every successful
//! `send_message`, the handler asks this module to enqueue deliveries for any
//! active subscriptions whose filter matches; a background worker drains the
//! queue, POSTs each event, advances cursors on 2xx, and retries with
//! exponential backoff on failure.
//!
//! The send path NEVER blocks on HTTP — it only inserts queue rows and pokes a
//! `Notify`. All I/O happens on the worker task.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use cowchat_core::{ChatMessage, Subscription};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::store::Store;

/// Retry schedule (seconds from the failed attempt). After this many failures
/// (== length of this slice) the subscription is marked `failed` and stops
/// receiving deliveries until the owner re-enables it.
const RETRY_SCHEDULE_SECS: &[u64] = &[1, 4, 16, 64, 256];
const WEBHOOK_DNS_TIMEOUT: Duration = Duration::from_secs(5);

type HmacSha256 = Hmac<Sha256>;

/// Compute the Standard Webhooks v1 signature:
///   v1,<base64(HMAC-SHA256(secret, "{webhook_id}.{timestamp}.{body}"))>
///
/// The receiver verifies by recomputing this string and comparing in constant time.
pub fn sign_request(secret: &str, webhook_id: &str, timestamp: i64, body: &str) -> String {
    let signed_payload = format!("{}.{}.{}", webhook_id, timestamp, body);
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC-SHA256 accepts any key length");
    mac.update(signed_payload.as_bytes());
    let sig = mac.finalize().into_bytes();
    format!("v1,{}", BASE64.encode(sig))
}

/// Decide whether `msg` matches `sub`'s filter. Cross-field semantics are AND;
/// within `kinds`, semantics are OR.
pub fn matches_filter(sub: &Subscription, msg: &ChatMessage) -> bool {
    if msg.seq <= sub.last_delivered_seq {
        return false;
    }
    let meta_type = msg.metadata.get("type").and_then(|v| v.as_str());
    // System messages aren't deliverable — they're protocol noise, not chat.
    if meta_type == Some("system") {
        return false;
    }
    if sub.exclude_thinking && meta_type == Some("thinking") {
        return false;
    }
    if !sub.kinds.is_empty() {
        let kind = msg.metadata.get("kind").and_then(|v| v.as_str());
        let any_match = sub.kinds.iter().any(|want| Some(want.as_str()) == kind);
        if !any_match {
            return false;
        }
    }
    if let Some(only) = &sub.only_from {
        if &msg.agent_name != only {
            return false;
        }
    }
    if let Some(skip) = &sub.not_from {
        if &msg.agent_name == skip {
            return false;
        }
    }
    true
}

/// Public handle held by handler/server. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct WebhookManager {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    notify: Notify,
    allow_private: bool,
    #[cfg(test)]
    validation_gate: Option<ValidationGate>,
}

#[cfg(test)]
struct ValidationGate {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
                || ip == Ipv4Addr::new(169, 254, 169, 254))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| !is_public_ip(IpAddr::V4(mapped))))
        }
    }
}

async fn resolve_public_webhook(
    raw: &str,
    allow_private: bool,
) -> Result<(reqwest::Url, String, SocketAddr), String> {
    let url = reqwest::Url::parse(raw).map_err(|error| format!("invalid webhook_url: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("webhook_url must be http(s)".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("webhook_url credentials are not allowed".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "webhook_url requires a host".to_string())?
        .to_string();
    if !allow_private && (host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost")) {
        return Err("webhook_url may not target localhost".into());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "webhook_url requires a known port".to_string())?;
    let addresses = tokio::time::timeout(
        WEBHOOK_DNS_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| "webhook host resolution timed out".to_string())?
    .map_err(|error| format!("webhook host resolution failed: {error}"))?
    .collect::<Vec<_>>();
    if addresses.is_empty()
        || (!allow_private && addresses.iter().any(|addr| !is_public_ip(addr.ip())))
    {
        return Err("webhook_url resolves to a private, local, or reserved address".into());
    }
    Ok((url, host, addresses[0]))
}

async fn pinned_http_client(
    raw: &str,
    allow_private: bool,
) -> Result<(reqwest::Client, reqwest::Url), String> {
    let (url, host, address) = resolve_public_webhook(raw, allow_private).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("cowchat-webhook/", env!("CARGO_PKG_VERSION")))
        .resolve(&host, address)
        .build()
        .map_err(|error| format!("webhook client setup failed: {error}"))?;
    Ok((client, url))
}

impl WebhookManager {
    pub fn new(store: Arc<Store>, allow_private: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                notify: Notify::new(),
                allow_private,
                #[cfg(test)]
                validation_gate: None,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_validation_gate(
        store: Arc<Store>,
        allow_private: bool,
        started: Arc<Notify>,
        release: Arc<Notify>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                notify: Notify::new(),
                allow_private,
                validation_gate: Some(ValidationGate { started, release }),
            }),
        }
    }

    pub async fn validate_url(&self, raw: &str) -> Result<(), String> {
        #[cfg(test)]
        if let Some(gate) = self.inner.validation_gate.as_ref() {
            gate.started.notify_one();
            gate.release.notified().await;
        }
        resolve_public_webhook(raw, self.inner.allow_private)
            .await
            .map(|_| ())
    }

    /// Enqueue deliveries for `msg` to every matching active subscription on
    /// the room. Idempotent: a duplicate (subscription, seq) is silently skipped.
    /// This is the only entry point called from the message send path; it does
    /// no HTTP I/O and returns quickly.
    pub fn enqueue_for_message(&self, msg: &ChatMessage) {
        let inner = self.inner.clone();
        let msg = msg.clone();
        // Spawned so a slow SQLite scan never blocks the send broadcast.
        tokio::spawn(async move {
            let subs = match inner.store.list_active_subscriptions_for_room(&msg.room_id) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("webhook enqueue: failed to list subs: {}", e);
                    return;
                }
            };
            let now = chrono::Utc::now();
            let mut enqueued = 0;
            for (sub, _owner, _secret) in subs {
                if !matches_filter(&sub, &msg) {
                    continue;
                }
                let delivery_id = Uuid::new_v4().to_string();
                match inner.store.enqueue_delivery(
                    &delivery_id,
                    &sub.subscription_id,
                    msg.seq,
                    &msg.message_id,
                    now,
                ) {
                    Ok(true) => enqueued += 1,
                    Ok(false) => {}
                    Err(e) => log::warn!("webhook enqueue: {}", e),
                }
            }
            if enqueued > 0 {
                inner.notify.notify_one();
            }
        });
    }

    /// Wake the worker. Used after enqueueing the backfill on a fresh subscription.
    pub fn wake(&self) {
        self.inner.notify.notify_one();
    }

    /// Spawn the delivery worker. Returns the JoinHandle so the server can keep it alive.
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            delivery_worker(inner).await;
        })
    }
}

async fn delivery_worker(inner: Arc<Inner>) {
    log::info!("webhook delivery worker started");
    loop {
        // Pull a batch of due deliveries.
        let now = chrono::Utc::now();
        let due = match inner.store.load_due_deliveries(now, 32) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("webhook worker: load_due_deliveries failed: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        if due.is_empty() {
            // Sleep until the next queued retry's deadline, or until something
            // gets enqueued (notify), or 60s for a housekeeping pass — whichever
            // comes first. Without this we'd miss the 1s/4s/16s retry slots and
            // only wake on the 60s timer.
            let until_next = match inner.store.earliest_pending_attempt() {
                Ok(Some(t)) => {
                    let now = chrono::Utc::now();
                    let delta = (t - now).num_milliseconds().max(0) as u64;
                    Duration::from_millis(delta.min(60_000))
                }
                _ => Duration::from_secs(60),
            };
            tokio::select! {
                _ = inner.notify.notified() => {}
                _ = tokio::time::sleep(until_next) => {}
            }
            continue;
        }

        // Process deliveries in parallel across subscriptions, serialized per-sub.
        // Simple approach: spawn one task per delivery; the per-subscription cursor
        // update is serialized in SQLite anyway. For our load this is plenty.
        let mut handles = Vec::with_capacity(due.len());
        for d in due {
            let inner = inner.clone();
            handles.push(tokio::spawn(async move {
                process_delivery(inner, d).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }
    }
}

async fn process_delivery(inner: Arc<Inner>, d: crate::store::PendingDelivery) {
    // Look up the subscription (need URL + secret + filter state to know cursor).
    let sub_lookup = match inner.store.get_subscription(&d.subscription_id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            // Subscription was deleted out from under us; drop the queued row.
            let _ = inner.store.delete_delivery(&d.delivery_id);
            return;
        }
        Err(e) => {
            log::warn!("webhook delivery: load sub {}: {}", d.subscription_id, e);
            return;
        }
    };
    let (sub, _owner, secret) = sub_lookup;
    if sub.status != "active" {
        let _ = inner.store.delete_delivery(&d.delivery_id);
        return;
    }

    let msg = match inner.store.get_message(&d.message_id) {
        Ok(Some(m)) => m,
        Ok(None) => {
            log::warn!(
                "webhook delivery: message {} not found, dropping",
                d.message_id
            );
            let _ = inner.store.delete_delivery(&d.delivery_id);
            return;
        }
        Err(e) => {
            log::warn!("webhook delivery: get_message {}: {}", d.message_id, e);
            return;
        }
    };

    let webhook_id = Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();
    let envelope = serde_json::json!({
        "type": "cowchat.message.created",
        "subscription_id": sub.subscription_id,
        "room_id": sub.room_id,
        "message": msg,
    });
    let body = serde_json::to_string(&envelope).unwrap_or_default();
    let signature = sign_request(&secret, &webhook_id, timestamp, &body);

    let (http, pinned_url) = match pinned_http_client(&sub.webhook_url, inner.allow_private).await {
        Ok(value) => value,
        Err(error) => {
            log::warn!(
                "webhook delivery blocked sub={}: {}",
                sub.subscription_id,
                error
            );
            let _ = inner.store.set_subscription_status(
                &sub.subscription_id,
                "failed",
                Some(d.attempts + 1),
            );
            return;
        }
    };
    let req = http
        .post(pinned_url)
        .header("content-type", "application/json")
        .header("webhook-id", &webhook_id)
        .header("webhook-timestamp", timestamp.to_string())
        .header("webhook-signature", &signature)
        .body(body);

    let outcome = match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(())
            } else {
                Err(format!("HTTP {}", status.as_u16()))
            }
        }
        Err(e) => Err(format!("transport: {}", e)),
    };

    match outcome {
        Ok(()) => {
            let _ = inner.store.delete_delivery(&d.delivery_id);
            let _ = inner
                .store
                .advance_subscription_cursor(&sub.subscription_id, d.message_seq);
            log::debug!(
                "webhook delivered sub={} seq={}",
                sub.subscription_id,
                d.message_seq
            );
        }
        Err(err) => {
            let new_attempts = d.attempts + 1;
            let idx = (new_attempts as usize).saturating_sub(1);
            if idx >= RETRY_SCHEDULE_SECS.len() {
                // Give up on this delivery and mark the sub failed.
                let _ = inner.store.delete_delivery(&d.delivery_id);
                let _ = inner.store.set_subscription_status(
                    &sub.subscription_id,
                    "failed",
                    Some(new_attempts),
                );
                log::warn!(
                    "webhook delivery failed permanently sub={} seq={} err={}",
                    sub.subscription_id,
                    d.message_seq,
                    err
                );
            } else {
                let delay = Duration::from_secs(RETRY_SCHEDULE_SECS[idx]);
                let next = chrono::Utc::now() + chrono::Duration::from_std(delay).unwrap();
                let _ = inner
                    .store
                    .reschedule_delivery(&d.delivery_id, next, new_attempts, &err);
                log::debug!(
                    "webhook delivery retry sub={} seq={} attempt={} in={:?} err={}",
                    sub.subscription_id,
                    d.message_seq,
                    new_attempts,
                    delay,
                    err
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_verifies() {
        let s1 = sign_request("topsecret", "wh_123", 1700000000, r#"{"a":1}"#);
        let s2 = sign_request("topsecret", "wh_123", 1700000000, r#"{"a":1}"#);
        assert_eq!(s1, s2);
        assert!(s1.starts_with("v1,"));
        // Different inputs → different sig.
        let s3 = sign_request("topsecret", "wh_124", 1700000000, r#"{"a":1}"#);
        assert_ne!(s1, s3);
    }

    #[tokio::test]
    async fn webhook_url_rejects_local_and_metadata_targets() {
        assert!(resolve_public_webhook("http://127.0.0.1/hook", false)
            .await
            .is_err());
        assert!(resolve_public_webhook("http://[::1]/hook", false)
            .await
            .is_err());
        assert!(
            resolve_public_webhook("http://169.254.169.254/latest", false)
                .await
                .is_err()
        );
        assert!(resolve_public_webhook("ftp://example.com/hook", false)
            .await
            .is_err());
    }

    #[test]
    fn matches_filter_kind_and_from() {
        use serde_json::json;
        let sub_base = Subscription {
            subscription_id: "s".into(),
            room_id: "r".into(),
            webhook_url: "http://x".into(),
            kinds: vec![],
            only_from: None,
            not_from: None,
            exclude_thinking: false,
            since_seq: 0,
            last_delivered_seq: 0,
            status: "active".into(),
            failure_count: 0,
            created_at: chrono::Utc::now(),
        };
        let msg = ChatMessage {
            message_id: "m".into(),
            room_id: "r".into(),
            agent_id: "a-id".into(),
            agent_name: "alice".into(),
            content: "hi".into(),
            reply_to_message: None,
            metadata: json!({"kind": "verdict"}),
            timestamp: chrono::Utc::now(),
            seq: 5,
        };

        // No filters → match.
        assert!(matches_filter(&sub_base, &msg));

        // Kind matches: pass.
        let s = Subscription {
            kinds: vec!["verdict".into()],
            ..sub_base.clone()
        };
        assert!(matches_filter(&s, &msg));

        // Kind doesn't match: skip.
        let s = Subscription {
            kinds: vec!["review_request".into()],
            ..sub_base.clone()
        };
        assert!(!matches_filter(&s, &msg));

        // only_from matches: pass.
        let s = Subscription {
            only_from: Some("alice".into()),
            ..sub_base.clone()
        };
        assert!(matches_filter(&s, &msg));

        // only_from doesn't: skip.
        let s = Subscription {
            only_from: Some("bob".into()),
            ..sub_base.clone()
        };
        assert!(!matches_filter(&s, &msg));

        // not_from skip.
        let s = Subscription {
            not_from: Some("alice".into()),
            ..sub_base.clone()
        };
        assert!(!matches_filter(&s, &msg));

        // last_delivered_seq advances past us: skip.
        let s = Subscription {
            last_delivered_seq: 5,
            ..sub_base.clone()
        };
        assert!(!matches_filter(&s, &msg));

        // System message excluded always.
        let mut sys_msg = msg.clone();
        sys_msg.metadata = json!({"type": "system"});
        assert!(!matches_filter(&sub_base, &sys_msg));

        // Thinking pulse excluded only when exclude_thinking is set.
        let mut think_msg = msg.clone();
        think_msg.metadata = json!({"type": "thinking"});
        assert!(matches_filter(&sub_base, &think_msg));
        let s = Subscription {
            exclude_thinking: true,
            ..sub_base.clone()
        };
        assert!(!matches_filter(&s, &think_msg));
    }
}
