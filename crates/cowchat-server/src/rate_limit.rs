use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Max API keys a single client IP may mint per [`SIGNUP_WINDOW`]. Open signup
/// stays self-serve, but a client can't spam unlimited keys to dodge per-key limits.
const SIGNUP_MAX_PER_WINDOW: u32 = 10;
const SIGNUP_WINDOW: Duration = Duration::from_secs(3600);

/// Per-IP key-creation counter within a sliding reset window.
struct SignupWindow {
    count: u32,
    window_start: Instant,
}

/// Per-key usage tracking.
pub struct KeyUsage {
    pub agent_count: AtomicU64,
    pub msg_count: AtomicU64,
    pub msg_window_start: std::sync::Mutex<Instant>,
    pub room_count: AtomicU64,
}

impl KeyUsage {
    fn new() -> Self {
        Self {
            agent_count: AtomicU64::new(0),
            msg_count: AtomicU64::new(0),
            msg_window_start: std::sync::Mutex::new(Instant::now()),
            room_count: AtomicU64::new(0),
        }
    }
}

/// Rate limits for a tier.
#[derive(Debug, Clone)]
pub struct TierLimits {
    pub max_agents: u64,
    pub max_messages_per_minute: u64,
    pub max_rooms: u64,
    /// Max bytes of message `content` as sent. Measured post-encryption, so the
    /// cap is set generously: `clw1:` ciphertext is base64 (~33% larger than the
    /// plaintext). This is a DoS guard against giant payloads, not a UX limit.
    pub max_message_bytes: usize,
    /// Days of message history retained before the background purge deletes it.
    /// `None` = retained indefinitely (enterprise). Doubles as the disk guard on
    /// the lowest tier and as an upsell lever on higher ones.
    pub history_retention_days: Option<u32>,
}

impl TierLimits {
    pub fn free() -> Self {
        Self {
            max_agents: 20,
            max_messages_per_minute: 200,
            max_rooms: 50,
            max_message_bytes: 64 * 1024,
            history_retention_days: Some(14),
        }
    }

    pub fn pro() -> Self {
        Self {
            max_agents: 200,
            max_messages_per_minute: 2000,
            max_rooms: 500,
            max_message_bytes: 256 * 1024,
            history_retention_days: Some(90),
        }
    }

    pub fn for_tier(tier: &str) -> Self {
        match tier {
            "pro" => Self::pro(),
            _ => Self::free(),
        }
    }
}

/// Tracks per-key usage for rate limiting.
pub struct RateLimiter {
    usage: DashMap<String, KeyUsage>,
    /// Per-IP API-key creation throttle (gates open signup against key-spam).
    signups: DashMap<String, SignupWindow>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            usage: DashMap::new(),
            signups: DashMap::new(),
        }
    }

    /// Record a key-creation attempt from `ip` and return whether it's allowed.
    /// Counts against a per-IP window; denies once the window cap is hit.
    pub fn try_register_signup(&self, ip: &str) -> bool {
        let mut w = self
            .signups
            .entry(ip.to_string())
            .or_insert_with(|| SignupWindow {
                count: 0,
                window_start: Instant::now(),
            });
        if w.window_start.elapsed() > SIGNUP_WINDOW {
            w.count = 0;
            w.window_start = Instant::now();
        }
        if w.count >= SIGNUP_MAX_PER_WINDOW {
            return false;
        }
        w.count += 1;
        true
    }

    fn get_or_create(&self, api_key: &str) -> dashmap::mapref::one::Ref<'_, String, KeyUsage> {
        if !self.usage.contains_key(api_key) {
            self.usage.insert(api_key.to_string(), KeyUsage::new());
        }
        self.usage.get(api_key).unwrap()
    }

    pub fn check_agent_limit(&self, api_key: &str, limits: &TierLimits) -> bool {
        let usage = self.get_or_create(api_key);
        usage.agent_count.load(Ordering::Relaxed) < limits.max_agents
    }

    pub fn check_message_rate(&self, api_key: &str, limits: &TierLimits) -> bool {
        let usage = self.get_or_create(api_key);
        let mut window = usage.msg_window_start.lock().unwrap();

        // Reset window if older than 1 minute
        if window.elapsed() > Duration::from_secs(60) {
            *window = Instant::now();
            usage.msg_count.store(0, Ordering::Relaxed);
        }

        usage.msg_count.load(Ordering::Relaxed) < limits.max_messages_per_minute
    }

    pub fn check_room_limit(&self, api_key: &str, limits: &TierLimits) -> bool {
        let usage = self.get_or_create(api_key);
        usage.room_count.load(Ordering::Relaxed) < limits.max_rooms
    }

    pub fn increment_message(&self, api_key: &str) {
        let usage = self.get_or_create(api_key);
        usage.msg_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_agent(&self, api_key: &str) {
        let usage = self.get_or_create(api_key);
        usage.agent_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn remove_agent(&self, api_key: &str) {
        if let Some(usage) = self.usage.get(api_key) {
            let current = usage.agent_count.load(Ordering::Relaxed);
            if current > 0 {
                usage.agent_count.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    pub fn add_room(&self, api_key: &str) {
        let usage = self.get_or_create(api_key);
        usage.room_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn remove_room(&self, api_key: &str) {
        if let Some(usage) = self.usage.get(api_key) {
            let current = usage.room_count.load(Ordering::Relaxed);
            if current > 0 {
                usage.room_count.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signup_throttle_caps_per_ip() {
        let rl = RateLimiter::new();
        for _ in 0..SIGNUP_MAX_PER_WINDOW {
            assert!(rl.try_register_signup("1.2.3.4"));
        }
        // Over the cap for this IP.
        assert!(!rl.try_register_signup("1.2.3.4"));
        // A different IP has its own budget.
        assert!(rl.try_register_signup("5.6.7.8"));
    }
}
