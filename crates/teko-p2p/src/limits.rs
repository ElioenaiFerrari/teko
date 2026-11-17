//! Connection caps, rate limits and bans (ADR-0048). Pure logic with the clock passed in, so the
//! rules are testable without sleeping; the host decides which socket each rule guards.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Counts {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
    per_key: HashMap<Vec<u8>, usize>,
}

/// Caps on simultaneous connections: in total, per remote IP and per authenticated key.
#[derive(Clone)]
pub struct ConnLimiter {
    counts: Arc<Mutex<Counts>>,
    max_total: usize,
    max_per_ip: usize,
    max_per_key: usize,
}

/// A connection slot: freed when dropped.
pub struct ConnGuard {
    counts: Arc<Mutex<Counts>>,
    ip: IpAddr,
    key: Option<Vec<u8>>,
    max_per_key: usize,
}

fn lock(m: &Mutex<Counts>) -> MutexGuard<'_, Counts> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ConnLimiter {
    pub fn new(max_total: usize, max_per_ip: usize, max_per_key: usize) -> Self {
        Self {
            counts: Arc::default(),
            max_total,
            max_per_ip,
            max_per_key,
        }
    }

    /// A slot for a connection from `ip`, or `None` when the node or that IP is full.
    pub fn try_acquire(&self, ip: IpAddr) -> Option<ConnGuard> {
        let mut c = lock(&self.counts);
        if c.total >= self.max_total || c.per_ip.get(&ip).copied().unwrap_or(0) >= self.max_per_ip {
            return None;
        }
        c.total += 1;
        *c.per_ip.entry(ip).or_default() += 1;
        Some(ConnGuard {
            counts: self.counts.clone(),
            ip,
            key: None,
            max_per_key: self.max_per_key,
        })
    }

    pub fn open(&self) -> usize {
        lock(&self.counts).total
    }
}

impl ConnGuard {
    /// After the handshake: counts this connection against the peer's key. `false` when that
    /// institution already has its share of connections open (the caller drops this one).
    pub fn bind_key(&mut self, key: &[u8]) -> bool {
        let mut c = lock(&self.counts);
        if c.per_key.get(key).copied().unwrap_or(0) >= self.max_per_key {
            return false;
        }
        *c.per_key.entry(key.to_vec()).or_default() += 1;
        self.key = Some(key.to_vec());
        true
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut c = lock(&self.counts);
        c.total = c.total.saturating_sub(1);
        if let Some(n) = c.per_ip.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.per_ip.remove(&self.ip);
            }
        }
        if let Some(k) = self.key.take()
            && let Some(n) = c.per_key.get_mut(&k)
        {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.per_key.remove(&k);
            }
        }
    }
}

/// A token bucket: `capacity` tokens, refilled at `per_sec`.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(capacity: u32, per_sec: u32, now: Instant) -> Self {
        Self {
            capacity: capacity as f64,
            tokens: capacity as f64,
            per_sec: per_sec as f64,
            last: now,
        }
    }

    /// Spends `n` tokens if there are that many.
    pub fn try_take(&mut self, n: u32, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.per_sec).min(self.capacity);
        if self.tokens >= n as f64 {
            self.tokens -= n as f64;
            true
        } else {
            false
        }
    }
}

/// Bad-behaviour points per subject (an IP or a consensus key); reaching the threshold bans it for a
/// while. Points fade: a subject that behaves again starts clean.
pub struct Reputation {
    threshold: u32,
    ban_for: Duration,
    forget_after: Duration,
    strikes: HashMap<Vec<u8>, (u32, Instant)>,
    bans: HashMap<Vec<u8>, Instant>,
}

impl Reputation {
    pub fn new(threshold: u32, ban_for: Duration, forget_after: Duration) -> Self {
        Self {
            threshold,
            ban_for,
            forget_after,
            strikes: HashMap::new(),
            bans: HashMap::new(),
        }
    }

    /// Adds `points`; `true` when the subject is now banned.
    pub fn strike(&mut self, who: &[u8], points: u32, now: Instant) -> bool {
        let e = self.strikes.entry(who.to_vec()).or_insert((0, now));
        if now.saturating_duration_since(e.1) > self.forget_after {
            e.0 = 0;
        }
        e.0 = e.0.saturating_add(points);
        e.1 = now;
        if e.0 >= self.threshold {
            self.strikes.remove(who);
            self.bans.insert(who.to_vec(), now + self.ban_for);
            return true;
        }
        false
    }

    pub fn is_banned(&mut self, who: &[u8], now: Instant) -> bool {
        match self.bans.get(who) {
            Some(until) if now < *until => true,
            Some(_) => {
                self.bans.remove(who);
                false
            }
            None => false,
        }
    }

    /// Bytes that name an IP address as a subject.
    pub fn ip_subject(ip: IpAddr) -> Vec<u8> {
        match ip {
            IpAddr::V4(a) => a.octets().to_vec(),
            IpAddr::V6(a) => a.octets().to_vec(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    #[test]
    fn connections_are_capped_in_total_per_ip_and_per_key_and_slots_come_back() {
        let l = ConnLimiter::new(4, 2, 1);
        let a = l.try_acquire(ip(1)).unwrap();
        let mut b = l.try_acquire(ip(1)).unwrap();
        assert!(l.try_acquire(ip(1)).is_none(), "per IP");
        let c = l.try_acquire(ip(2)).unwrap();
        let _d = l.try_acquire(ip(3)).unwrap();
        assert!(l.try_acquire(ip(4)).is_none(), "in total");
        assert_eq!(l.open(), 4);
        // per key: one connection per institution
        assert!(b.bind_key(&[7; 33]));
        let mut c = c;
        assert!(!c.bind_key(&[7; 33]));
        drop(a);
        assert!(l.try_acquire(ip(1)).is_some(), "a freed slot is reusable");
        drop(b);
        assert!(c.bind_key(&[7; 33]), "the key's slot is free again");
    }

    #[test]
    fn a_bucket_allows_a_burst_then_the_refill_rate() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(10, 5, t0);
        assert!(b.try_take(10, t0));
        assert!(!b.try_take(1, t0), "empty");
        assert!(!b.try_take(2, t0 + Duration::from_millis(300)));
        assert!(b.try_take(2, t0 + Duration::from_millis(700)));
        // never more than the capacity, however long it rests
        assert!(b.try_take(10, t0 + Duration::from_secs(3_600)));
        assert!(!b.try_take(1, t0 + Duration::from_secs(3_600)));
    }

    #[test]
    fn enough_strikes_ban_for_a_while_and_old_strikes_fade() {
        let t0 = Instant::now();
        let mut r = Reputation::new(10, Duration::from_secs(600), Duration::from_secs(60));
        let who = Reputation::ip_subject(ip(9));
        assert!(!r.strike(&who, 4, t0));
        assert!(!r.strike(&who, 4, t0 + Duration::from_secs(1)));
        assert!(r.strike(&who, 4, t0 + Duration::from_secs(2)));
        assert!(r.is_banned(&who, t0 + Duration::from_secs(300)));
        assert!(
            !r.is_banned(&who, t0 + Duration::from_secs(700)),
            "the ban expires"
        );
        // strikes spread over time fade and never add up to a ban
        let other = Reputation::ip_subject(ip(8));
        for i in 0..20 {
            assert!(!r.strike(&other, 4, t0 + Duration::from_secs(100 * i)));
        }
        assert!(!r.is_banned(&other, t0 + Duration::from_secs(2_000)));
    }
}
