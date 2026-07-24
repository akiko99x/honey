//! In-memory brute-force guard for the login endpoint. Keyed by client (IP):
//! after too many failures inside a window the key is locked out for a while.
//! Per-process state is fine — the master runs as a single service.
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::SystemTime;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;

const MAX_FAILURES: u32 = 5;
const WINDOW: Duration = Duration::from_secs(300); // 5 minutes
const LOCKOUT: Duration = Duration::from_secs(900); // 15 minutes

struct Entry {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

#[derive(Default)]
pub struct LoginLimiter {
    entries: Mutex<HashMap<String, Entry>>,
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// If the key is locked out, returns the seconds until it clears.
    pub async fn locked_for(&self, key: &str) -> Option<u64> {
        let mut map = self.entries.lock().await;
        prune(&mut map);
        let entry = map.get(key)?;
        match entry.locked_until {
            Some(until) if until > Instant::now() => {
                Some((until - Instant::now()).as_secs().saturating_add(1))
            }
            _ => None,
        }
    }

    pub async fn record_failure(&self, key: &str) {
        let now = Instant::now();
        let mut map = self.entries.lock().await;
        let entry = map.entry(key.to_string()).or_insert(Entry {
            failures: 0,
            window_start: now,
            locked_until: None,
        });
        // reset the window if it has elapsed and we're not currently locked.
        if entry.locked_until.map_or(true, |until| until <= now)
            && now.duration_since(entry.window_start) > WINDOW
        {
            entry.failures = 0;
            entry.window_start = now;
            entry.locked_until = None;
        }
        entry.failures += 1;
        if entry.failures >= MAX_FAILURES {
            entry.locked_until = Some(now + LOCKOUT);
        }
    }

    pub async fn record_success(&self, key: &str) {
        self.entries.lock().await.remove(key);
    }
}

/// Drop entries that are neither locked nor inside their failure window.
fn prune(map: &mut HashMap<String, Entry>) {
    let now = Instant::now();
    map.retain(|_, entry| {
        entry.locked_until.is_some_and(|until| until > now)
            || now.duration_since(entry.window_start) <= WINDOW
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionLimitConfig {
    pub enabled: bool,
    pub max_requests: u32,
    pub window: Duration,
    pub block: Duration,
}

impl Default for SubscriptionLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_requests: 120,
            window: Duration::from_secs(60),
            block: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitDecision {
    Allow,
    Block { retry_after: u64 },
}

struct SubscriptionEntry {
    requests: u32,
    window_start: Instant,
    blocked_until: Option<Instant>,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionLimitStats {
    pub allowed_total: u64,
    pub blocked_total: u64,
    pub active_buckets: usize,
    pub last_blocked_at: Option<i64>,
}

#[derive(Default)]
pub struct SubscriptionLimiter {
    entries: Mutex<HashMap<String, SubscriptionEntry>>,
    allowed_total: AtomicU64,
    blocked_total: AtomicU64,
    last_blocked_at: AtomicI64,
}

impl SubscriptionLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn check(&self, key: &str, config: SubscriptionLimitConfig) -> LimitDecision {
        if !config.enabled {
            self.allowed_total.fetch_add(1, Ordering::Relaxed);
            return LimitDecision::Allow;
        }
        let now = Instant::now();
        let mut entries = self.entries.lock().await;
        entries.retain(|_, entry| {
            entry.blocked_until.is_some_and(|until| until > now)
                || now.duration_since(entry.window_start) <= config.window
        });
        let entry = entries.entry(key.to_string()).or_insert(SubscriptionEntry {
            requests: 0,
            window_start: now,
            blocked_until: None,
        });
        if let Some(until) = entry.blocked_until.filter(|until| *until > now) {
            return self.block(until, now);
        }
        if now.duration_since(entry.window_start) > config.window {
            entry.requests = 0;
            entry.window_start = now;
            entry.blocked_until = None;
        }
        entry.requests = entry.requests.saturating_add(1);
        if entry.requests > config.max_requests {
            let until = now + config.block;
            entry.blocked_until = Some(until);
            return self.block(until, now);
        }
        self.allowed_total.fetch_add(1, Ordering::Relaxed);
        LimitDecision::Allow
    }

    fn block(&self, until: Instant, now: Instant) -> LimitDecision {
        self.blocked_total.fetch_add(1, Ordering::Relaxed);
        let unix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64;
        self.last_blocked_at.store(unix, Ordering::Relaxed);
        LimitDecision::Block {
            retry_after: (until - now).as_secs().saturating_add(1),
        }
    }

    pub async fn stats(&self) -> SubscriptionLimitStats {
        let entries = self.entries.lock().await;
        let last = self.last_blocked_at.load(Ordering::Relaxed);
        SubscriptionLimitStats {
            allowed_total: self.allowed_total.load(Ordering::Relaxed),
            blocked_total: self.blocked_total.load(Ordering::Relaxed),
            active_buckets: entries.len(),
            last_blocked_at: (last > 0).then_some(last),
        }
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;

    fn config(max_requests: u32) -> SubscriptionLimitConfig {
        SubscriptionLimitConfig {
            max_requests,
            ..SubscriptionLimitConfig::default()
        }
    }

    #[tokio::test]
    async fn subscription_limits_are_isolated_and_report_retry_after() {
        let limiter = SubscriptionLimiter::new();
        assert_eq!(
            limiter.check("client-a:user-a", config(2)).await,
            LimitDecision::Allow
        );
        assert_eq!(
            limiter.check("client-a:user-a", config(2)).await,
            LimitDecision::Allow
        );
        assert!(matches!(
            limiter.check("client-a:user-a", config(2)).await,
            LimitDecision::Block { retry_after } if retry_after > 0
        ));
        assert_eq!(
            limiter.check("client-a:user-b", config(2)).await,
            LimitDecision::Allow
        );
        assert_eq!(
            limiter.check("client-b:user-a", config(2)).await,
            LimitDecision::Allow
        );
        let stats = limiter.stats().await;
        assert_eq!(stats.blocked_total, 1);
        assert_eq!(stats.allowed_total, 4);
    }

    #[tokio::test]
    async fn disabled_subscription_guard_never_blocks() {
        let limiter = SubscriptionLimiter::new();
        let disabled = SubscriptionLimitConfig {
            enabled: false,
            max_requests: 0,
            ..SubscriptionLimitConfig::default()
        };
        for _ in 0..10 {
            assert_eq!(limiter.check("same", disabled).await, LimitDecision::Allow);
        }
        assert_eq!(limiter.stats().await.blocked_total, 0);
    }
}
