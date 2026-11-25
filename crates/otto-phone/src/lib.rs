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

/// Phones used in the last 10 minutes: a daemon restart drops their
/// connection until Otto is back (the restart dialog).
pub async fn restart_impact(pool: &DbPool) -> Vec<otto_core::restart::ImpactItem> {
    let names = otto_rbac::devices::DevicesRepo::new(pool.clone())
        .recently_used(600)
        .await
        .unwrap_or_default();
    let labels = (
        "phone used in the last 10 minutes",
        "phones used in the last 10 minutes",
    );
    vec![otto_core::restart::ImpactItem::named(
        false, "phones", labels, names,
    )]
}

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
