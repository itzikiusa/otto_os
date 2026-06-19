//! Preset swarm templates: 5 embedded YAML org charts (paperclip-style, but
//! focused), instantiated into real swarm rows on "create from preset". Provider
//! auto-detection (multica-style): each template provider is mapped to an
//! installed CLI, falling back to the workspace default — so a preset never
//! creates an agent on a missing provider.

use include_dir::{include_dir, Dir};
use otto_core::{Id, Result};
use otto_state::{NewAgent, NewProject, Swarm, SwarmPatch, SwarmRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::types::{PresetAgent, SwarmPreset};

static PRESETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/presets");

#[derive(Debug, Deserialize)]
struct PresetFile {
    slug: String,
    name: String,
    description: String,
    #[serde(default = "default_cap")]
    max_parallel_sessions: i64,
    // Budget guardrails (D3). Absent → the shared defaults below (non-null so a
    // runaway swarm self-stops); set explicitly to a YAML `null` for unlimited.
    #[serde(default = "default_max_total_runs")]
    max_total_runs: Option<i64>,
    #[serde(default = "default_max_runtime_secs")]
    max_runtime_secs: Option<i64>,
    #[serde(default)]
    max_cost_usd: Option<f64>,
    #[serde(default = "default_max_attempts")]
    max_attempts: Option<i64>,
    #[serde(default)]
    projects: Vec<PresetProjectDef>,
    agents: Vec<PresetAgentDef>,
}

fn default_cap() -> i64 {
    3
}

fn default_max_total_runs() -> Option<i64> {
    Some(crate::service::DEFAULT_MAX_TOTAL_RUNS)
}

fn default_max_runtime_secs() -> Option<i64> {
    Some(crate::service::DEFAULT_MAX_RUNTIME_SECS)
}

fn default_max_attempts() -> Option<i64> {
    Some(crate::service::DEFAULT_MAX_ATTEMPTS)
}

#[derive(Debug, Deserialize)]
struct PresetProjectDef {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    goal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PresetSkillDef {
    name: String,
    #[serde(default)]
    must_use: bool,
}

#[derive(Debug, Deserialize)]
struct PresetAgentDef {
    key: String,
    name: String,
    title: String,
    provider: String,
    #[serde(default)]
    reports_to: Option<String>,
    #[serde(default)]
    specialization: String,
    #[serde(default)]
    soul: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    avatar: String,
    #[serde(default)]
    skills: Vec<PresetSkillDef>,
    #[serde(default)]
    schedule: Option<Value>,
}

fn parse_all() -> Vec<PresetFile> {
    PRESETS
        .files()
        .filter(|f| f.path().extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false))
        .filter_map(|f| f.contents_utf8())
        .filter_map(|s| match serde_yaml::from_str::<PresetFile>(s) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!("swarm preset parse: {e}");
                None
            }
        })
        .collect()
}

/// Public summaries for the preset picker.
pub fn list_presets() -> Vec<SwarmPreset> {
    let mut out: Vec<SwarmPreset> = parse_all()
        .into_iter()
        .map(|p| SwarmPreset {
            slug: p.slug,
            name: p.name,
            description: p.description,
            max_parallel_sessions: p.max_parallel_sessions,
            max_total_runs: p.max_total_runs,
            max_runtime_secs: p.max_runtime_secs,
            max_cost_usd: p.max_cost_usd,
            max_attempts: p.max_attempts,
            agents: p
                .agents
                .into_iter()
                .map(|a| PresetAgent {
                    key: a.key,
                    name: a.name,
                    title: a.title,
                    reports_to: a.reports_to,
                    provider: a.provider,
                    specialization: a.specialization,
                })
                .collect(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Map a template provider to an installed one, else the default.
fn map_provider(provider: &str, available: &[String], default: &str) -> String {
    if available.iter().any(|p| p == provider) {
        provider.to_string()
    } else {
        default.to_string()
    }
}

/// Instantiate a preset's projects + agents into an existing (blank) swarm.
pub async fn instantiate(
    repo: &SwarmRepo,
    swarm: &Swarm,
    user: &Id,
    slug: &str,
    available_providers: &[String],
    default_provider: &str,
) -> Result<()> {
    let Some(preset) = parse_all().into_iter().find(|p| p.slug == slug) else {
        tracing::warn!("swarm preset '{slug}' not found");
        return Ok(());
    };

    // Set the swarm's parallel cap from the preset (keep other config defaults),
    // and apply the preset's budget guardrails to the swarm row.
    let mut config = swarm.config.clone();
    if let Some(obj) = config.as_object_mut() {
        obj.insert("max_parallel_sessions".into(), json!(preset.max_parallel_sessions));
        obj.entry("cwd_mode").or_insert(json!("scratch"));
    }
    let _ = repo
        .update_swarm(
            &swarm.id,
            SwarmPatch {
                config: Some(config),
                max_total_runs: Some(preset.max_total_runs),
                max_runtime_secs: Some(preset.max_runtime_secs),
                max_cost_usd: Some(preset.max_cost_usd),
                max_attempts: Some(preset.max_attempts),
                ..Default::default()
            },
        )
        .await;

    // Projects.
    for (i, p) in preset.projects.iter().enumerate() {
        let _ = repo
            .create_project(NewProject {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                name: p.name.clone(),
                description: p.description.clone(),
                repo_path: None,
                goal_md: p.goal.clone(),
                order_idx: i as i64,
                created_by: user.clone(),
            })
            .await;
    }

    // Agents — pass 1: create without reports_to, remember key → id.
    let mut key_to_id: std::collections::HashMap<String, Id> = std::collections::HashMap::new();
    for (i, a) in preset.agents.iter().enumerate() {
        let provider = map_provider(&a.provider, available_providers, default_provider);
        let skills = json!(a
            .skills
            .iter()
            .map(|s| json!({"name": s.name, "must_use": s.must_use}))
            .collect::<Vec<_>>());
        let soul_md = if a.soul.trim().is_empty() { None } else { Some(a.soul.clone()) };
        match repo
            .create_agent(NewAgent {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                name: a.name.clone(),
                title: a.title.clone(),
                reports_to: None,
                provider,
                model: None,
                soul_name: None,
                soul_md,
                specialization: a.specialization.clone(),
                scope_md: a.scope.clone(),
                skills,
                schedule: a.schedule.clone(),
                cwd_mode: None,
                avatar: a.avatar.clone(),
                order_idx: i as i64,
                created_by: user.clone(),
            })
            .await
        {
            Ok(created) => {
                key_to_id.insert(a.key.clone(), created.id);
            }
            Err(e) => tracing::warn!("swarm preset agent '{}': {e}", a.key),
        }
    }

    // Pass 2: wire reports_to by key.
    for a in &preset.agents {
        let (Some(id), Some(mgr_key)) = (key_to_id.get(&a.key), a.reports_to.as_ref()) else {
            continue;
        };
        if let Some(mgr_id) = key_to_id.get(mgr_key) {
            let _ = repo
                .update_agent(
                    id,
                    otto_state::AgentPatch {
                        reports_to: Some(Some(mgr_id.clone())),
                        ..Default::default()
                    },
                )
                .await;
        }
    }

    Ok(())
}

/// Seed swarm role skills/souls into the library on startup (only if absent).
/// Preset agents use inline souls, so this is currently a no-op; kept as the
/// hook for future library-backed swarm skills.
pub fn seed(_library: &otto_context::Library) {}
