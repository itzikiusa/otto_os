//! Design Hall glue (the graph itself lives in `otto-design`): the per-request
//! service handle, the host-owned deep format validation, and the startup
//! import that mirrors legacy Product-arena attachments + Canvas scenes into
//! the graph (idempotent; legacy rows/routes untouched).

use otto_core::{Error, Result};

use crate::state::ServerCtx;

/// A `DesignService` over the shared pool, `<data>/design` and the event bus.
/// Cheap (clones two handles) — built per request instead of adding a
/// ServerCtx field.
pub fn service(ctx: &ServerCtx) -> otto_design::DesignService {
    otto_design::DesignService::new(
        ctx.pool.clone(),
        ctx.data_dir.clone(),
        Some(ctx.events.clone()),
    )
}

/// Deep validation the design crate can't do itself: `scene3d` documents go
/// through the same schema check as the Product arena's content PUT
/// (`design_scene3d::validate_bytes`). Other formats get only otto-design's
/// own checks.
pub fn validate_content(format: &str, bytes: &[u8]) -> Result<()> {
    match format {
        "scene3d" => crate::design_scene3d::validate_bytes(bytes)
            .map(|_| ())
            .map_err(|e| match e {
                Error::Invalid(m) => Error::Invalid(m),
                other => Error::Invalid(other.to_string()),
            }),
        _ => Ok(()),
    }
}

/// Boot hook: build the FTS index and run the idempotent legacy import in the
/// background (never blocks or fails startup; the outcome is logged).
pub fn spawn_startup_import(ctx: &ServerCtx) {
    let svc = service(ctx);
    tokio::spawn(otto_design::import::startup(svc));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_content_is_schema_validated() {
        assert!(validate_content(
            "scene3d",
            br#"{"type":"otto-scene3d","version":1,"objects":[]}"#
        )
        .is_ok());
        assert!(validate_content("scene3d", br#"{"type":"nope","version":1}"#).is_err());
        assert!(validate_content("scene3d", b"not json").is_err());
        // Other formats are left to otto-design's own checks.
        assert!(validate_content("html", b"<p>x</p>").is_ok());
    }
}
