//! The improvement engine: run a self-reflection pass for one workspace, then
//! apply / approve / reject / rollback individual edits through the version log.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use otto_core::domain::{
    Autonomy, ImprovementEdit, ImprovementEditKind, ImprovementEditStatus, ImprovementRunStatus,
    ImprovementTarget, ImprovementTrigger,
};
use otto_core::event::Event;
use otto_core::{Error, Id, Result};
use otto_state::convert::fmt as fmt_ts;
use otto_state::{ImprovementsRepo, NewEdit, SessionsRepo, WorkspacesRepo};
use tokio::sync::broadcast;

use crate::classify::{decide, Disposition};
use crate::config::{effective_config, next_run, write_config};
use crate::digest::{build_digest, SessionDigest};
use crate::pathsafe::resolve_target;
use crate::producer::ProposalProducer;
use crate::prompt::{build_prompt, load_skill_instructions};
use crate::proposal::{ImprovementProposal, ProposedEdit};

/// The providers to run analysis on, defaulting to `["claude"]` when unset.
fn effective_providers(configured: &[String]) -> Vec<String> {
    let cleaned: Vec<String> = configured
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if cleaned.is_empty() {
        vec!["claude".to_string()]
    } else {
        cleaned
    }
}

/// Prefix an edit's rationale with the provider that suggested it, so merged
/// multi-provider suggestions stay attributable in the UI.
fn label_provider(provider: &str, rationale: &str) -> String {
    if rationale.trim().is_empty() {
        format!("[via {provider}]")
    } else {
        format!("[via {provider}] {rationale}")
    }
}

/// Max runs/edits we read back for the UI list endpoints.
pub const RUN_LIST_LIMIT: i64 = 50;

pub struct ImprovementEngine {
    pub improvements: ImprovementsRepo,
    pub sessions: SessionsRepo,
    pub workspaces: WorkspacesRepo,
    pub producer: Arc<dyn ProposalProducer>,
    pub events: broadcast::Sender<Event>,
    /// Otto context-library root (`<data_dir>/library`). Skill edits target the
    /// library entry when present (the library is the source of truth).
    pub library_root: PathBuf,
}

impl ImprovementEngine {
    /// Run a self-reflection pass. `Manual` runs regardless of `enabled`;
    /// scheduled callers should pre-check `config::is_due`.
    /// Create the run row, then execute it to completion. Used by the
    /// scheduler (awaited fully). `Manual` runs ignore `enabled`.
    pub async fn run_for_workspace(
        &self,
        ws_id: &Id,
        trigger: ImprovementTrigger,
    ) -> Result<Id> {
        let run = self.improvements.create_run(ws_id, trigger).await?;
        let id = run.id;
        self.execute_run(&id, ws_id, trigger).await?;
        Ok(id)
    }

    /// Execute an already-created run row. The HTTP "run now" handler creates
    /// the row (so it can return the id immediately) and spawns this in the
    /// background, mirroring the PR-review run pattern in `modules.rs`.
    pub async fn execute_run(
        &self,
        run_id: &Id,
        ws_id: &Id,
        _trigger: ImprovementTrigger,
    ) -> Result<()> {
        let _ = self.events.send(Event::ImprovementRunStarted {
            workspace_id: ws_id.clone(),
            run_id: run_id.clone(),
        });
        let ws = self.workspaces.get(ws_id).await?;
        let mut cfg = effective_config(&ws.settings);

        // Gather recent sessions.
        let since = fmt_ts(Utc::now() - ChronoDuration::hours(cfg.lookback_hours.max(1) as i64));
        let sessions = self.sessions.list_active_since(ws_id, &since).await?;

        // Always advance the schedule, even on skip/fail, so a broken run
        // doesn't busy-loop the scheduler.
        let advance_schedule = |engine_cfg: &mut otto_core::api::SelfImprovementConfig| {
            let now = Utc::now();
            engine_cfg.last_run_at = Some(now);
            engine_cfg.next_run_at = Some(next_run(engine_cfg, now));
        };

        // Build digests; skip cheaply if nothing to review.
        let digests: Vec<SessionDigest> = sessions.iter().filter_map(build_digest).collect();
        if digests.is_empty() {
            advance_schedule(&mut cfg);
            self.persist_config(ws_id, &ws.settings, &cfg).await?;
            self.improvements
                .finish_run(run_id, ImprovementRunStatus::Skipped, "no sessions in window", 0, 0, 0, None)
                .await?;
            self.emit_finished(ws_id, run_id, "skipped", 0, 0);
            return Ok(());
        }

        // Detect candidate skills (used in-window ∩ allow-list) and read files.
        let used: Vec<String> = digests
            .iter()
            .flat_map(|d| d.skills_used.clone())
            .collect();
        let current_skills = self.read_candidate_skills(&ws.root_path, &used, &cfg.skill_allowlist);
        let current_memory = self.read_memory(&ws.root_path);

        let skill_instructions = load_skill_instructions(&ws.root_path);
        let prompt = build_prompt(
            &skill_instructions,
            &ws.name,
            &digests,
            &current_skills,
            &current_memory,
            &cfg.skill_allowlist,
        );

        // Run the analysis on every configured provider; each contributes its
        // own suggestions (labeled by provider). One provider failing never
        // aborts the others — we merge whatever succeeds.
        let providers = effective_providers(&cfg.providers);
        let mut edits: Vec<ProposedEdit> = Vec::new();
        let mut summaries: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        for provider in &providers {
            match self.producer.produce(&prompt, &ws.root_path, provider).await {
                Ok(mut p) => {
                    for e in &mut p.edits {
                        e.rationale = label_provider(provider, &e.rationale);
                    }
                    if !p.run_summary.trim().is_empty() {
                        summaries.push(format!("[{provider}] {}", p.run_summary.trim()));
                    }
                    edits.extend(p.edits);
                }
                Err(e) => {
                    tracing::warn!(provider, "self-improvement: provider produced no proposal: {e}");
                    errors.push(format!("{provider}: {e}"));
                }
            }
        }

        // Every provider failed → the run failed.
        if edits.is_empty() && summaries.is_empty() && !errors.is_empty() {
            advance_schedule(&mut cfg);
            self.persist_config(ws_id, &ws.settings, &cfg).await?;
            let msg = errors.join("; ");
            self.improvements
                .finish_run(run_id, ImprovementRunStatus::Failed, "", digests.len() as i64, 0, 0, Some(&msg))
                .await?;
            self.emit_finished(ws_id, run_id, "failed", 0, 0);
            return Ok(());
        }

        let proposal = ImprovementProposal {
            run_summary: summaries.join("\n"),
            edits,
        };

        let (applied, pending) = self
            .process_edits(ws_id, &ws.root_path, run_id, &proposal, &cfg.skill_allowlist, cfg.autonomy)
            .await;

        advance_schedule(&mut cfg);
        self.persist_config(ws_id, &ws.settings, &cfg).await?;
        // Surface any provider that was skipped (failed) in the run summary.
        let mut summary = proposal.run_summary.clone();
        if !errors.is_empty() {
            summary.push_str(&format!("\n\n(skipped: {})", errors.join("; ")));
        }
        self.improvements
            .finish_run(
                run_id,
                ImprovementRunStatus::Done,
                &summary,
                digests.len() as i64,
                applied,
                pending,
                None,
            )
            .await?;
        self.emit_finished(ws_id, run_id, "done", applied, pending);
        Ok(())
    }

    /// Apply or queue every edit in `proposal`, emitting events. Returns
    /// `(applied, pending)`. Shared by `execute_run` and `evolve_session`.
    async fn process_edits(
        &self,
        ws_id: &Id,
        root: &str,
        run_id: &Id,
        proposal: &ImprovementProposal,
        allowlist: &[String],
        autonomy: Autonomy,
    ) -> (i64, i64) {
        let mut applied = 0i64;
        let mut pending = 0i64;
        for edit in &proposal.edits {
            match self
                .process_edit(root, ws_id, run_id, edit, allowlist, autonomy)
                .await
            {
                Ok(ImprovementEditStatus::Applied) => {
                    applied += 1;
                    if let Some(last) = self
                        .improvements
                        .list_edits_by_run(run_id)
                        .await
                        .ok()
                        .and_then(|v| v.into_iter().last())
                    {
                        let _ = self.events.send(Event::ImprovementEditApplied {
                            workspace_id: ws_id.clone(),
                            run_id: run_id.clone(),
                            edit_id: last.id,
                            target_ref: edit.target_ref.clone(),
                        });
                    }
                }
                Ok(ImprovementEditStatus::Pending) => {
                    pending += 1;
                    if let Some(last) = self
                        .improvements
                        .list_edits_by_run(run_id)
                        .await
                        .ok()
                        .and_then(|v| v.into_iter().last())
                    {
                        let _ = self.events.send(Event::ImprovementApprovalPending {
                            workspace_id: ws_id.clone(),
                            run_id: run_id.clone(),
                            edit_id: last.id,
                            target_ref: edit.target_ref.clone(),
                        });
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(edit = %edit.target_ref, "self-improvement: dropping edit: {e}")
                }
            }
        }
        (applied, pending)
    }

    /// Live in-loop evolution: analyze ONE just-finished interaction and improve
    /// the skill(s) it used. Reuses the same gate/version-log; does NOT touch the
    /// cron schedule. Returns the run id.
    pub async fn evolve_session(&self, session_id: &Id) -> Result<Id> {
        let session = self.sessions.get(session_id).await?;
        let ws = self.workspaces.get(&session.workspace_id).await?;
        let cfg = effective_config(&ws.settings);

        let run = self.improvements.create_run(&ws.id, ImprovementTrigger::Live).await?;
        let run_id = run.id.clone();
        let _ = self.events.send(Event::ImprovementRunStarted {
            workspace_id: ws.id.clone(),
            run_id: run_id.clone(),
        });

        let Some(digest) = build_digest(&session) else {
            self.improvements
                .finish_run(&run_id, ImprovementRunStatus::Skipped, "no transcript yet", 0, 0, 0, None)
                .await?;
            self.emit_finished(&ws.id, &run_id, "skipped", 0, 0);
            return Ok(run_id);
        };

        let used = digest.skills_used.clone();
        let current_skills = self.read_candidate_skills(&ws.root_path, &used, &cfg.skill_allowlist);
        let current_memory = self.read_memory(&ws.root_path);
        let skill_instructions = load_skill_instructions(&ws.root_path);
        let mut prompt = build_prompt(
            &skill_instructions,
            &ws.name,
            std::slice::from_ref(&digest),
            &current_skills,
            &current_memory,
            &cfg.skill_allowlist,
        );
        prompt.push_str(
            "\n\nNOTE: This is a LIVE single-interaction review. Focus narrowly on improving \
             the skill(s) THIS one session used, based on how it went. Be conservative — only \
             propose a change you have clear evidence for from this interaction.\n",
        );
        // Live evolve fires after every interaction, so it runs on a single
        // provider (the first configured, default claude) rather than fanning
        // out to all of them on every turn.
        let provider = effective_providers(&cfg.providers)
            .into_iter()
            .next()
            .unwrap_or_else(|| "claude".to_string());

        match self.producer.produce(&prompt, &ws.root_path, &provider).await {
            Ok(proposal) => {
                let (applied, pending) = self
                    .process_edits(&ws.id, &ws.root_path, &run_id, &proposal, &cfg.skill_allowlist, cfg.autonomy)
                    .await;
                self.improvements
                    .finish_run(&run_id, ImprovementRunStatus::Done, &proposal.run_summary, 1, applied, pending, None)
                    .await?;
                self.emit_finished(&ws.id, &run_id, "done", applied, pending);
            }
            Err(e) => {
                let msg = e.to_string();
                self.improvements
                    .finish_run(&run_id, ImprovementRunStatus::Failed, "", 1, 0, 0, Some(&msg))
                    .await?;
                self.emit_finished(&ws.id, &run_id, "failed", 0, 0);
            }
        }
        Ok(run_id)
    }

    /// Resolve, classify, and apply-or-queue a single proposed edit. Returns
    /// the resulting status (Applied or Pending). Path-unsafe edits error out
    /// (caller logs + drops them).
    async fn process_edit(
        &self,
        root: &str,
        ws_id: &Id,
        run_id: &Id,
        edit: &ProposedEdit,
        allowlist: &[String],
        autonomy: otto_core::domain::Autonomy,
    ) -> Result<ImprovementEditStatus> {
        let path = resolve_target(
            root,
            edit.target_type,
            &edit.target_ref,
            Some(self.library_root.as_path()),
        )?;
        let path_str = path.to_string_lossy().to_string();
        let current = tokio::fs::read_to_string(&path).await.ok();
        let disposition = decide(edit, allowlist, autonomy);

        let status = match disposition {
            Disposition::Apply => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        Error::Internal(format!("create dir {}: {e}", parent.display()))
                    })?;
                }
                if edit.kind == ImprovementEditKind::Remove {
                    let _ = tokio::fs::remove_file(&path).await;
                } else {
                    tokio::fs::write(&path, &edit.patch.after)
                        .await
                        .map_err(|e| Error::Internal(format!("write {path_str}: {e}")))?;
                }
                ImprovementEditStatus::Applied
            }
            Disposition::Queue => ImprovementEditStatus::Pending,
        };

        self.improvements
            .create_edit(NewEdit {
                run_id: run_id.clone(),
                workspace_id: ws_id.clone(),
                target: edit.target_type,
                target_ref: edit.target_ref.clone(),
                target_path: path_str,
                kind: edit.kind,
                risk: edit.risk,
                status,
                rationale: edit.rationale.clone(),
                evidence: edit.evidence.clone(),
                before_content: current,
                after_content: edit.patch.after.clone(),
                actor: if status == ImprovementEditStatus::Applied {
                    Some("system".to_string())
                } else {
                    None
                },
            })
            .await?;
        Ok(status)
    }

    // ---- approval-queue actions ----

    /// Approve a pending edit → apply it (with a conflict check).
    pub async fn approve_edit(&self, edit_id: &Id, actor: &str) -> Result<ImprovementEdit> {
        let edit = self.improvements.get_edit(edit_id).await?;
        if edit.status != ImprovementEditStatus::Pending {
            return Err(Error::Invalid("edit is not pending".into()));
        }
        // Conflict: the file changed since we snapshotted `before_content`.
        let current = tokio::fs::read_to_string(&edit.target_path).await.ok();
        if current != edit.before_content {
            return Ok(self
                .improvements
                .set_edit_status(edit_id, ImprovementEditStatus::Conflict, Some(actor))
                .await?);
        }
        if let Some(parent) = std::path::Path::new(&edit.target_path).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if edit.kind == ImprovementEditKind::Remove {
            let _ = tokio::fs::remove_file(&edit.target_path).await;
        } else {
            tokio::fs::write(&edit.target_path, &edit.after_content)
                .await
                .map_err(|e| Error::Internal(format!("write {}: {e}", edit.target_path)))?;
        }
        self.improvements
            .set_edit_status(edit_id, ImprovementEditStatus::Applied, Some(actor))
            .await
    }

    /// Reject a pending edit (no file change).
    pub async fn reject_edit(&self, edit_id: &Id, actor: &str) -> Result<ImprovementEdit> {
        let edit = self.improvements.get_edit(edit_id).await?;
        if edit.status != ImprovementEditStatus::Pending {
            return Err(Error::Invalid("edit is not pending".into()));
        }
        self.improvements
            .set_edit_status(edit_id, ImprovementEditStatus::Rejected, Some(actor))
            .await
    }

    /// Roll back an applied edit, restoring `before_content` (deletes the file
    /// if it was newly created). Conflict-checks against `after_content`.
    pub async fn rollback_edit(&self, edit_id: &Id, actor: &str) -> Result<ImprovementEdit> {
        let edit = self.improvements.get_edit(edit_id).await?;
        if edit.status != ImprovementEditStatus::Applied {
            return Err(Error::Invalid("only applied edits can be rolled back".into()));
        }
        let current = tokio::fs::read_to_string(&edit.target_path).await.ok();
        // If the file changed since we wrote it, don't clobber — flag conflict.
        if edit.kind != ImprovementEditKind::Remove && current.as_deref() != Some(edit.after_content.as_str()) {
            return Ok(self
                .improvements
                .set_edit_status(edit_id, ImprovementEditStatus::Conflict, Some(actor))
                .await?);
        }
        match &edit.before_content {
            Some(before) => {
                tokio::fs::write(&edit.target_path, before)
                    .await
                    .map_err(|e| Error::Internal(format!("write {}: {e}", edit.target_path)))?;
            }
            None => {
                // File was created by the edit → rollback deletes it.
                let _ = tokio::fs::remove_file(&edit.target_path).await;
            }
        }
        self.improvements
            .set_edit_status(edit_id, ImprovementEditStatus::RolledBack, Some(actor))
            .await
    }

    // ---- helpers ----

    async fn persist_config(
        &self,
        ws_id: &Id,
        current_settings: &serde_json::Value,
        cfg: &otto_core::api::SelfImprovementConfig,
    ) -> Result<()> {
        let merged = write_config(current_settings, cfg);
        self.workspaces
            .update(ws_id, None, None, Some(&merged), None)
            .await?;
        Ok(())
    }

    fn emit_finished(&self, ws_id: &Id, run_id: &Id, status: &str, applied: i64, pending: i64) {
        let _ = self.events.send(Event::ImprovementRunFinished {
            workspace_id: ws_id.clone(),
            run_id: run_id.clone(),
            status: status.to_string(),
            applied,
            pending,
        });
    }

    /// Read allow-listed skills that were actually used in-window. (Allow-list
    /// scoping keeps the prompt focused and bounds blast radius.)
    fn read_candidate_skills(
        &self,
        root: &str,
        used: &[String],
        allowlist: &[String],
    ) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for name in allowlist {
            if !used.iter().any(|u| u == name) {
                continue; // not exercised this window — nothing new to learn
            }
            if let Ok(path) =
                resolve_target(root, ImprovementTarget::Skill, name, Some(self.library_root.as_path()))
            {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    out.push((name.clone(), content));
                }
            }
        }
        out
    }

    /// Read `MEMORY.md` + sibling `*.md` files from the workspace's project
    /// memory dir. Bounded to keep the prompt small.
    fn read_memory(&self, root: &str) -> Vec<(String, String)> {
        let dir = otto_orchestrator::claude_pty::project_dir(root).join("memory");
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let (Some(name), Ok(content)) =
                        (path.file_name().and_then(|n| n.to_str()), std::fs::read_to_string(&path))
                    {
                        out.push((name.to_string(), content));
                    }
                }
                if out.len() >= 25 {
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::{EditPatch, ImprovementProposal, ProposedEdit};
    use otto_core::domain::{ImprovementEditKind, ImprovementRisk, ImprovementTarget};

    struct FakeProducer(ImprovementProposal);
    impl ProposalProducer for FakeProducer {
        fn produce<'a>(
            &'a self,
            _prompt: &'a str,
            _cwd: &'a str,
            _provider: &'a str,
        ) -> otto_core::auth::BoxFuture<'a, Result<ImprovementProposal>> {
            Box::pin(async move { Ok(self.0.clone()) })
        }
    }

    // Build an engine over a temp SQLite pool + temp workspace dir.
    async fn harness() -> (ImprovementEngine, String, Id, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let pool = otto_state::open(&db).await.unwrap();
        // Minimal: create a workspace whose root is the temp dir.
        let workspaces = WorkspacesRepo::new(pool.clone());
        // A user row must exist for create()'s member insert FK (and for the
        // session's created_by FK), so create one and reuse its id.
        let users = otto_state::UsersRepo::new(pool.clone());
        let uid = users.create("root", "pw", "root", true).await.unwrap().id;
        let ws = workspaces.create("t", dir.path().to_str().unwrap(), &uid).await.unwrap();
        let (events, _) = broadcast::channel(16);
        let proposal = ImprovementProposal {
            run_summary: "s".into(),
            edits: vec![ProposedEdit {
                id: "e1".into(),
                target_type: ImprovementTarget::Memory,
                target_ref: "MEMORY.md".into(),
                kind: ImprovementEditKind::Add,
                risk: ImprovementRisk::Low,
                rationale: "note".into(),
                evidence: vec!["sess".into()],
                dedup_checked: true,
                dedup_quote: None,
                patch: EditPatch { before: None, after: "# notes\n- learned X\n".into() },
            }],
        };
        let engine = ImprovementEngine {
            improvements: ImprovementsRepo::new(pool.clone()),
            sessions: SessionsRepo::new(pool.clone()),
            workspaces,
            producer: Arc::new(FakeProducer(proposal)),
            events,
            library_root: dir.path().join("library"),
        };
        (engine, ws.id, uid, dir)
    }

    #[tokio::test]
    async fn memory_low_edit_applies_and_rolls_back() {
        let (engine, ws_id, uid, dir) = harness().await;

        // No sessions yet → manual run skips.
        let run_id = engine
            .run_for_workspace(&ws_id, ImprovementTrigger::Manual)
            .await
            .unwrap();
        let run = engine.improvements.get_run(&run_id).await.unwrap();
        assert_eq!(run.status, ImprovementRunStatus::Skipped);

        // Seed a session WITH a transcript so the next run has something to do.
        let psid = "11111111-1111-4111-8111-111111111111";
        let proj = otto_orchestrator::claude_pty::project_dir(dir.path().to_str().unwrap());
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join(format!("{psid}.jsonl")),
            r#"{"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#,
        )
        .unwrap();
        engine
            .sessions
            .create(otto_state::NewSession {
                workspace_id: ws_id.clone(),
                kind: otto_core::domain::SessionKind::Agent,
                provider: "claude".into(),
                title: "t".into(),
                cwd: dir.path().to_str().unwrap().to_string(),
                provider_session_id: Some(psid.to_string()),
                connection_id: None,
                created_by: uid.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();

        // Run again → memory edit auto-applies (tiered + memory).
        let run_id = engine
            .run_for_workspace(&ws_id, ImprovementTrigger::Manual)
            .await
            .unwrap();
        let run = engine.improvements.get_run(&run_id).await.unwrap();
        assert_eq!(run.status, ImprovementRunStatus::Done);
        assert_eq!(run.applied, 1);

        let mem_path = proj.join("memory").join("MEMORY.md");
        assert_eq!(std::fs::read_to_string(&mem_path).unwrap(), "# notes\n- learned X\n");

        // Roll it back → file deleted (before_content was None).
        let edits = engine.improvements.list_edits_by_run(&run_id).await.unwrap();
        let applied = edits.iter().find(|e| e.status == ImprovementEditStatus::Applied).unwrap();
        engine.rollback_edit(&applied.id, "u").await.unwrap();
        assert!(!mem_path.exists());
        let after = engine.improvements.get_edit(&applied.id).await.unwrap();
        assert_eq!(after.status, ImprovementEditStatus::RolledBack);
    }
}
