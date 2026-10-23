//! The Design Hall turn (architecture-4-02): one assist or variant turn as an
//! [`AssistTarget`] of the shared assist job in `otto_agent_run::assist`,
//! which owns the claim check, the turn, the live preview poll and the turn
//! cap. This module keeps what is Design Hall's own: the done marker, the
//! citation / findings report, validation and the commit (main head, a
//! variant branch, or a side version when a human saved meanwhile), and the
//! working-copy restore after a failure. The handlers in `lib.rs` build a
//! [`TurnJob`] and spawn [`run_job`] under the artifact's claim.

use super::*;
use otto_agent_run::assist::{
    run_assist_job, AssistClaim, AssistOutcome, AssistState, AssistTarget, Stage,
};

/// Everything one spawned turn owns.
pub(crate) struct TurnJob<C> {
    pub(crate) ctx: C,
    pub(crate) ws: Workspace,
    pub(crate) user: User,
    pub(crate) artifact: DesignArtifact,
    pub(crate) turn_id: Id,
    pub(crate) mode: Mode,
    pub(crate) branch: Branch,
    pub(crate) direction: Option<String>,
    pub(crate) provider: String,
    pub(crate) model: Option<String>,
    pub(crate) existing_session: Option<Id>,
    pub(crate) adapter: Adapter,
    pub(crate) dir: PathBuf,
    pub(crate) agent_path: PathBuf,
    /// The artifact's working copy (main turns) — restored on failure.
    pub(crate) work_file: Option<PathBuf>,
    pub(crate) base_doc: Vec<u8>,
    pub(crate) base_source: String,
    pub(crate) base_version_id: Option<Id>,
    pub(crate) prompt: String,
    pub(crate) user_prompt: String,
    pub(crate) offered: Vec<OfferedRef>,
    pub(crate) rule_keys: Vec<String>,
    pub(crate) brand: Option<Value>,
    pub(crate) selection: Option<Value>,
    pub(crate) ready_tx: Option<oneshot::Sender<Id>>,
}

/// One [`TurnJob`] as the shared assist job sees it, plus what the run
/// derives (title, done marker) and the parse step's report.
struct DesignTurn<'a, C> {
    job: &'a TurnJob<C>,
    title: String,
    dir_str: String,
    done_path: PathBuf,
    done_line: String,
    ready_tx: Mutex<Option<oneshot::Sender<Id>>>,
    /// The citations / findings / summary [`AssistTarget::parse`] read; they
    /// stay on the turn when the parse or the commit then fails.
    partial: Mutex<Option<Outcome>>,
}

/// What the agent's turn produced.
enum Proposal {
    /// A critique (non-editing mode): report only; the head is restored.
    Review(Outcome),
    /// No new source: nothing to commit.
    Unchanged(Outcome),
    /// A valid document to commit.
    Commit { out: Outcome, doc: Vec<u8> },
}

fn done_marker() -> (PathBuf, String) {
    let path = std::env::temp_dir().join(format!("otto-design-done-{}.txt", new_id()));
    let _ = std::fs::write(&path, "");
    let line = format!(
        "\nLAST OF ALL — strictly after every other step above is complete — write your ONE-LINE \
         summary (with its [R…] citations) as plain text to this exact filesystem path: `{}`. \
         Writing this file ends your turn; never write it early.\n",
        path.display()
    );
    (path, line)
}

impl<C: DesignAssistCtx> AssistTarget for DesignTurn<'_, C> {
    type Proposal = Proposal;
    type Applied = Outcome;

    fn busy_key(&self) -> &str {
        &self.job.artifact.id
    }

    /// The handlers materialized the working copy and the context files
    /// before spawning (their errors answer the request), so nothing is left
    /// to put on disk here.
    async fn load(&self) -> Result<(), Error> {
        Ok(())
    }

    fn prompt(&self) -> String {
        format!("{}{}", self.job.prompt, self.done_line)
    }

    fn turn<'a>(&'a self, prompt: &'a str) -> AgentTurn<'a> {
        let job = self.job;
        let mut meta = json!({
            "source": "design_assist",
            "artifact_id": job.artifact.id,
            "turn_id": job.turn_id,
            "mode": job.mode.as_str(),
            "branch": job.branch.name(),
        });
        if let Some(m) = job
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            meta["model"] = json!(m);
        }
        AgentTurn {
            ws: &job.ws,
            user: &job.user,
            existing: job.existing_session.as_ref(),
            title: &self.title,
            cwd: &self.dir_str,
            provider: &job.provider,
            meta,
            prompt,
            stuck_after: STUCK_AFTER,
            done_file: Some(self.done_path.clone()),
            quiet_done: None,
        }
    }

    fn turn_cap(&self) -> Option<Duration> {
        Some(TURN_CAP)
    }

    /// Only a MAIN editing turn previews live.
    fn live_path(&self) -> Option<PathBuf> {
        (self.job.branch == Branch::Main && self.job.mode.edits())
            .then(|| self.job.agent_path.clone())
    }

    fn live_cap(&self) -> usize {
        dformat::MAX_CONTENT_BYTES
    }

    fn live_base(&self) -> Vec<u8> {
        self.job.base_source.clone().into_bytes()
    }

    /// Broadcast each valid change of the agent's file.
    fn on_live(&self, bytes: &[u8]) {
        let job = self.job;
        let Ok(src) = std::str::from_utf8(bytes) else {
            return;
        };
        let Ok(doc) = job.adapter.wrap(&job.base_doc, src) else {
            return;
        };
        // A half-written / invalid document is never pushed to viewers.
        if validate_doc(&job.adapter.format, &doc, |f, b| {
            job.ctx.validate_content(f, b)
        })
        .is_err()
        {
            return;
        }
        let content =
            dformat::spec(&job.adapter.format).and_then(|s| dformat::event_content(s, &doc));
        let _ = job.ctx.events().send(Event::DesignArtifactUpdated {
            workspace_id: job.artifact.workspace_id.clone(),
            artifact_id: job.artifact.id.clone(),
            format: job.adapter.format.clone(),
            change: "live".into(),
            version_id: None,
            content,
        });
    }

    fn report(&self, state: AssistState<'_>) {
        let AssistState::Running { session } = state else {
            return;
        };
        let job = self.job;
        if let Some(t) = update_turn(&job.artifact.id, &job.turn_id, |t| {
            t.session_id = Some(session.clone());
            t.status = "running".into();
        }) {
            emit_turn(job.ctx.events(), &t);
        }
        if let Some(tx) = self
            .ready_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = tx.send(session.clone());
        }
    }

    /// Keep the main assist session resumable (success or not).
    async fn remember(&self, session: &Id, _turn_ok: bool) {
        let job = self.job;
        if job.branch != Branch::Main {
            return;
        }
        let _ = job
            .ctx
            .design()
            .update_meta(
                &job.artifact.id,
                UpdateArtifactReq {
                    meta: Some(
                        json!({ "assist": { "session_id": session, "provider": job.provider } }),
                    ),
                    ..Default::default()
                },
                &Author::user(&job.user.id),
            )
            .await;
    }

    /// Read the agent's report and its document: the in-place edit, else a
    /// fenced block in the reply (an agent that printed instead of editing /
    /// the E2E stub). An unusable document is a parse failure.
    async fn parse(&self, reply: &str) -> Result<Proposal, Error> {
        let job = self.job;
        let findings = match read_capped(&job.dir.join("findings.json"), 64 * 1024).await {
            Some(b) => parse_findings(&String::from_utf8_lossy(&b)),
            None => vec![],
        };
        let prov_file: Option<Value> = read_capped(&job.dir.join("provenance.json"), 16 * 1024)
            .await
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(Value::is_object);
        let out = Outcome {
            status: "unchanged",
            cit: cite::verify(&job.offered, reply, prov_file.as_ref()),
            findings,
            message: summary_line(reply),
            ..Default::default()
        };
        *self.partial.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.clone());
        if !job.mode.edits() {
            return Ok(Proposal::Review(out));
        }
        let after = read_capped(&job.agent_path, dformat::MAX_CONTENT_BYTES).await;
        let src: Option<String> = match after {
            Some(b)
                if b != job.base_source.as_bytes() && !b.iter().all(u8::is_ascii_whitespace) =>
            {
                Some(
                    String::from_utf8(b)
                        .map_err(|_| Error::Invalid("the agent wrote a non-UTF-8 file".into()))?,
                )
            }
            _ => extract_fenced(reply, job.adapter.fence),
        };
        let Some(src) = src else {
            return Ok(Proposal::Unchanged(out)); // nothing changed
        };
        let doc = job
            .adapter
            .wrap(&job.base_doc, &src)
            .and_then(|d| {
                validate_doc(&job.adapter.format, &d, |f, b| {
                    job.ctx.validate_content(f, b)
                })
                .map(|_| d)
            })
            .map_err(|e| {
                Error::Invalid(format!(
                    "the agent produced an invalid {} document — nothing committed: {e}",
                    job.adapter.format
                ))
            })?;
        Ok(Proposal::Commit { out, doc })
    }

    /// Commit the document (main head, a variant branch, or — when a human
    /// saved meanwhile — a side version).
    async fn apply(&self, proposal: Proposal, sid: &Id) -> Result<Outcome, Error> {
        let job = self.job;
        let (mut out, doc) = match proposal {
            Proposal::Review(out) => {
                restore_current_head(job).await;
                return Ok(out);
            }
            Proposal::Unchanged(out) => return Ok(out),
            Proposal::Commit { out, doc } => (out, doc),
        };
        let svc = job.ctx.design();
        let message = out.message.clone();
        let author = Author {
            kind: "agent".into(),
            id: job.user.id.clone(),
            session_id: Some(sid.clone()),
        };
        let provenance = build_provenance(
            &job.turn_id,
            job.mode,
            &job.branch,
            job.direction.as_deref(),
            &job.provider,
            Some(sid.as_str()),
            &job.user_prompt,
            &job.offered,
            &out.cit,
            job.brand.as_ref(),
            &job.rule_keys,
            job.selection.as_ref(),
        );
        let commit_msg = message
            .clone()
            .unwrap_or_else(|| format!("Design assist ({})", job.mode.as_str()));

        let committed: Result<(DesignVersion, bool), Error> = match &job.branch {
            Branch::Variant { run_id, k } => svc
                .commit_variant(
                    &job.artifact,
                    doc,
                    run_id,
                    *k,
                    job.base_version_id.as_deref(),
                    author.clone(),
                    commit_msg,
                    provenance,
                )
                .await
                .map(|v| (v, true)),
            Branch::Main => {
                let fresh = match svc.store().require_artifact(&job.artifact.id).await {
                    Ok(a) => a,
                    Err(e) => return Err(e),
                };
                let res = svc
                    .commit_bytes(
                        &fresh,
                        doc.clone(),
                        SaveOpts {
                            base: Some(job.base_version_id.clone().unwrap_or_default()),
                            kind: "agent".into(),
                            author: author.clone(),
                            message: commit_msg.clone(),
                            provenance: provenance.clone(),
                            force: false,
                            validate: true,
                            change: "content",
                        },
                    )
                    .await;
                match res {
                    Ok(saved) => Ok((saved.version, saved.created)),
                    Err(Error::Conflict(_)) => {
                        // A human (or another writer) saved while the agent worked:
                        // never clobber — keep the draft as a side version they can
                        // compare and accept (`POST …/variants/{v}/accept`).
                        out.status = "conflict";
                        out.error = Some(
                            "the design changed while the agent worked — its draft was kept as a \
                             variant instead of replacing your edits"
                                .into(),
                        );
                        let variant = svc
                            .commit_variant(
                                &fresh,
                                doc,
                                &job.turn_id,
                                1,
                                job.base_version_id.as_deref(),
                                author.clone(),
                                commit_msg,
                                provenance,
                            )
                            .await
                            .map(|v| (v, true));
                        // The working copy keeps mirroring the HEAD (S4-21): the
                        // live poll left the agent's draft in the work/agent
                        // files while the head is the human's save — put the
                        // human's version back so the editor and the next turn
                        // start from it, not from the side-lined draft.
                        if variant.is_ok() {
                            restore_current_head(job).await;
                        }
                        variant
                    }
                    Err(e) => Err(e),
                }
            }
        };
        // A failed commit restores the head in `run_job` (Stage::Apply).
        let (version, created) = committed?;
        if !created {
            return Ok(out); // byte-identical to the head
        }
        if out.status != "conflict" {
            out.status = "done";
        }
        out.version_id = Some(version.id.clone());

        // Learning signal + provenance links (best-effort).
        let payload = json!({
            "turn_id": job.turn_id,
            "mode": job.mode.as_str(),
            "branch": version.branch,
            "direction": job.direction,
            "provider": job.provider,
            "cited": out.cit.cited.iter().map(|c| c.artifact_id.clone()).collect::<Vec<_>>(),
            "offered": job.offered.len(),
            "unverified": out.cit.unverified.len(),
        });
        let _ = svc
            .record_signal(
                &job.artifact,
                SignalReq {
                    artifact_id: job.artifact.id.clone(),
                    kind: "agent_draft".into(),
                    version_id: Some(version.id.clone()),
                    actor_kind: Some("agent".into()),
                    session_id: Some(sid.clone()),
                    payload: Some(payload),
                },
                &author,
            )
            .await;
        if version.branch == "main" {
            for c in &out.cit.cited {
                let _ = svc
                    .create_link(
                        &job.artifact,
                        CreateLinkReq {
                            rel: "references".into(),
                            dst_kind: "artifact".into(),
                            dst_id: c.artifact_id.clone(),
                            dst_node: None,
                            src_node: None,
                            policy: Some("pinned".into()),
                            pinned_version_id: c.version_id.clone(),
                            meta: Some(json!({
                                "via": "design_assist",
                                "label": c.label,
                                "cited_in_version": version.id,
                            })),
                        },
                        &author,
                    )
                    .await;
            }
        }
        Ok(out)
    }
}

/// Run one turn to completion under the artifact's `claim` and return its
/// final state (also stored in the registry and broadcast as
/// `design_assist_updated`).
pub(crate) async fn run_job<C: DesignAssistCtx>(
    mut job: TurnJob<C>,
    claim: &AssistClaim,
) -> DesignAssistTurn {
    let ctx = job.ctx.clone();
    let aid = job.artifact.id.clone();
    let tid = job.turn_id.clone();
    let ready_tx = job.ready_tx.take();
    let (done_path, done_line) = done_marker();
    let turn = DesignTurn {
        job: &job,
        title: match &job.branch {
            Branch::Main => format!("Design: {}", job.artifact.title),
            Branch::Variant { k, .. } => format!("Design variant {k}: {}", job.artifact.title),
        },
        dir_str: job.dir.to_string_lossy().to_string(),
        done_path,
        done_line,
        ready_tx: Mutex::new(ready_tx),
        partial: Mutex::new(None),
    };
    let ran = run_assist_job(ctx.sessions(), &turn, claim).await;
    let _ = std::fs::remove_file(&turn.done_path);
    let partial = turn
        .partial
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_default();
    let (outcome, sid) = match ran {
        AssistOutcome::Applied { applied, session } => (applied, Some(session)),
        AssistOutcome::Stopped { session } => {
            restore_current_head(&job).await;
            (Outcome::failed("the agent turn was stopped"), session)
        }
        AssistOutcome::Failed {
            stage,
            error,
            session,
        } => {
            restore_current_head(&job).await;
            let error = match (stage, error) {
                // Design Hall's own messages, without the error-kind prefix.
                (Stage::Parse | Stage::Deadline, Error::Invalid(m) | Error::Upstream(m)) => m,
                (_, e) => e.to_string(),
            };
            (
                Outcome {
                    status: "failed",
                    error: Some(error),
                    ..partial
                },
                session,
            )
        }
    };
    let final_turn = update_turn(&aid, &tid, |t| {
        t.status = outcome.status.to_string();
        t.session_id = sid.clone().or(t.session_id.clone());
        t.version_id = outcome.version_id.clone();
        t.cited = outcome.cit.cited.clone();
        t.unverified_citations = outcome.cit.unverified.clone();
        t.findings = outcome.findings.clone();
        t.message = outcome.message.clone();
        t.error = outcome.error.clone();
        t.finished_at = Some(Utc::now());
    });
    match final_turn {
        Some(t) => {
            emit_turn(ctx.events(), &t);
            t
        }
        None => {
            // Evicted from the history (very busy artifact) — still report.
            let t = DesignAssistTurn {
                turn_id: tid,
                artifact_id: aid,
                workspace_id: job.artifact.workspace_id.clone(),
                mode: job.mode.as_str().into(),
                status: outcome.status.to_string(),
                branch: job.branch.name(),
                direction: job.direction.clone(),
                provider: job.provider.clone(),
                session_id: sid,
                base_version_id: job.base_version_id.clone(),
                version_id: outcome.version_id,
                references: job.offered.clone(),
                cited: outcome.cit.cited,
                unverified_citations: outcome.cit.unverified,
                team_rules: job.rule_keys.clone(),
                findings: outcome.findings,
                message: outcome.message,
                error: outcome.error,
                started_at: Utc::now(),
                finished_at: Some(Utc::now()),
            };
            emit_turn(ctx.events(), &t);
            t
        }
    }
}

/// How a finished turn ended.
#[derive(Default, Clone)]
pub(crate) struct Outcome {
    status: &'static str,
    version_id: Option<Id>,
    cit: CitationReport,
    findings: Vec<Value>,
    message: Option<String>,
    error: Option<String>,
}

impl Outcome {
    fn failed(e: impl Into<String>) -> Self {
        Self {
            status: "failed",
            error: Some(e.into()),
            ..Default::default()
        }
    }
}

/// Restore the current published head after a failed, invalid, or critique
/// turn. A user's save during the turn must win over the captured base.
async fn restore_current_head<C: DesignAssistCtx>(job: &TurnJob<C>) {
    if job.branch != Branch::Main {
        return;
    }
    if let Err(error) = restore_document(
        &job.ctx.design(),
        &job.artifact,
        &job.adapter,
        job.work_file.as_deref(),
        &job.agent_path,
    )
    .await
    {
        tracing::warn!(artifact = %job.artifact.id, "restore design working head: {error}");
    }
}

pub(crate) async fn restore_document(
    svc: &DesignService,
    artifact: &DesignArtifact,
    adapter: &Adapter,
    work_file: Option<&FsPath>,
    agent_path: &FsPath,
) -> otto_core::Result<()> {
    let io = |e: std::io::Error| Error::Internal(format!("restore design working copy: {e}"));
    let _publication = svc.lock_working_copy(&artifact.id).await?;
    let fresh = svc.store().require_artifact(&artifact.id).await?;
    let (_, head) = svc.head_content(&fresh).await?;
    if let Some(wf) = work_file {
        tokio::fs::write(wf, &head).await.map_err(io)?;
    }
    if adapter.canvas_inner.is_some() {
        tokio::fs::write(agent_path, adapter.agent_source(&head))
            .await
            .map_err(io)?;
    }
    Ok(())
}
