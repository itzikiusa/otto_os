//! Capability primitives shared by the two in-memory relays: session rooms
//! (`rooms/`) and game rooms (`game_rooms/`). Both hand out a random 32-byte
//! token (base64url, 43 chars), keep only its SHA-256 digest, accept it over
//! the `Sec-WebSocket-Protocol` header and throttle each member with a token
//! bucket. One copy keeps the hardening from drifting between the relays:
//! every digest compare is constant-time by construction, every token must
//! be exactly `TOKEN_LEN`, and the subprotocol pair is parsed one way.
use axum::http::HeaderMap;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use otto_core::{Error, Result};
use rand::{rand_core::UnwrapErr, rngs::SysRng, Rng};
use sha2::{Digest as _, Sha256};
use std::{
    hash::{Hash, Hasher},
    time::Instant,
};

/// 32 random bytes, base64url without padding.
pub(crate) const TOKEN_LEN: usize = 43;

/// A capability's stored form. `PartialEq` is the constant-time fold, so a
/// `==` on two digests (or a `matches`) never short-circuits on the first
/// differing byte and cannot become a token-prefix oracle.
#[derive(Clone, Copy, Eq)]
pub(crate) struct Digest([u8; 32]);
impl Digest {
    pub fn of(token: &str) -> Self {
        Self(Sha256::digest(token.as_bytes()).into())
    }
    /// True when `token` is the capability this digest was issued for. A
    /// malformed token is rejected before hashing.
    pub fn matches(&self, token: &str) -> bool {
        well_formed(token) && *self == Self::of(token)
    }
}
impl PartialEq for Digest {
    fn eq(&self, other: &Self) -> bool {
        // All digests have equal length. Accumulating XOR prevents an
        // early-byte comparison from making a useful token prefix oracle.
        let diff = self
            .0
            .iter()
            .zip(other.0)
            .fold(0u8, |acc, (x, y)| acc | (x ^ y));
        std::hint::black_box(diff) == 0
    }
}
// Keyed lookups (the session room's invite map) hash the digest itself; a
// SHA-256 image leaks nothing useful about the token through bucket timing.
impl Hash for Digest {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// Issues fresh capabilities.
pub(crate) struct Capability;
impl Capability {
    /// A new random token and the digest to store in its place.
    pub fn issue() -> (String, Digest) {
        let mut bytes = [0u8; 32];
        UnwrapErr(SysRng).fill_bytes(&mut bytes);
        let token = URL_SAFE_NO_PAD.encode(bytes);
        let digest = Digest::of(&token);
        (token, digest)
    }
}

/// Every issued token has this exact length; anything else is refused before
/// a lock or a hash.
pub(crate) fn well_formed(token: &str) -> bool {
    token.len() == TOKEN_LEN
}

/// A per-member token bucket. `take` refills at `per_second` up to `burst`
/// and spends one token, or fails with `Conflict(message)`. Each relay passes
/// its own message (the game socket treats its message as fatal).
pub(crate) struct TokenBucket {
    tokens: f64,
    at: Instant,
}
impl TokenBucket {
    pub fn new(burst: f64) -> Self {
        Self {
            tokens: burst,
            at: Instant::now(),
        }
    }
    /// Tests drive a long sequence without waiting for the refill.
    #[cfg(test)]
    pub fn refill(&mut self, tokens: f64) {
        self.tokens = tokens;
    }
    pub fn take(&mut self, per_second: f64, burst: f64, message: &str) -> Result<()> {
        self.take_at(Instant::now(), per_second, burst, message)
    }
    fn take_at(&mut self, now: Instant, per_second: f64, burst: f64, message: &str) -> Result<()> {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.at).as_secs_f64() * per_second)
            .min(burst);
        self.at = now;
        if self.tokens < 1.0 {
            return Err(Error::Conflict(message.into()));
        }
        self.tokens -= 1.0;
        Ok(())
    }
}

/// The capability presented as the `Sec-WebSocket-Protocol` pair
/// `<protocol>, <token>`. Exactly one header, exactly two entries, the named
/// protocol first and a well-formed token second; anything else (an owner
/// bearer, a query string token, extra entries) is `Unauthorized`.
pub(crate) fn socket_token(headers: &HeaderMap, protocol: &str) -> Result<String> {
    let mut values = headers.get_all("sec-websocket-protocol").iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized);
    }
    let parts: Vec<_> = value.split(',').map(str::trim).collect();
    if parts.len() != 2 || parts[0] != protocol || !well_formed(parts[1]) {
        return Err(Error::Unauthorized);
    }
    Ok(parts[1].into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn issued_tokens_are_unique_well_formed_and_match_only_their_digest() {
        let (a, da) = Capability::issue();
        let (b, db) = Capability::issue();
        assert_ne!(a, b);
        assert!(well_formed(&a) && well_formed(&b));
        assert!(da.matches(&a));
        assert!(!da.matches(&b));
        assert!(db.matches(&b));
        assert!(da == Digest::of(&a));
        assert!(da != db);
    }

    #[test]
    fn digest_compare_folds_every_byte() {
        let base = Digest([7u8; 32]);
        // A difference in the first, a middle or the last byte all compare unequal,
        // and equality needs every byte: the fold never stops early.
        for i in [0usize, 15, 31] {
            let mut other = base.0;
            other[i] ^= 1;
            assert!(base != Digest(other), "byte {i}");
        }
        assert!(base == Digest([7u8; 32]));
    }

    #[test]
    fn malformed_tokens_never_match() {
        let (token, digest) = Capability::issue();
        assert!(!digest.matches(""));
        assert!(!digest.matches(&token[..42]));
        assert!(!digest.matches(&format!("{token}x")));
    }

    #[test]
    fn token_bucket_spends_burst_then_refills_at_rate() {
        let start = Instant::now();
        let mut bucket = TokenBucket {
            tokens: 2.0,
            at: start,
        };
        assert!(bucket.take_at(start, 1.0, 2.0, "slow down").is_ok());
        assert!(bucket.take_at(start, 1.0, 2.0, "slow down").is_ok());
        let err = bucket.take_at(start, 1.0, 2.0, "slow down").unwrap_err();
        assert!(err.to_string().contains("slow down"));
        // Half a second refills half a token: still throttled.
        let half = start + Duration::from_millis(500);
        assert!(bucket.take_at(half, 1.0, 2.0, "slow down").is_err());
        let later = start + Duration::from_millis(1100);
        assert!(bucket.take_at(later, 1.0, 2.0, "slow down").is_ok());
        // A long idle refills only up to the burst.
        let idle = later + Duration::from_secs(60);
        assert!(bucket.take_at(idle, 1.0, 2.0, "x").is_ok());
        assert!(bucket.take_at(idle, 1.0, 2.0, "x").is_ok());
        assert!(bucket.take_at(idle, 1.0, 2.0, "x").is_err());
    }

    #[test]
    fn socket_token_requires_the_exact_protocol_pair() {
        let token = "a".repeat(TOKEN_LEN);
        let mut headers = HeaderMap::new();
        assert!(socket_token(&headers, "otto-room").is_err());
        headers.insert("authorization", "Bearer owner".parse().unwrap());
        assert!(socket_token(&headers, "otto-room").is_err());
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-room, {token}").parse().unwrap(),
        );
        assert_eq!(socket_token(&headers, "otto-room").unwrap(), token);
        // The other relay's protocol name is refused.
        assert!(socket_token(&headers, "otto-game").is_err());
        // A short token, extra entries, or a second header are refused.
        headers.insert("sec-websocket-protocol", "otto-room, abc".parse().unwrap());
        assert!(socket_token(&headers, "otto-room").is_err());
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-room, {token}, extra").parse().unwrap(),
        );
        assert!(socket_token(&headers, "otto-room").is_err());
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-room, {token}").parse().unwrap(),
        );
        headers.append("sec-websocket-protocol", "other".parse().unwrap());
        assert!(socket_token(&headers, "otto-room").is_err());
    }
}
