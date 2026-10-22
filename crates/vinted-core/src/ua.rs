//! Browser-like User-Agent pool.
//!
//! Vinted's anti-bot evaluates request headers, so we send a realistic,
//! modern UA. The value is stable per session (randomized once) to mimic a
//! returning browser rather than a rotating bot.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
];

/// Pick a UA deterministically-random for this process.
pub fn random_user_agent() -> &'static str {
    static PICKED: OnceLock<usize> = OnceLock::new();
    let idx = *PICKED.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        nanos % USER_AGENTS.len()
    });
    USER_AGENTS[idx]
}
