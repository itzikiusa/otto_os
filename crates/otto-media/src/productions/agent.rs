//! The production planner's agent turn: ONE turn of the production's agent
//! (the one picked on Create, else the workspace's default) through the
//! shared assist job (`otto_agent_run::assist`). The agent
//! writes `plan.json` in the production's scratch dir
//! (`<data>/productions/<id>/`, an agent work area of Otto's data folder:
//! `otto_sandbox::AGENT_DATA_SUBDIRS`); the file (or, failing that, the
//! reply) is the answer [`super::parse_plan_reply`] checks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use otto_agent_run::assist::{run_assist_job, AgentTurn, AssistOutcome, AssistSlots, AssistTarget};
use otto_agent_run::session_turn::SessionManager;
use otto_core::domain::{User, Workspace};
use otto_core::{Error, Id, Result};
use otto_sessions::provider_models::{probe_provider_models_cached, ModelProbe};
use otto_sessions::ProviderRegistry;
use serde_json::json;

use super::{AgentPick, ProductionPlanner};

/// A planning turn is short work; past this its session is stopped.
const PLAN_CAP: Duration = Duration::from_secs(10 * 60);
const PLAN_FILE: &str = "plan.json";

static SLOTS: LazyLock<AssistSlots> = LazyLock::new(AssistSlots::default);

pub struct AgentPlanner {
    sessions: Arc<SessionManager>,
    ws: Workspace,
    user: User,
    provider: String,
    /// `None`: the provider's default. The session's `meta.model`, which
    /// its launch turns into the CLI's model flag.
    model: Option<String>,
    /// `None`: the model's default. The session's `meta.effort`, which its
    /// launch turns into the CLI's effort flag.
    effort: Option<String>,
    root: PathBuf,
}

/// The planner the host hands out (`MediaCtx::production_planner`), once
/// [`check_agent`] passed: a pick no session could run is refused here (a
/// 400), never swapped for the default.
pub async fn agent_planner(
    sessions: Arc<SessionManager>,
    ws: Workspace,
    user: User,
    provider: String,
    pick: &AgentPick,
    data_dir: &Path,
) -> Result<Arc<dyn ProductionPlanner>> {
    let (model, effort) = (pick.model.as_deref(), pick.effort.as_deref());
    check_agent(sessions.providers(), &provider, model, effort).await?;
    Ok(Arc::new(AgentPlanner {
        sessions,
        ws,
        user,
        provider,
        model: pick.model.clone(),
        effort: pick.effort.clone(),
        root: planner_root(data_dir),
    }))
}

/// Refuse what a new agent session could not run as picked, as session
/// creation does: an unknown or turned-off provider; a model the provider's
/// catalog (what `/meta` lists, so what the pickers offer) lacks; and an
/// effort that is no level, or that the model (none picked: any of the
/// provider's) does not list.
pub async fn check_agent(
    reg: &ProviderRegistry,
    provider: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<()> {
    reg.ensure_enabled(provider)?;
    if !reg.is_enabled(provider) {
        return Err(Error::Invalid(format!("unknown provider '{provider}'")));
    }
    if model.is_none() && effort.is_none() {
        return Ok(());
    }
    let catalog = probe_provider_models_cached(&ModelProbe::for_registry(reg)).await;
    let models = catalog.get(provider).map(Vec::as_slice).unwrap_or_default();
    let picked: Vec<_> = models
        .iter()
        .filter(|m| model.is_none_or(|id| m.id == id))
        .collect();
    if let (Some(model), true) = (model, picked.is_empty()) {
        return Err(Error::Invalid(format!(
            "{provider} has no model '{model}' — pick one it lists"
        )));
    }
    match effort {
        Some(e)
            if !picked
                .iter()
                .any(|m| m.supported_efforts.iter().any(|s| s == e)) =>
        {
            Err(Error::Invalid(format!(
                "{provider} {} has no effort '{e}' — pick one it lists",
                model.unwrap_or("models")
            )))
        }
        _ => Ok(()),
    }
}

/// Where planner turns run: like every engine's agent sessions, a folder of
/// its own under the data dir that the session guard lists as a work area
/// (a cwd anywhere else in the data dir is refused before the agent starts).
pub fn planner_root(data_dir: &Path) -> PathBuf {
    data_dir.join("productions")
}

#[async_trait]
impl ProductionPlanner for AgentPlanner {
    async fn run(&self, production_id: &str, prompt: String) -> Result<String> {
        let dir = otto_core::paths::confine_join(&self.root, production_id)
            .ok_or_else(|| Error::Invalid(format!("unsafe production id {production_id}")))?;
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::Internal(format!("production scratch dir: {e}")))?;
        let dir_str = dir.to_string_lossy().to_string();
        otto_sessions::trust::ensure_trusted_scratch(&self.provider, &self.root, &dir_str);
        let claim = LazyLock::force(&SLOTS)
            .claim(production_id)
            .ok_or_else(|| Error::Conflict("this production is already planning".into()))?;
        let turn = PlanTurn {
            planner: self,
            key: production_id.to_string(),
            file: dir.join(PLAN_FILE),
            dir_str,
            prompt,
        };
        let outcome = run_assist_job(&self.sessions, &turn, &claim).await;
        end_session(&self.sessions, &outcome).await;
        match outcome {
            AssistOutcome::Applied { applied, .. } => Ok(applied),
            AssistOutcome::Stopped { .. } => Err(Error::Conflict("planning was stopped".into())),
            AssistOutcome::Failed { error, .. } => salvage(&turn.file, error).await,
        }
    }
}

/// A turn reported failed while its agent wrote the plan anyway (the
/// first planner turn of a fresh daemon: "agent never accepted the prompt"
/// five seconds after the 16-item plan was written): the plan stands; it is
/// checked like any other.
async fn salvage(file: &Path, error: Error) -> Result<String> {
    match tokio::fs::read_to_string(file).await {
        Ok(plan) if !plan.trim().is_empty() => {
            tracing::warn!("production planner: {error}; the plan it wrote is used");
            Ok(plan)
        }
        _ => Err(error),
    }
}

/// A planning turn is one-shot: its session ends with it, whatever the
/// outcome (each plan or steer used to leave an idle agent running, even
/// past the daemon's own shutdown).
async fn end_session<A>(sessions: &SessionManager, outcome: &AssistOutcome<A>) {
    let session = match outcome {
        AssistOutcome::Applied { session, .. } => Some(session),
        AssistOutcome::Stopped { session } | AssistOutcome::Failed { session, .. } => {
            session.as_ref()
        }
    };
    if let Some(sid) = session {
        if let Err(e) = sessions.kill_session(sid).await {
            tracing::warn!(session = %sid, "production planner: end session: {e}");
        }
    }
}

struct PlanTurn<'a> {
    planner: &'a AgentPlanner,
    key: Id,
    file: PathBuf,
    dir_str: String,
    prompt: String,
}

impl AssistTarget for PlanTurn<'_> {
    type Proposal = String;
    type Applied = String;

    fn busy_key(&self) -> &str {
        &self.key
    }

    async fn load(&self) -> Result<()> {
        // A plan left by an earlier turn must never pass for this one's.
        let _ = tokio::fs::remove_file(&self.file).await;
        Ok(())
    }

    fn prompt(&self) -> String {
        self.prompt.clone()
    }

    fn turn<'b>(&'b self, prompt: &'b str) -> AgentTurn<'b> {
        AgentTurn {
            ws: &self.planner.ws,
            user: &self.planner.user,
            existing: None,
            title: "Smart mode plan",
            cwd: &self.dir_str,
            provider: &self.planner.provider,
            meta: json!({
                "source": "production_plan",
                "production_id": self.key,
                "model": self.planner.model,
                "effort": self.planner.effort,
            }),
            prompt,
            stuck_after: PLAN_CAP,
            done_file: Some(self.file.clone()),
            quiet_done: None,
        }
    }

    fn turn_cap(&self) -> Option<Duration> {
        Some(PLAN_CAP)
    }

    fn live_cap(&self) -> usize {
        512 * 1024
    }

    async fn parse(&self, reply: &str) -> Result<String> {
        match tokio::fs::read_to_string(&self.file).await {
            Ok(s) if !s.trim().is_empty() => Ok(s),
            _ => Ok(reply.to_string()),
        }
    }

    async fn apply(&self, plan: String, _session: &Id) -> Result<String> {
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed turn keeps a plan its agent wrote; without one the error stands.
    #[tokio::test]
    async fn a_plan_written_before_a_failed_turn_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(PLAN_FILE);
        let failed = || Error::Upstream("agent never accepted the prompt".into());
        assert!(salvage(&file, failed()).await.is_err());
        tokio::fs::write(&file, "{\"title\": \"Pier\"}")
            .await
            .unwrap();
        assert_eq!(
            salvage(&file, failed()).await.unwrap(),
            "{\"title\": \"Pier\"}"
        );
    }

    /// The session manager refuses an agent cwd inside Otto's data folder
    /// unless it is a listed work area: the planner's folder (create and steer
    /// share it) passes the real guard, and the old `media/productions` did not.
    /// Acceptance defect 10: the planner's session is ended after its turn,
    /// whether the plan was applied, stopped or failed.
    #[tokio::test]
    async fn a_planner_turn_ends_its_session() {
        use otto_agent_run::assist::Stage;
        use otto_core::domain::{SessionKind, SessionStatus};
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("state.db"))
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("u", "x", "U", false)
            .await
            .unwrap();
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .create("w", dir.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let repo = otto_state::SessionsRepo::new(pool);
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let sessions = SessionManager::new(
            repo.clone(),
            events,
            otto_sessions::ProviderRegistry::new(None),
        );
        for n in 0..3 {
            let req = otto_core::api::CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("Smart mode plan".into()),
                cwd: Some(dir.path().to_string_lossy().into()),
                connection_id: None,
                model: None,
                meta: None,
            };
            let sid = sessions.create(&ws, &user.id, req, None).await.unwrap().id;
            assert_ne!(repo.get(&sid).await.unwrap().status, SessionStatus::Exited);
            let outcome = match n {
                0 => AssistOutcome::Applied {
                    applied: String::new(),
                    session: sid.clone(),
                },
                1 => AssistOutcome::Stopped {
                    session: Some(sid.clone()),
                },
                _ => AssistOutcome::Failed {
                    stage: Stage::Turn,
                    error: Error::Upstream("no plan".into()),
                    session: Some(sid.clone()),
                },
            };
            end_session(&sessions, &outcome).await;
            assert_eq!(
                repo.get(&sid).await.unwrap().status,
                SessionStatus::Exited,
                "{n}"
            );
        }
    }

    /// A pick no session could run is refused before anything starts: an
    /// unknown or turned-off provider, a model its catalog does not list. A
    /// listed model and the provider's default pass.
    #[tokio::test]
    async fn check_agent_refuses_what_a_session_could_not_run() {
        let reg = ProviderRegistry::new(None);
        reg.set_disabled(&["codex"]);
        let refused = |r: Result<()>| match r {
            Err(Error::Invalid(m)) => m,
            other => panic!("not a 400: {other:?}"),
        };
        assert!(refused(check_agent(&reg, "nope", None, None).await).contains("unknown provider"));
        assert!(refused(check_agent(&reg, "codex", None, None).await).contains("turned off"));
        let m = refused(check_agent(&reg, "claude", Some("no-such-model"), None).await);
        assert!(m.contains("no model 'no-such-model'"), "{m}");
        check_agent(&reg, "claude", None, None).await.unwrap();
        let catalog = probe_provider_models_cached(&ModelProbe::for_registry(&reg)).await;
        let listed = &catalog["claude"][0];
        check_agent(&reg, "claude", Some(&listed.id), None)
            .await
            .unwrap();
        // An effort the model lists passes, with or without the model; a
        // made-up one is refused.
        let effort = listed.supported_efforts[0].as_str();
        check_agent(&reg, "claude", Some(&listed.id), Some(effort))
            .await
            .unwrap();
        check_agent(&reg, "claude", None, Some(effort))
            .await
            .unwrap();
        let m = refused(check_agent(&reg, "claude", Some(&listed.id), Some("turbo")).await);
        assert!(m.contains("no effort 'turbo'"), "{m}");
    }

    /// The pick reaches the launched CLI. The stand-in agent's "CLI" writes
    /// the arguments it was started with as its plan, so the plan the turn
    /// returns IS the spawned process's argv: the production's model, through
    /// the session's `meta.model` and the provider's model-flag template, and
    /// its effort, through `meta.effort` and the CLI family's effort flag (a
    /// pi-family stand-in: `--thinking`; no CLI home is touched for it).
    #[tokio::test]
    async fn the_planner_session_launches_with_the_picked_model_and_effort() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("state.db"))
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("u", "x", "U", false)
            .await
            .unwrap();
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .create("w", dir.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let reg = ProviderRegistry::new(Some(&json!({ "stand-in": {
            "cmd": "/bin/sh",
            "args": ["-c", "printf 'ready> '; printf 'argv:%s' \"$*\" > plan.json; exec cat", "stand-in"],
            "model_args": ["--model", "{model}"],
            "model_family": "pi"
        }})));
        let repo = otto_state::SessionsRepo::new(pool);
        let planner = AgentPlanner {
            sessions: Arc::new(SessionManager::new(repo, events, reg)),
            ws,
            user,
            provider: "stand-in".into(),
            model: Some("big-brain".into()),
            effort: Some("high".into()),
            root: planner_root(dir.path()),
        };
        let plan = planner
            .run("01M7VQ6QCRN9X8BX", "plan it".into())
            .await
            .unwrap();
        assert!(plan.contains("--model big-brain"), "{plan}");
        assert!(plan.contains("--thinking high"), "{plan}");
    }

    #[test]
    fn the_planner_folder_passes_the_session_cwd_guard() {
        let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir =
            otto_core::paths::confine_join(&planner_root(data.path()), "01M7VQ6QCRN9X8BX").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            otto_sandbox::unsafe_session_cwd(&dir, home.path(), data.path()),
            None
        );
        let old = data
            .path()
            .join("media")
            .join("productions")
            .join("01M7VQ6QCRN9X8BX");
        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(
            otto_sandbox::unsafe_session_cwd(&old, home.path(), data.path()),
            Some("inside Otto's data folder")
        );
    }
}
