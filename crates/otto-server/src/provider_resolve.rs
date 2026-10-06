//! One place that answers "which agent provider should this run spawn?".
//!
//! Every daemon feature that launches an agent without the user having picked
//! one resolves through [`ServerCtx::resolve_provider`], so the precedence is
//! identical everywhere:
//!
//! ```text
//! requested  ->  workspace `default_provider`  ->  global `default_provider`  ->  "claude"
//! ```
//!
//! Before this module ~30 call sites hand-rolled the chain; several skipped the
//! workspace default (canvas preview, insights) and all of them silently
//! swallowed a settings-read failure. The pure precedence lives in
//! [`otto_core::provider`]; this module only adds the I/O (workspace + global
//! settings lookups) and surfaces DB errors to the caller.

use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::settings::DEFAULT_PROVIDER_KEY;

use crate::state::ServerCtx;

impl ServerCtx {
    /// The global `default_provider` setting (bare JSON string), `""` when
    /// unset or malformed. Read errors propagate.
    pub async fn global_default_provider(&self) -> Result<String> {
        let v = otto_state::SettingsRepo::new(self.pool.clone())
            .get(DEFAULT_PROVIDER_KEY)
            .await?;
        Ok(otto_core::provider::global_default(v.as_ref()).to_string())
    }

    /// Resolve the provider for an agent launch: `requested` (when non-blank)
    /// → `ws`'s per-workspace default → the global default → `"claude"`.
    ///
    /// `ws` is `None` only for genuinely workspace-less launches (plugin host
    /// runs, global review/eval config defaults). An explicit, non-blank
    /// `requested` short-circuits without touching the DB.
    pub async fn resolve_provider(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
    ) -> Result<String> {
        let requested = requested.map(str::trim).unwrap_or("");
        if !requested.is_empty() {
            return Ok(requested.to_string());
        }
        let ws_default = ws
            .map(|w| otto_core::provider::workspace_default(&w.settings))
            .unwrap_or("");
        let global = self.global_default_provider().await?;
        Ok(otto_core::provider::resolve_provider(&[
            ws_default,
            global.as_str(),
        ]))
    }

    /// [`Self::resolve_provider`] for callers holding only a workspace id. A
    /// missing workspace is an error (it used to be swallowed into "no
    /// workspace default").
    pub async fn resolve_provider_for_ws(
        &self,
        ws_id: &Id,
        requested: Option<&str>,
    ) -> Result<String> {
        let requested_t = requested.map(str::trim).unwrap_or("");
        if !requested_t.is_empty() {
            return Ok(requested_t.to_string());
        }
        let ws = self.workspaces.get(ws_id).await?;
        self.resolve_provider(Some(&ws), None).await
    }

    /// Best-effort variant for background paths that cannot fail the caller
    /// (schedulers, headless runs): resolution errors are LOGGED (not silently
    /// dropped) and fall back to [`otto_core::provider::FALLBACK_PROVIDER`].
    pub async fn resolve_provider_or_fallback(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
        site: &'static str,
    ) -> String {
        match self.resolve_provider(ws, requested).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(site, error = %e, "provider resolution failed; using fallback");
                otto_core::provider::FALLBACK_PROVIDER.to_string()
            }
        }
    }
}
