//! The refine flow: ONE resumed session per (vault, note) applies user
//! instructions to an existing note across turns. Routed from
//! [`super::routes`].

use super::*;

/// The note's most recent persisted refine session, IF that session still
/// exists (retention may have pruned it — then a fresh first turn is correct).
pub(super) async fn rehydrate_refine_session(
    ctx: &ServerCtx,
    vault_id: i64,
    path: &str,
    workspace: &str,
    creator: &str,
) -> Option<Id> {
    let row = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone())
        .latest_refine_for_note_scoped(vault_id, path, workspace, creator)
        .await
        .ok()
        .flatten()?;
    let run: VaultDocsRun = serde_json::from_str(&row.payload).ok()?;
    let sid = Id::from(run.agents.first()?.session_id.clone()?);
    let session = ctx.manager.get(&sid).await.ok()?;
    (session.workspace_id == workspace && session.created_by == creator).then_some(sid)
}

/// `POST /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine` — one resumed
/// session per (workspace, creator, vault, note); returns when the turn completes (LONG request).
/// The turn itself runs in a DETACHED task: a client disconnect (page reload)
/// drops this handler's future, but the turn still completes and finalizes
/// both the refine registry and the recorded turn row.
pub(super) async fn refine(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RefineReq>,
) -> ApiResult<Json<RefineResp>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Editor)
        .await?;
    let vault = ctx
        .vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    let ws = ctx
        .workspaces
        .get(&Id::from(ws_id.clone()))
        .await
        .map_err(ApiError)?;
    let note = ctx
        .vault
        .note(&ws_id, vault_id, &req.path)
        .await
        .map_err(ApiError)?;
    let key = (
        ws_id.clone(),
        user.id.to_string(),
        vault_id,
        req.path.clone(),
    );

    // Resume the note's session when one exists AND the requested provider
    // matches it. The registry is memory-only — on a true miss (fresh daemon,
    // note never bound), rehydrate from the newest persisted refine turn so
    // "one session per note" survives restarts; an explicit reset writes a
    // tombstone entry (present, no session) that must NOT rehydrate.
    let (mut existing, bound) = {
        let reg = ctx.vault_docs_refine.lock().unwrap();
        match reg.get(&key) {
            Some(e) => (e.session_id.clone(), true),
            None => (None, false),
        }
    };
    if existing.is_none() && !bound {
        existing = rehydrate_refine_session(&ctx, vault_id, &req.path, &ws_id, &user.id).await;
        if let Some(sid) = &existing {
            let mut reg = ctx.vault_docs_refine.lock().unwrap();
            let entry = reg.entry(key.clone()).or_insert_with(|| RefineEntry {
                session_id: Some(sid.clone()),
                ..Default::default()
            });
            existing = entry.session_id.clone();
        }
    }
    let mut provider = ctx
        .resolve_provider(Some(&ws), req.provider.as_deref())
        .await
        .map_err(ApiError)?;
    if let Some(sid) = existing.clone() {
        match ctx.manager.get(&sid).await {
            // An explicitly different provider means "give me a fresh agent on
            // this note" (the drawer's picker is live, not locked) — drop the
            // old binding and spawn new below. Same/unspecified resumes, and
            // the session's REAL provider drives transcript polling.
            Ok(s) => {
                let switched = s.workspace_id != ws_id
                    || s.created_by != user.id
                    || req
                        .provider
                        .as_deref()
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .is_some_and(|p| p != s.provider);
                if switched {
                    existing = None;
                } else {
                    provider = s.provider;
                }
            }
            // Session record gone (deleted) — the binding is stale; start over.
            Err(_) => existing = None,
        }
    }
    let first_turn = existing.is_none();
    otto_sessions::trust::ensure_trusted(&provider, &vault.root_path);

    let refine_skill_names = if vault.okf {
        vec!["okf-authoring".to_string()]
    } else {
        Vec::new()
    };
    let refine_bundle_dir = otto_context::materialize::default_context_root()
        .join("vault-docs-skills")
        .join(format!("refine-{}", otto_core::new_id()));
    let refine_bundle = (!refine_skill_names.is_empty())
        .then(|| {
            crate::modules::stage_skill_packages_at(
                &ctx.context_library,
                &refine_skill_names,
                &refine_bundle_dir,
            )
        })
        .flatten();
    let refine_fallback = skill_fallbacks(&ctx.context_library, &refine_skill_names);
    let skill_guidance = skill_package_guidance(
        &provider,
        refine_bundle.as_ref(),
        &refine_skill_names,
        &refine_fallback,
    );
    let prompt = build_refine_prompt(
        &req.prompt,
        vault_id,
        &req.path,
        &note.meta.hash,
        first_turn.then_some(note.raw.as_str()),
        vault.okf,
        skill_guidance.as_deref(),
    );
    let (done_path, done_line) = done_marker();
    let prompt = format!("{prompt}{done_line}");
    let mut meta = serde_json::json!({
        "source": "vault-docs",
        "vault_id": vault_id,
    });
    if let Some(m) = &req.model {
        meta["model"] = Value::String(m.clone());
    }
    if let Some(dirs) = otto_review::session::review_skills_extra_dirs(
        &provider,
        staged_package_root(refine_bundle.as_ref()),
    ) {
        meta["extra_dirs"] = dirs;
    }

    // Mark the turn in flight + surface the session id the MOMENT it exists,
    // so `GET refine-session` can attach the live shell while the turn runs.
    let turn_id = otto_core::new_id().to_string();
    {
        let mut reg = ctx.vault_docs_refine.lock().unwrap();
        let e = reg.entry(key.clone()).or_default();
        if e.running {
            return Err(ApiError(Error::Conflict(
                "a refine turn is already running for this note".into(),
            )));
        }
        e.running = true;
        e.turn_id = Some(turn_id.clone());
    }

    // Each refine turn is recorded as a `kind:"refine"` run (one agent entry)
    // so edits share the docs runs history. Not run-registry-held — the turn
    // is a single await; the durable row is the whole story. Shared behind a
    // mutex: on_ready, the finalizer, and any in-flight persist snapshot the
    // SAME object, so no persist can regress the row.
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let turn_run = VaultDocsRun {
        id: turn_id.clone(),
        ws_id: ws_id.clone(),
        vault_id,
        kind: "refine".into(),
        prompt: req.prompt.clone(),
        target_dir: String::new(),
        note_path: req.path.clone(),
        state: "running".into(),
        agents: vec![VaultDocsAgent {
            index: 0,
            name: format!("refine \u{00b7} {provider}"),
            provider: provider.clone(),
            model: req.model.clone().filter(|m| !m.trim().is_empty()),
            state: "running".into(),
            session_id: existing.as_ref().map(|s| s.to_string()),
            error: None,
            drafts: Vec::new(),
        }],
        summarizer: VaultDocsSummarizer {
            provider: String::new(),
            model: None,
            state: "skipped".into(),
            session_id: None,
            error: None,
        },
        review: VaultDocsReview::default(),
        written: Vec::new(),
        error: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
    };
    if let Err(e) = repo.upsert(&run_row(&turn_run)).await {
        warn!("vault_docs: persist refine turn {turn_id}: {e}");
    }
    let turn_run = Arc::new(Mutex::new(turn_run));
    // The turn's own write-behind persister (same single-writer discipline as
    // docs runs; see `spawn_persister`). Exits when the last sender drops.
    let persist_tx = {
        let snap = Arc::clone(&turn_run);
        spawn_persister(repo.clone(), move || Some(snap.lock().unwrap().clone()))
    };

    // This turn's session id, for the finalizer: a reset / provider switch can
    // rebind the note to a DIFFERENT session while this turn still runs — the
    // finalizer must only touch the registry entry if it still points at us.
    let turn_sid: Arc<Mutex<Option<Id>>> = Arc::new(Mutex::new(existing.clone()));

    let ready_reg = ctx.vault_docs_refine.clone();
    let ready_key = key.clone();
    let ready_tx = persist_tx.clone();
    let ready_turn = Arc::clone(&turn_run);
    let ready_sid = Arc::clone(&turn_sid);
    let ready_turn_id = turn_id.clone();
    let on_ready = move |sid: &Id| {
        *ready_sid.lock().unwrap() = Some(sid.clone());
        update_refine_entry(&ready_reg, &ready_key, &ready_turn_id, Some(sid), false);
        // Durable ASAP (a restart mid-turn must still know the session).
        if let Some(a) = ready_turn.lock().unwrap().agents.get_mut(0) {
            a.session_id = Some(sid.to_string());
        }
        let _ = ready_tx.send(None);
    };

    let task = {
        let ctx = ctx.clone();
        let refine_reg = ctx.vault_docs_refine.clone();
        let key = key.clone();
        let repo = repo.clone();
        let turn_run = Arc::clone(&turn_run);
        let persist_tx = persist_tx.clone();
        let path = req.path.clone();
        let title = format!("Vault refine: {}", req.path);
        let root = vault.root_path.clone();
        let provider = provider.clone();
        tokio::spawn(async move {
            let turn = crate::agent_session::run_session_turn_with(
                &ctx,
                &ws,
                &user,
                existing.as_ref(),
                &title,
                &root,
                &provider,
                meta,
                &prompt,
                crate::agent_session::STUCK_IDLE,
                crate::agent_session::TurnOpts {
                    done_file: Some(done_path.clone()),
                    done_file_validator: Some(valid_done_marker),
                    kill_on_stall: true,
                    ..Default::default()
                },
                on_ready,
            )
            .await;
            let _ = std::fs::remove_file(&done_path);

            // The turn is over either way — clear `running` + finish the
            // recorded turn (this task survives a client disconnect, so the
            // row can never be stranded `running` until the next restart).
            // Guarded: a reset / provider switch may have rebound this note to
            // a different session mid-turn — leave that newer binding alone.
            let ours = turn_sid.lock().unwrap().clone();
            update_refine_entry(&refine_reg, &key, &turn_id, ours.as_ref(), true);
            let final_row = {
                let mut run = turn_run.lock().unwrap();
                run.finished_at = Some(chrono::Utc::now().to_rfc3339());
                match &turn {
                    Ok((_reply, sid)) => {
                        run.state = "done".into();
                        run.written = vec![path.clone()];
                        if let Some(a) = run.agents.get_mut(0) {
                            a.state = "done".into();
                            a.session_id = Some(sid.to_string());
                        }
                    }
                    Err(e) => {
                        run.state = "error".into();
                        run.error = Some(e.0.to_string());
                        if let Some(a) = run.agents.get_mut(0) {
                            a.state = "error".into();
                            a.error = Some(e.0.to_string());
                        }
                    }
                }
                run_row(&run)
            };
            // Flush any queued on_ready write, then land the terminal row
            // directly — the persister is idle after the ack, so this write
            // has no concurrent twin.
            flush_persister(&persist_tx).await;
            if let Err(e) = repo.upsert(&final_row).await {
                warn!("vault_docs: persist refine turn {}: {}", final_row.id, e);
            }
            turn
        })
    };

    let (reply, sid) = task
        .await
        .map_err(|e| ApiError(Error::Internal(format!("refine turn panicked: {e}"))))??;
    Ok(Json(RefineResp {
        session_id: sid.to_string(),
        reply,
    }))
}

/// `GET /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine-session?path=` —
/// the caller's (workspace, vault, note) session + whether a turn is currently in flight.
pub(super) async fn refine_session(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<RefineSessionQ>,
) -> ApiResult<Json<RefineSessionResp>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Viewer)
        .await?;
    // Files are global, but conversational state belongs to a workspace/user.
    ctx.vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    let (mut entry, bound) = {
        let reg = ctx.vault_docs_refine.lock().unwrap();
        match reg.get(&(ws_id.clone(), user.id.to_string(), vault_id, q.path.clone())) {
            Some(e) => (e.clone(), true),
            None => (RefineEntry::default(), false),
        }
    };
    // TRUE registry miss (fresh daemon): rehydrate the note's persisted session
    // so the drawer re-attaches the SAME conversation after a restart. A bound
    // entry with no session is a reset tombstone — never resurrect those.
    if !bound && entry.session_id.is_none() && !entry.running {
        if let Some(sid) = rehydrate_refine_session(&ctx, vault_id, &q.path, &ws_id, &user.id).await
        {
            let mut reg = ctx.vault_docs_refine.lock().unwrap();
            entry = reg
                .entry((ws_id.clone(), user.id.to_string(), vault_id, q.path.clone()))
                .or_insert_with(|| RefineEntry {
                    session_id: Some(sid),
                    ..Default::default()
                })
                .clone();
        }
    }
    Ok(Json(RefineSessionResp {
        session_id: entry.session_id.map(|s| s.to_string()),
        running: entry.running,
    }))
}

/// `DELETE …/docs-agents/refine-session?path=` — detach the note's refine
/// session so the next Send starts a FRESH agent (any provider). The old
/// session itself is left alone (still inspectable in the sessions list). The
/// tombstone entry (present, no session) blocks rehydration from persisted
/// turns — without it the very next GET/POST would resurrect the old binding.
pub(super) async fn reset_refine_session(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<RefineSessionQ>,
) -> ApiResult<Json<RefineSessionResp>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Editor)
        .await?;
    ctx.vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    ctx.vault_docs_refine.lock().unwrap().insert(
        (ws_id, user.id.to_string(), vault_id, q.path),
        RefineEntry::default(),
    );
    Ok(Json(RefineSessionResp {
        session_id: None,
        running: false,
    }))
}
