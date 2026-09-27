//! Webview↔daemon transport facts shared by routes (TRANSPORT_PLAN §3d/§2c).
//!
//! * **Loopback alias lane.** WebKit (and Chromium) cap HTTP/1.1 sockets per
//!   HOST — ~6, shared by every Otto window. `ottod` also listens on
//!   `[::1]:<port>` (still loopback-only), and `/meta` advertises
//!   `http://localhost:<port>` so the UI can send background polls and
//!   known-slow calls there: a second, physically separate pool that can never
//!   starve what the user clicked on `127.0.0.1`. `localhost` (not `[::1]`)
//!   because CSP host-sources cannot express IPv6 literals; it is safe only
//!   because this process then holds BOTH loopback addresses on the port. Set
//!   once by `ottod` only when the IPv6 bind succeeded — the UI never uses an
//!   alias the daemon did not claim (a squatter on `[::1]:<port>` would
//!   otherwise receive the bearer token).
//! * **Boot id.** Minted once per daemon process and sent in `hello_ack` /
//!   `subscribe_ack`, so a client can tell "reconnected to the same daemon"
//!   from "the daemon restarted" (every ephemeral id is then gone).

use std::sync::{LazyLock, OnceLock};

static ALT_LOOPBACK_BASE: OnceLock<String> = OnceLock::new();

/// Record the second loopback base (`http://[::1]:<port>`). First call wins.
pub fn set_alt_loopback_base(base: String) {
    let _ = ALT_LOOPBACK_BASE.set(base);
}

/// The advertised second loopback base, when `ottod` bound one.
pub fn alt_loopback_base() -> Option<String> {
    ALT_LOOPBACK_BASE.get().cloned()
}

static BOOT_ID: LazyLock<String> = LazyLock::new(otto_core::new_id);

/// This daemon process's boot id (stable for its lifetime).
pub fn boot_id() -> &'static str {
    BOOT_ID.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_id_is_stable_per_process() {
        assert!(!boot_id().is_empty());
        assert_eq!(boot_id(), boot_id());
    }
}
