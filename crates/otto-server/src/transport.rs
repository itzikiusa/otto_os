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
//! * **Boot restore.** `hello_ack` also carries what the boot restore did
//!   (`kept_running` / `suspended`), so the UI can say "Otto restarted — N kept
//!   running, M suspended" once per boot id.

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

/// Port of the network (0.0.0.0, TLS) listener `ottod` ACTUALLY bound —
/// `None` while the setting is off, before the restart that applies it, or
/// after its TLS setup / bind failed (S20-303). Share links read this, never
/// the saved setting, so a QR never points at an address nothing listens on.
static NETWORK_LISTENER_PORT: std::sync::Mutex<Option<u16>> = std::sync::Mutex::new(None);

/// Record (or clear) the bound network listener's port.
pub fn set_network_listener_port(port: Option<u16>) {
    *NETWORK_LISTENER_PORT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = port;
}

/// The bound network listener's port, when one is serving.
pub fn network_listener_port() -> Option<u16> {
    *NETWORK_LISTENER_PORT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

static BOOT_ID: LazyLock<String> = LazyLock::new(otto_core::new_id);

/// This daemon process's boot id (stable for its lifetime).
pub fn boot_id() -> &'static str {
    BOOT_ID.as_str()
}

/// What the boot restore did (review A4): sessions re-adopted from their PTY
/// holders vs sessions that lost their process with the previous daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct BootRestore {
    pub kept_running: usize,
    pub suspended: usize,
}

static BOOT_RESTORE: OnceLock<BootRestore> = OnceLock::new();

/// Record the boot restore summary. Set once by `ottod` after
/// `restore_all`; first call wins.
pub fn set_boot_restore(summary: BootRestore) {
    let _ = BOOT_RESTORE.set(summary);
}

/// The boot restore summary sent in `hello_ack` (`None` until recorded).
pub fn boot_restore() -> Option<BootRestore> {
    BOOT_RESTORE.get().copied()
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
