//! Agent self-improvement.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/core.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Agent self-improvement
// ---------------------------------------------------------------------------

/// Per-workspace self-reflection config (stored under
/// `Workspace.settings.self_improvement`). `last_run_at`/`next_run_at` are
/// scheduler-managed; the rest are user-editable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfImprovementConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cadence_minutes")]
    pub cadence_minutes: u32,
    #[serde(default = "default_lookback_hours")]
    pub lookback_hours: u32,
    #[serde(default)]
    pub skill_allowlist: Vec<String>,
    #[serde(default)]
    pub autonomy: crate::domain::Autonomy,
    /// Agent CLIs to run the analysis on. Each runs independently with its own
    /// default model, so you get a separate set of suggestions per provider.
    /// Defaults to `["claude"]`.
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    /// When true, the in-loop evolver watches this workspace's live agent
    /// sessions and improves the skills they use right after each interaction.
    #[serde(default)]
    pub live_evolve: bool,
    #[serde(default)]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub next_run_at: Option<DateTime<Utc>>,
}

pub(super) fn default_cadence_minutes() -> u32 {
    60
}
pub(super) fn default_lookback_hours() -> u32 {
    24
}
pub(super) fn default_providers() -> Vec<String> {
    vec!["claude".to_string()]
}

impl Default for SelfImprovementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cadence_minutes: default_cadence_minutes(),
            lookback_hours: default_lookback_hours(),
            skill_allowlist: Vec::new(),
            autonomy: crate::domain::Autonomy::default(),
            providers: default_providers(),
            live_evolve: false,
            last_run_at: None,
            next_run_at: None,
        }
    }
}

/// `PUT /workspaces/{id}/self-improvement` — user-editable fields only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSelfImprovementReq {
    pub enabled: bool,
    pub cadence_minutes: u32,
    pub lookback_hours: u32,
    pub skill_allowlist: Vec<String>,
    pub autonomy: crate::domain::Autonomy,
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    #[serde(default)]
    pub live_evolve: bool,
}

/// `POST /workspaces/{id}/self-improvement/run`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunNowResp {
    pub run_id: Id,
}
