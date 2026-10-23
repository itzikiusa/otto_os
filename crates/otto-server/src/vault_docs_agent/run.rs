//! The docs-run orchestration (background task; all errors isolated — never
//! panics): writers fan out through the shared runner ([`super::authors`]),
//! the summarizer consolidates, drafts are soft-trashed and `written` is
//! resolved; then the optional review loop ([`super::review_loop`]).
//! Spawned by `start_run`.

use super::*;

// ---------------------------------------------------------------------------
// Orchestration (background task; all errors isolated — never panics)
// ---------------------------------------------------------------------------

/// One writer's resolved spec (post provider-precedence).
#[derive(Clone)]
pub(super) struct WriterSpec {
    pub(super) provider: String,
    pub(super) model: Option<String>,
}

/// The whole run: writers fan out → (multi) summarizer consolidates → drafts
/// dir soft-trashed → `written` resolved (results file, else before/after diff).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_docs(
    ctx: ServerCtx,
    ws: Workspace,
    user: User,
    vault: otto_vault::VaultRec,
    run_id: String,
    prompt: String,
    target_dir: String,
    writers: Vec<WriterSpec>,
    summarizer: WriterSpec,
    skills: Vec<String>,
) {
    let reg = ctx.vault_docs_runs.clone();
    let (cancel, retries) = match reg.lock().unwrap().get(&run_id) {
        Some(e) => (Arc::clone(&e.cancel), Arc::clone(&e.retries)),
        None => return,
    };
    let review_config = reg
        .lock()
        .unwrap()
        .get(&run_id)
        .map(|entry| entry.run.review.clone())
        .unwrap_or_default();
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let m = writers.len();
    let run8: String = run_id.chars().take(8).collect();

    // The FINAL writer (single-writer or summarizer) reports what it wrote to
    // this pre-created temp file; parsed tolerantly, with a before/after diff
    // of note paths under target_dir as the fallback.
    let results_path = std::env::temp_dir().join(format!("otto-vaultdocs-{run_id}.json"));
    let _ = std::fs::write(&results_path, "");
    let results_str = results_path.to_string_lossy().to_string();

    // Pre-trust every distinct provider on the vault root (mirrors product_run)
    // so no session stalls on the interactive "trust this folder?" prompt.
    {
        let mut trusted = HashSet::<String>::new();
        for p in writers
            .iter()
            .map(|w| w.provider.clone())
            .chain(std::iter::once(summarizer.provider.clone()))
            .chain(
                review_config
                    .reviewers
                    .iter()
                    .map(|reviewer| reviewer.provider.clone()),
            )
        {
            if trusted.insert(p.clone()) {
                otto_sessions::trust::ensure_trusted(&p, &vault.root_path);
            }
        }
    }

    // Baseline note-path snapshot under target_dir (the `written` fallback).
    let _ = ctx.vault.scan(vault.id).await;
    let before: HashSet<String> = note_paths(&ctx, vault.id).await;

    // Skill vehicles, resolved once: request-selected skills (prepared prompts
    // pass e.g. `vault-repo-docs`) + `okf-authoring` on OKF vaults. The complete
    // packages are staged per run: Claude loads the native view via extra_dirs;
    // every other provider reads the provider-neutral view by explicit path.
    let mut author_skill_names = skills;
    if vault.okf && !author_skill_names.iter().any(|s| s == "okf-authoring") {
        author_skill_names.push("okf-authoring".into());
    }
    let mut staged_skill_names = author_skill_names.clone();
    for reviewer in &review_config.reviewers {
        if !staged_skill_names.contains(&reviewer.skill) {
            staged_skill_names.push(reviewer.skill.clone());
        }
    }
    let skill_bundle_dir = otto_context::materialize::default_context_root()
        .join("vault-docs-skills")
        .join(&run_id);
    let skill_bundle = (!staged_skill_names.is_empty())
        .then(|| {
            crate::modules::stage_skill_packages_at(
                &ctx.context_library,
                &staged_skill_names,
                &skill_bundle_dir,
            )
        })
        .flatten();
    let fallback_text = skill_fallbacks(&ctx.context_library, &author_skill_names);

    // ---- Stage 1: writers, concurrently (per-writer errors isolated) --------
    // The shared fan-out runner drives each writer's turn loop: normally one
    // pass; a user retry kills the session and flags the slot, and the runner
    // re-spawns a FRESH session with the same prompt (new done-marker each
    // attempt; the old one is embedded in the old prompt). A failed writer
    // keeps listening for a retry while any PEER is still moving.
    let writer_ctl = writer_control(&reg, &run_id, &cancel, &retries);
    let mut slots = Vec::with_capacity(m);
    for (i, w) in writers.iter().cloned().enumerate() {
        let skill_guidance = skill_package_guidance(
            &w.provider,
            skill_bundle.as_ref(),
            &author_skill_names,
            &fallback_text,
        );
        let prompt = build_writer_prompt(
            &prompt,
            vault.id,
            i + 1,
            m,
            &run8,
            &target_dir,
            vault.okf,
            skill_guidance.as_deref(),
            (m == 1).then_some(results_str.as_str()),
        );
        let turn = AuthorTurn {
            ctx: ctx.clone(),
            ws: ws.clone(),
            user: user.clone(),
            reg: reg.clone(),
            run_id: run_id.clone(),
            root: vault.root_path.clone(),
            title: format!("Vault docs: writer {}/{}", i + 1, m),
            provider: w.provider.clone(),
            meta: author_meta(vault.id, &run_id, &w, skill_bundle.as_ref()),
            prompt,
            cancel: Arc::clone(&cancel),
            results: (m == 1).then(|| results_path.clone()),
            role: AuthorRole::Writer(i),
        };
        slots.push(move || turn.clone().run());
    }
    let done = otto_agent_run::fanout::run_slots(slots, &writer_ctl).await;
    let any_ok = done.as_ref().is_some_and(|d| !d.is_empty());
    // A retry that lands in the instant between an errored waiter giving up
    // and the barrier closing flips its slot "pending" with nobody left to
    // serve it — surface that honestly instead of a forever-pending writer.
    with_run(&reg, &run_id, |r| {
        for a in r.agents.iter_mut() {
            if a.state == "pending" {
                a.state = "error".into();
                a.error =
                    Some("retry arrived after the writers stage closed — start a new run".into());
            }
        }
    });

    if cancel.load(Ordering::Relaxed) {
        finish_run(&reg, &run_id, "cancelled", None);
        finalize_run(&reg, &repo, &run_id).await;
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    // ---- Stage 2 (single writer): finals are already in target_dir ----------
    if m == 1 {
        let _ = ctx.vault.scan(vault.id).await;
        let after = note_paths(&ctx, vault.id).await;
        let written = resolve_written(&results_path, &before, &after, &target_dir);
        with_run(&reg, &run_id, |r| r.written = written);
        if any_ok {
            let author_sid = reg
                .lock()
                .unwrap()
                .get(&run_id)
                .and_then(|entry| entry.run.agents.first()?.session_id.clone());
            complete_review_or_finish(
                &ctx,
                &ws,
                &user,
                &vault,
                &reg,
                &repo,
                &run_id,
                &prompt,
                &target_dir,
                &writers[0],
                author_sid,
                skill_bundle.clone(),
                Arc::clone(&cancel),
                Arc::clone(&retries),
            )
            .await;
        } else {
            finish_run(&reg, &run_id, "error", Some("writer agent failed".into()));
            finalize_run(&reg, &repo, &run_id).await;
        }
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    // ---- Stage 2 (multi): collect drafts, then the summarizer ---------------
    let _ = ctx.vault.scan(vault.id).await;
    let all_after_writers = note_paths(&ctx, vault.id).await;
    let mut drafts: Vec<(String, String)> = Vec::new(); // (path, raw content)
    for i in 0..m {
        let prefix = format!("{}/", draft_dir(&run8, i + 1));
        let mut agent_drafts: Vec<String> = all_after_writers
            .iter()
            .filter(|p| p.starts_with(&prefix))
            .cloned()
            .collect();
        agent_drafts.sort();
        for p in &agent_drafts {
            let raw = match ctx.vault.note(&vault.ws_id, vault.id, p).await {
                Ok(n) => n.raw,
                Err(_) => String::new(),
            };
            drafts.push((p.clone(), raw));
        }
        with_run(&reg, &run_id, |r| {
            if let Some(a) = r.agents.get_mut(i) {
                a.drafts = agent_drafts;
            }
        });
    }

    if !any_ok {
        // Nothing to consolidate — every writer errored.
        with_run(&reg, &run_id, |r| r.summarizer.state = "skipped".into());
        finish_run(
            &reg,
            &run_id,
            "error",
            Some("all writer agents failed".into()),
        );
        finalize_run(&reg, &repo, &run_id).await;
        let _ = std::fs::remove_file(&results_path);
        return;
    }
    if cancel.load(Ordering::Relaxed) {
        finish_run(&reg, &run_id, "cancelled", None);
        finalize_run(&reg, &repo, &run_id).await;
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    let sum_guidance = skill_package_guidance(
        &summarizer.provider,
        skill_bundle.as_ref(),
        &author_skill_names,
        &fallback_text,
    );
    let sum_prompt = build_summarizer_prompt(
        &prompt,
        vault.id,
        m,
        &target_dir,
        &drafts,
        vault.okf,
        sum_guidance.as_deref(),
        &results_str,
    );
    let sum_turn = AuthorTurn {
        ctx: ctx.clone(),
        ws: ws.clone(),
        user: user.clone(),
        reg: reg.clone(),
        run_id: run_id.clone(),
        root: vault.root_path.clone(),
        title: "Vault docs: summarizer".into(),
        provider: summarizer.provider.clone(),
        meta: author_meta(vault.id, &run_id, &summarizer, skill_bundle.as_ref()),
        prompt: sum_prompt,
        cancel: Arc::clone(&cancel),
        results: Some(results_path.clone()),
        role: AuthorRole::Summarizer,
    };
    // Same turn loop as the writers (a user retry re-spawns a fresh
    // summarizer), without waiting: a failed summarizer has no deterministic
    // stand-in here, so its "fallback" is to keep the drafts in place.
    let mut sum_ctl = writer_ctl.clone();
    sum_ctl.max_auto_retries = 0;
    sum_ctl.wait = otto_agent_run::fanout::WaitForRetry::Never;
    let (summary, _cancelled) = otto_agent_run::fanout::summarize_with_retries(
        done.as_deref().unwrap_or_default(),
        &sum_ctl,
        |_: &[(usize, ())]| sum_turn.clone().run(),
        |_: &[(usize, ())]| (),
    )
    .await;
    let sum_err = summary.error;
    persist(&reg, &run_id);

    // ---- Stage 3: soft-trash the drafts dir + resolve `written` -------------
    // std::fs::rename is atomic within a filesystem; a cross-device rename (or
    // a missing drafts dir, e.g. the E2E stub) fails and is deliberately
    // ignored — a leftover `_drafts/` is harmless and user-visible.
    // A FAILED summarizer keeps the drafts in place: they are the only copy of
    // the writers' work at that point, and the user can consolidate manually
    // or re-run from them.
    if sum_err.is_none() && !cancel.load(Ordering::Relaxed) {
        let src = std::path::Path::new(&vault.root_path)
            .join("_drafts")
            .join(format!("docs-run-{run8}"));
        let trash = std::path::Path::new(&vault.root_path).join(".trash");
        let _ = std::fs::create_dir_all(&trash);
        let _ = std::fs::rename(&src, trash.join(format!("docs-run-{run8}")));
    }
    let _ = ctx.vault.scan(vault.id).await;

    let after = note_paths(&ctx, vault.id).await;
    let written = resolve_written(&results_path, &before, &after, &target_dir);
    with_run(&reg, &run_id, |r| r.written = written);
    match &sum_err {
        None => {
            let author_sid = reg
                .lock()
                .unwrap()
                .get(&run_id)
                .and_then(|entry| entry.run.summarizer.session_id.clone());
            complete_review_or_finish(
                &ctx,
                &ws,
                &user,
                &vault,
                &reg,
                &repo,
                &run_id,
                &prompt,
                &target_dir,
                &summarizer,
                author_sid,
                skill_bundle,
                Arc::clone(&cancel),
                Arc::clone(&retries),
            )
            .await;
        }
        Some(e) => finish_run(
            &reg,
            &run_id,
            "error",
            Some(format!("summarizer failed: {e}")),
        ),
    }
    if sum_err.is_some() {
        finalize_run(&reg, &repo, &run_id).await;
    }
    let _ = std::fs::remove_file(&results_path);
}
