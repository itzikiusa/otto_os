//! otto-phone — Phone access: reach Otto from your own phone over Tailscale
//! with a QR pairing and a per-phone device token (docs/features/phone-access.md).
//!
//! * [`tailscale`] — detect Tailscale and turn `tailscale serve` on/off for
//!   Otto (never Funnel), behind the [`tailscale::TailscaleCli`] trait.
//! * [`routes`] — the `/phone/*` REST surface, generic over [`PhoneCtx`]; the
//!   storage is `otto_rbac::devices` (pairings + device tokens).
//! * [`policy`] — the feature-guard hooks: `/phone/*` exemption and the
//!   device-token narrowing ([`policy::device_gate`]).
//! * [`ua`] — a human device label from a `User-Agent`.

pub mod policy;
pub mod routes;
pub mod tailscale;
pub mod throttle;
pub mod ua;

use std::sync::Arc;

use otto_core::auth::{AuthContext, BoxFuture};
use otto_state::audit::NewAuditEntry;
use otto_state::DbPool;

pub use routes::{protected_router, public_router};

/// What the host (otto-server) hands the Phone access routes.
pub trait PhoneCtx: Clone + Send + Sync + 'static {
    fn pool(&self) -> DbPool;
    /// The Tailscale CLI (the real one in production, a fake in tests).
    fn tailscale(&self) -> Arc<dyn tailscale::TailscaleCli> {
        tailscale::system()
    }
    /// A person's own credential (not an agent session, MCP or share token).
    fn is_human(&self, auth: &AuthContext) -> bool;
    /// Best-effort audit append.
    fn audit(&self, entry: NewAuditEntry) -> BoxFuture<'_, ()>;
}
