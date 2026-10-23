//! Pure helpers + prompt builders (unit-tested; no DB / no agent): retry
//! budgets, the review request rules and round policy, done markers, target
//! dirs and the writer / summarizer / refine / reviewer / revision prompts.

use super::*;

// ---------------------------------------------------------------------------
// Pure helpers + prompt builders (unit-tested; no DB / no agent)
// ---------------------------------------------------------------------------

/// Execution rules appended to every docs-agent prompt (writer, summarizer,
/// refine). Mirrors the workflow engine's WF_STEP_RULES: run completion is
/// detected when the TURN ends, so an agent that fans work out to sub-agents
/// gets marked done while its background workers are still writing — exactly
/// the "DONE but 7 agents still running" failure observed live.
pub(super) const DOCS_STEP_RULES: &str = "\nEXECUTION RULES:\n\
    - Do ALL of the work yourself in THIS turn. Do NOT spawn, launch, or \
    delegate to sub-agents, background agents, parallel workers, or the Task \
    tool. No run_in_background, no fan-out — you are the only agent.\n\
    - Do NOT end your turn until the work is fully complete. Never stop early \
    to \"wait for\" something you started.\n";

/// Cap on user-requested retries per writer/summarizer slot within one run —
/// a sanity bound, not a policy (each retry is an explicit click).
pub(super) const MAX_USER_RETRIES: u32 = 5;

/// One automatic respawn per writer slot when its CLI process dies underneath
/// the turn (codex has been seen exiting seconds after its trust prompt). A
/// single attempt heals the flake without looping on a persistent failure.
pub(super) const MAX_AUTO_RETRIES: u32 = 1;

/// Bundled, read-only review methods accepted by the public run request.
pub const VAULT_REVIEWER_SKILLS: [&str; 5] = [
    "vault-docs-review",
    "vault-api-review",
    "vault-data-review",
    "vault-runtime-review",
    "vault-evidence-review",
];

/// Categories shared by every bundled reviewer finding contract.
pub const VAULT_REVIEW_CATEGORIES: [&str; 6] =
    ["coverage", "api", "data", "runtime", "evidence", "quality"];

/// Validate the optional review block before any sessions or durable run rows
/// are created. Provider resolution follows the same defaults as writers, so
/// an empty provider is intentionally valid here.
pub fn validate_review_request(review: &ReviewReq) -> Result<(), String> {
    if review.reviewers.is_empty() || review.reviewers.len() > 4 {
        return Err("review.reviewers must be 1..=4 when review is enabled".into());
    }
    if !(1..=10).contains(&review.max_iterations) {
        return Err("review.max_iterations must be 1..=10".into());
    }
    for (index, reviewer) in review.reviewers.iter().enumerate() {
        let skill = reviewer.skill.trim();
        if !VAULT_REVIEWER_SKILLS.contains(&skill) {
            return Err(format!(
                "review.reviewers[{index}] has unknown reviewer skill '{}'; expected one of: {}",
                reviewer.skill,
                VAULT_REVIEWER_SKILLS.join(", ")
            ));
        }
    }
    Ok(())
}

/// Parse the reviewer's result artifact. Only a bare JSON array satisfying the
/// findings contract is accepted: prose, wrapper objects, incomplete evidence,
/// and unsupported severities are errors rather than accidental clean verdicts.
pub fn parse_review_findings(content: &str) -> Result<Vec<VaultDocsFinding>, String> {
    let findings: Vec<VaultDocsFinding> = serde_json::from_str(content.trim())
        .map_err(|error| format!("review output must be a valid JSON finding array: {error}"))?;
    for (index, finding) in findings.iter().enumerate() {
        if !matches!(finding.severity.as_str(), "blocking" | "major" | "minor") {
            return Err(format!(
                "review finding {index} severity must be blocking, major, or minor"
            ));
        }
        if !VAULT_REVIEW_CATEGORIES.contains(&finding.category.as_str()) {
            return Err(format!(
                "review finding {index} category must be one of: {}",
                VAULT_REVIEW_CATEGORIES.join(", ")
            ));
        }
        for (field, value) in [
            ("summary", finding.summary.as_str()),
            ("missed_item", finding.missed_item.as_str()),
            ("required_fix", finding.required_fix.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("review finding {index} requires non-empty {field}"));
            }
        }
        if finding.evidence.is_empty() {
            return Err(format!(
                "review finding {index} requires at least one evidence location"
            ));
        }
        for (evidence_index, evidence) in finding.evidence.iter().enumerate() {
            let repo_complete = evidence
                .repo_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
                && evidence.line.is_some_and(|line| line > 0);
            let doc_complete = evidence
                .doc_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
                && evidence
                    .section
                    .as_deref()
                    .is_some_and(|section| !section.trim().is_empty());
            if !repo_complete && !doc_complete {
                return Err(format!(
                    "review finding {index} evidence {evidence_index} requires repo_path+line or doc_path+section"
                ));
            }
        }
    }
    Ok(findings)
}

/// A round is clean only after every configured reviewer has completed in the
/// same round and every one returned an empty findings array.
pub fn all_reviewers_clean(reviewers: &[VaultDocsReviewer]) -> bool {
    !reviewers.is_empty()
        && reviewers
            .iter()
            .all(|reviewer| reviewer.state == "done" && reviewer.findings.is_empty())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewAction {
    Pending,
    Clean,
    Revise,
    Exhausted,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReviewRoundOutcome {
    Clean,
    Revise,
    Exhausted,
    Error,
}

/// Clone only immutable reviewer configuration into a new independently
/// visible round. Runtime/result fields never leak from an earlier round.
pub(super) fn fresh_review_round(
    iteration: u8,
    configured: &[VaultDocsReviewer],
) -> VaultDocsReviewRound {
    VaultDocsReviewRound {
        iteration,
        state: "reviewing".into(),
        reviewers: configured
            .iter()
            .map(|reviewer| VaultDocsReviewer {
                index: reviewer.index,
                provider: reviewer.provider.clone(),
                model: reviewer.model.clone(),
                skill: reviewer.skill.clone(),
                focus: reviewer.focus.clone(),
                state: "pending".into(),
                session_id: None,
                findings: Vec::new(),
                error: None,
            })
            .collect(),
        revision: VaultDocsRevision::default(),
    }
}

pub(super) fn review_round_outcome(
    reviewers: &[VaultDocsReviewer],
    iteration: u8,
    max_iterations: u8,
) -> ReviewRoundOutcome {
    match next_review_action(reviewers, iteration, max_iterations) {
        ReviewAction::Clean => ReviewRoundOutcome::Clean,
        ReviewAction::Revise => ReviewRoundOutcome::Revise,
        ReviewAction::Exhausted => ReviewRoundOutcome::Exhausted,
        ReviewAction::Pending | ReviewAction::Error => ReviewRoundOutcome::Error,
    }
}

/// Decide the next stage without advancing an incomplete or failed round.
pub fn next_review_action(
    reviewers: &[VaultDocsReviewer],
    current_iteration: u8,
    max_iterations: u8,
) -> ReviewAction {
    if reviewers.is_empty()
        || reviewers
            .iter()
            .any(|reviewer| !matches!(reviewer.state.as_str(), "pending" | "running" | "done"))
    {
        ReviewAction::Error
    } else if reviewers
        .iter()
        .any(|reviewer| matches!(reviewer.state.as_str(), "pending" | "running"))
    {
        ReviewAction::Pending
    } else if all_reviewers_clean(reviewers) {
        ReviewAction::Clean
    } else if current_iteration >= max_iterations {
        ReviewAction::Exhausted
    } else {
        ReviewAction::Revise
    }
}

/// A completion marker is written last, after all output artifacts are complete.
pub(super) fn valid_done_marker(text: &str) -> bool {
    serde_json::from_str::<Value>(text).ok().is_some_and(|v| {
        v.get("status").and_then(Value::as_str) == Some("done")
            && v.get("summary")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
    })
}

pub(super) fn done_marker() -> (std::path::PathBuf, String) {
    let path =
        std::env::temp_dir().join(format!("otto-vaultdocs-done-{}.json", otto_core::new_id()));
    let line = format!(
        "\nLAST OF ALL — strictly after every other step above is complete — write exactly one JSON object \
         {{\"status\":\"done\",\"summary\":\"one-line summary\"}} to `{}`. \
         Writing this file ends your turn; never write it early.\n", path.display());
    (path, line)
}

pub(super) fn require_author_results(path: &std::path::Path) -> Result<(), ApiError> {
    let valid = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|v| {
            v.as_array()
                .or_else(|| v.get("written").and_then(Value::as_array))
                .is_some_and(|paths| paths.iter().all(Value::is_string))
        });
    if valid {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid(
            "author did not write valid {\"written\": [...]} results".into(),
        )))
    }
}

/// The vault-relative drafts folder for writer `n` (1-based) of a run.
pub(super) fn draft_dir(run8: &str, n: usize) -> String {
    format!("_drafts/docs-run-{run8}/agent-{n}")
}

/// Normalize + validate a vault-relative target dir: strip surrounding
/// slashes/whitespace, reject traversal, backslashes, and hidden segments.
/// `""` = the vault root.
pub(super) fn normalize_target_dir(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim().trim_matches('/').to_string();
    if t.is_empty() {
        return Ok(t);
    }
    if t.contains('\\')
        || t.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.'))
    {
        return Err(ApiError(Error::Invalid(format!(
            "invalid target_dir: {raw}"
        ))));
    }
    Ok(t)
}

/// Human phrasing of the target dir for prompts.
pub(super) fn dir_display(target_dir: &str) -> String {
    if target_dir.is_empty() {
        "the vault root (no folder prefix)".to_string()
    } else {
        format!("`{target_dir}/`")
    }
}

/// Take the first `cap` CHARS of `s` (never splits a char). Returns the
/// (possibly shortened) text + whether it was truncated.
pub(super) fn cap_chars(s: &str, cap: usize) -> (String, bool) {
    let mut it = s.char_indices();
    match it.nth(cap) {
        Some((byte, _)) => (s[..byte].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// The shared OKF requirements block (present iff the vault is OKF). For
/// non-claude providers `inline` carries the full okf-authoring SKILL.md text;
/// claude instead has the skill staged first-class via extra_dirs.
pub(super) fn okf_block(okf: bool, inline: Option<&str>) -> String {
    let mut b = String::new();
    if okf {
        b.push_str(
            "\nOKF — this vault is OKF-conformant and EVERY note you write MUST comply:\n\
             - YAML frontmatter on every note with at least `type:` (concept/service/endpoint/\
             dataset/decision/runbook/metric/…) and a one-sentence `description:`.\n\
             - Internal references use standard markdown links (relative, `.md`).\n\
             - Keep each folder's `index.md` listing its children up to date.\n",
        );
    }
    // `inline` carries EVERY skill selected for this run (okf-authoring plus
    // any prepared-prompt skills like vault-repo-docs) for providers that
    // can't load a staged bundle; claude gets the bundle via extra_dirs and
    // only the invoke hint below.
    match inline {
        Some(text) => {
            b.push_str("\n--- AUTHORING SKILLS (follow these methods exactly) ---\n");
            b.push_str(text);
            b.push_str("\n--- END AUTHORING SKILLS ---\n");
        }
        None if okf => {
            b.push_str(
                "You have the `okf-authoring` skill available — invoke it before writing.\n",
            );
        }
        None => {}
    }
    b
}

/// Build one writer's prompt. Multi-writer (`m > 1`): drafts ONLY under its own
/// `_drafts/docs-run-<run8>/agent-<n>/` folder. Single writer: finals straight
/// into `target_dir` + the results JSON file (`results_path` is `Some`).
#[allow(clippy::too_many_arguments)]
pub(super) fn build_writer_prompt(
    user_prompt: &str,
    vault_id: i64,
    n: usize,
    m: usize,
    run8: &str,
    target_dir: &str,
    okf: bool,
    okf_inline: Option<&str>,
    results_path: Option<&str>,
) -> String {
    let mut p = String::from("OTTO_TASK: vault_docs_write\n");
    if m > 1 {
        let dir = draft_dir(run8, n);
        p.push_str(&format!(
            "You are WRITER AGENT {n} of {m} documenting the request below into Otto vault \
             {vault_id}. Peers cover the same request independently; a summarizer consolidates \
             all drafts afterwards.\n\n\
             RULES — follow exactly:\n\
             - Write DRAFT notes ONLY under `{dir}/` (vault-relative). NO edits anywhere else \
             in the vault.\n\
             - Write every note via the otto MCP tool `otto_vault_write` with vault_id \
             {vault_id} and a path like `{dir}/<topic>.md` (parent folders are auto-created).\n\
             - Cover the request thoroughly; split into multiple focused notes where that \
             helps, cross-linked with standard markdown links.\n\
             - Read existing notes first (`otto_vault_list` / `otto_vault_read`) when the \
             request builds on them.\n"
        ));
    } else {
        p.push_str(&format!(
            "You are the WRITER AGENT producing FINAL documentation for the request below \
             into Otto vault {vault_id}.\n\n\
             RULES — follow exactly:\n\
             - Write the final notes directly under {dd} via the otto MCP tool \
             `otto_vault_write` with vault_id {vault_id} (parent folders are auto-created).\n\
             - Cover the request thoroughly; split into multiple focused notes where that \
             helps, cross-linked with standard markdown links.\n\
             - Read existing notes first (`otto_vault_list` / `otto_vault_read`) when the \
             request builds on them.\n",
            dd = dir_display(target_dir),
        ));
        if okf {
            p.push_str(&format!(
                "- Update (or create) {dd}'s `index.md` to reference the notes you wrote.\n",
                dd = dir_display(target_dir),
            ));
        }
    }
    // Quality floor baked into the PROMPT (not only the staged skill): weaker
    // CLIs skim skill packages — real runs came back with meta-placeholders in
    // example position and ~1/7th of the flow inventory covered.
    p.push_str(
        "- EXAMPLES ARE REAL: every request/response/payload example must be a concrete \
         body with the actual field names from the code (DTOs, handlers, tests, fixtures) \
         and plausible values. NEVER a meta-description in example position — \
         `{\"note\": \"DTO fields per handler\"}` is a FAILED example.\n\
         - COMPLETENESS: when the request demands an inventory (flows, routes, tables, \
         topics), enumerate the FULL inventory from the code FIRST, then cover EVERY \
         item — no sampling, no representative subset. Before finishing, re-count: \
         items documented must equal items enumerated; if they differ, keep writing.\n",
    );
    p.push_str(DOCS_STEP_RULES);
    p.push_str(&okf_block(okf, okf_inline));
    if let Some(rp) = results_path {
        p.push_str(&format!(
            "\nFINALLY, write a results file to this exact filesystem path: `{rp}` \
             containing ONLY this JSON (every vault-relative note path you wrote): \
             {{\"written\": [\"path/to/note.md\", ...]}}\n"
        ));
    }
    p.push_str("\nWhen done, reply with a ONE-LINE summary of what you wrote.\n");
    p.push_str(&format!("\nRequest:\n{user_prompt}\n"));
    p
}

/// Build the summarizer prompt: every draft inlined (path + content) under a
/// total cap; drafts past the cap are listed by path only, and the cap is
/// stated so the agent knows to `otto_vault_read` the rest.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_summarizer_prompt(
    user_prompt: &str,
    vault_id: i64,
    m: usize,
    target_dir: &str,
    drafts: &[(String, String)],
    okf: bool,
    okf_inline: Option<&str>,
    results_path: &str,
) -> String {
    let dd = dir_display(target_dir);
    let mut p = format!(
        "OTTO_TASK: vault_docs_summarize\n\
         You are the SUMMARIZER. {m} writer agents drafted documentation for the SAME request \
         in Otto vault {vault_id}; their drafts are inlined below (total inlined content is \
         capped at {DRAFT_INLINE_CAP} chars — a draft marked [truncated] or [not inlined] has \
         its full content on disk, read it with `otto_vault_read`).\n\n\
         CONSOLIDATE — follow exactly:\n\
         - Produce ONE coherent, deduplicated set of FINAL notes under {dd} via \
         `otto_vault_write` (vault_id {vault_id}).\n\
         - Prefer the best content per topic; merge overlaps; fix contradictions. Do NOT \
         copy drafts in verbatim as duplicates.\n\
         - Do NOT write anything under `_drafts/` — finals only.\n\
         - A draft marked [truncated] or [not inlined] MUST be read in full via \
         `otto_vault_read` before you write the final note it feeds — never consolidate \
         from the truncation.\n\
         - COVERAGE IS THE UNION: every distinct flow/route/table/topic ANY writer \
         documented must survive into the finals as its own note/section. Do NOT collapse \
         distinct flows into one generic \"route family\" note.\n\
         - EXAMPLES ARE REAL: keep the most concrete request/response/payload examples \
         from the drafts in every final note. NEVER replace them with meta-placeholders — \
         `{{\"note\": \"DTO fields per handler\"}}` is a FAILED example. If no draft has a \
         real example, lift one from the repository code; omit the example section \
         entirely rather than writing a stub.\n"
    );
    if okf {
        p.push_str(&format!(
            "- OKF vault: give every final note conformant frontmatter, refresh {dd}'s \
             `index.md`, then run `otto_vault_okf_validate` (vault_id {vault_id}) and fix \
             EVERY error it reports.\n",
        ));
    }
    p.push_str(DOCS_STEP_RULES);
    p.push_str(&okf_block(okf, okf_inline));
    p.push_str(&format!(
        "\nFINALLY, write a results file to this exact filesystem path: `{results_path}` \
         containing ONLY this JSON (every vault-relative note path you wrote): \
         {{\"written\": [\"path/to/note.md\", ...]}}\n\
         Then reply with a ONE-LINE summary.\n"
    ));
    p.push_str(&format!(
        "\nRequest the writers documented:\n{user_prompt}\n"
    ));

    p.push_str("\nDRAFTS:\n");
    let mut budget = DRAFT_INLINE_CAP;
    for (path, content) in drafts {
        p.push_str(&format!("\n### {path}\n"));
        if budget == 0 {
            p.push_str("[not inlined — read via otto_vault_read]\n");
            continue;
        }
        let (chunk, truncated) = cap_chars(content, budget);
        budget -= chunk.chars().count();
        p.push_str(&chunk);
        if truncated {
            p.push_str("\n[truncated]\n");
        } else {
            p.push('\n');
        }
    }
    p
}

/// Build a refine-turn prompt. First turn (`content` = `Some`): full contract
/// with the current note inlined (capped). Later turns: just the instruction +
/// the CURRENT hash (the resumed session already holds the context).
#[allow(clippy::too_many_arguments)]
pub(super) fn build_refine_prompt(
    user_prompt: &str,
    vault_id: i64,
    path: &str,
    hash: &str,
    content: Option<&str>,
    okf: bool,
    okf_inline: Option<&str>,
) -> String {
    let Some(raw) = content else {
        return format!(
            "OTTO_TASK: vault_docs_refine\n\
             Apply this instruction to the SAME note `{path}` in Otto vault {vault_id}. Its \
             current content hash is now `{hash}` — pass it as if_hash when you write \
             (re-read + re-apply on a conflict).\n\n\
             Instruction:\n{user_prompt}\n\n\
             Reply with ONE line describing the change.\n"
        );
    };
    let (inlined, truncated) = cap_chars(raw, REFINE_INLINE_CAP);
    let mut p = format!(
        "OTTO_TASK: vault_docs_refine\n\
         You are refining the note `{path}` in Otto vault {vault_id}. Its current content \
         hash is `{hash}`; the current content is inlined below (capped at \
         {REFINE_INLINE_CAP} chars{trunc} — the full note is on disk via `otto_vault_read`).\n\n\
         RULES — follow exactly:\n\
         - Apply the instruction by writing the FULL revised note via `otto_vault_write` \
         (vault_id {vault_id}, path `{path}`, if_hash `{hash}`).\n\
         - On a hash conflict, re-read with `otto_vault_read` and re-apply.\n",
        trunc = if truncated { "; TRUNCATED here" } else { "" },
    );
    if okf {
        p.push_str(
            "- OKF vault: keep the note conformant. AUGMENT, don't rewrite — existing \
             headings survive unless the instruction says otherwise.\n",
        );
    }
    p.push_str(DOCS_STEP_RULES);
    p.push_str(&okf_block(okf, okf_inline));
    p.push_str(&format!(
        "\nCURRENT CONTENT:\n{inlined}\n\nInstruction:\n{user_prompt}\n\n\
         Reply with ONE line describing the change.\n"
    ));
    p
}

/// Build an independent review turn. The package is referenced by its staged
/// path and the only allowed mutation is the server-owned JSON result file.
#[allow(clippy::too_many_arguments)]
pub fn build_reviewer_prompt(
    original_request: &str,
    vault_id: i64,
    target_dir: &str,
    iteration: u8,
    skill: &str,
    focus: Option<&str>,
    package_path: &str,
    results_path: &str,
) -> String {
    let focus = focus
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("\nADDITIONAL FOCUS (narrows, never waives the method):\n{value}\n"))
        .unwrap_or_default();
    format!(
        "OTTO_TASK: vault_docs_review\n\
         You are an INDEPENDENT REVIEWER in review round {iteration}. Review the current FINAL \
         documentation under {target} in Otto vault {vault_id} against the original request and \
         the real repository in your working directory.\n\n\
         METHOD — open and follow the complete `{skill}` package starting at:\n\
         `{package_path}`\n\
         Read its referenced checklists and examples before judging the bundle.{focus}\n\
         REVIEW RULES — follow exactly:\n\
         - This is read-only with respect to the vault: never call a vault write, rename, or \
         delete tool and never repair findings yourself.\n\
         - Reconcile the current `coverage.md`, final notes, indexes, and deterministic audit \
         output against real code. Do not blindly repeat audit candidates.\n\
         - Every finding must be proven by a real repository path and line and/or an exact \
         documentation path and section. Reject speculative findings and stylistic preferences.\n\
         - Re-read the current bundle in this round; do not repeat a finding already repaired.\n\
         - `[]` is clean only after completing every mandatory check in the method.\n\n\
         OUTPUT — write ONLY one JSON array to `{results_path}`. Each item must have exactly:\n\
         {{\"severity\":\"blocking|major|minor\",\
         \"category\":\"coverage|api|data|runtime|evidence|quality\",\
         \"summary\":\"...\",\"evidence\":[{{\"repo_path\":\"path\",\"line\":1}},\
         {{\"doc_path\":\"path.md\",\"section\":\"Heading\"}}],\
         \"missed_item\":\"...\",\"required_fix\":\"...\"}}\n\
         Missing, malformed, wrapped, or prose-contaminated output is an error.\n\
         {rules}\n\
         ORIGINAL REQUEST:\n{original_request}\n",
        target = dir_display(target_dir),
        rules = DOCS_STEP_RULES,
    )
}

/// Build the resumed final-author repair turn from the combined, persisted
/// findings of one round.
pub fn build_revision_prompt(
    original_request: &str,
    vault_id: i64,
    target_dir: &str,
    iteration: u8,
    findings: &[VaultDocsFinding],
    results_path: &str,
) -> String {
    let findings_json = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".into());
    format!(
        "OTTO_TASK: vault_docs_revise\n\
         Resume as the FINAL AUTHOR for review round {iteration}. Repair the current final \
         documentation under {target} in Otto vault {vault_id}.\n\n\
         REVISION RULES — follow exactly:\n\
         - Re-read the affected source and current notes; apply every valid required_fix below.\n\
         - Preserve correct existing material. Resolve the underlying omission, not only its wording.\n\
         - Refresh `coverage.md`, neighboring `index.md` files, links, examples, diagrams, and text \
         artifacts affected by a repair.\n\
         - Run `otto_vault_okf_validate` and the bundle audit; fix every introduced error.\n\
         - Write through the guarded Otto vault tools only.\n\
         {rules}\n\
         FINALLY, write ONLY this JSON shape to `{results_path}` with every vault-relative path \
         changed in this revision: {{\"written\": [\"path/to/changed.md\", ...]}}\n\n\
         ORIGINAL REQUEST:\n{original_request}\n\n\
         COMBINED SOURCE-BACKED FINDINGS:\n{findings_json}\n",
        target = dir_display(target_dir),
        rules = DOCS_STEP_RULES,
    )
}

/// Parse the agent-written results file TOLERANTLY: accept a bare JSON array,
/// a `{"written": [...]}` object (direct or buried in prose via the shared
/// balanced-object extractor), normalize paths (strip `./` and leading `/`),
/// drop empties, dedupe preserving order. `None` = nothing parseable.
pub(super) fn parse_results(content: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(content.trim())
        .ok()
        .or_else(|| otto_product::run::extract_json_block(content))?;
    let arr = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(_) => v.get("written")?.as_array()?.clone(),
        _ => return None,
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            let p = s.trim().trim_start_matches("./").trim_start_matches('/');
            if !p.is_empty() && seen.insert(p.to_string()) {
                out.push(p.to_string());
            }
        }
    }
    Some(out)
}

/// Fallback `written`: notes present after but not before, under `target_dir`
/// (`""` = anywhere), never counting leftover drafts. Sorted for determinism.
pub(super) fn written_fallback(
    before: &HashSet<String>,
    after: &HashSet<String>,
    target_dir: &str,
) -> Vec<String> {
    let prefix = if target_dir.is_empty() {
        String::new()
    } else {
        format!("{target_dir}/")
    };
    let mut out: Vec<String> = after
        .difference(before)
        .filter(|p| p.starts_with(&prefix) && !p.starts_with("_drafts/"))
        .cloned()
        .collect();
    out.sort();
    out
}
