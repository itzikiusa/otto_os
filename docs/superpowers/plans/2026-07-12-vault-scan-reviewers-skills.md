# Vault Scan Iterative Reviewers and Skill Depth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional, visible multi-reviewer/revision loop to Vault docs runs and ship complete OKF/repo-scan/reviewer skill packages that make full-scan coverage auditable.

**Architecture:** Existing writers and the optional summarizer continue to produce the final bundle. When review is configured, independent read-only reviewers write structured findings, the final author's existing managed session revises the bundle, and the server repeats up to the configured cap with an early clean exit. Complete skill trees are materialized for every provider, while source-backed inventory/audit scripts and a coverage ledger enforce scan depth.

**Tech Stack:** Rust/Axum/Tokio/Serde/SQLite, Svelte 5/TypeScript, Python 3 stdlib scripts, Playwright.

## Global Constraints

- Existing run requests without `review` remain backward compatible.
- Review configuration accepts 1–4 reviewers and 1–10 maximum iterations; default 3.
- Reviewers never edit Vault notes; the final author owns every revision.
- Early exit requires valid empty findings from every reviewer in the same round.
- Missing/malformed reviewer output is an error, not a clean verdict.
- Full scans create `coverage.md`; no discovered candidate is silently omitted.
- OpenAPI YAML and other approved text artifacts are written through a guarded
  Vault file API/MCP tool, never by bypassing the engine with filesystem writes.
- Skills use progressive disclosure: concise `SKILL.md`, focused one-level references, deterministic scripts, realistic examples, and evals.
- Migrations are append-only; this design needs no schema migration because run details remain in `payload`.
- Preserve unrelated user changes and run only named Playwright specs.

---

## File structure

### Backend

- Modify `crates/otto-server/src/vault_docs_agent.rs`: review DTOs, validation, orchestration, prompts, retries, cancel/recovery.
- Modify `crates/otto-state/src/vault_docs_runs.rs`: active-state recovery predicate.
- Modify `crates/otto-skills/src/lib.rs`: complete bundled-tree materialization and reviewer roster assertions.
- Modify `crates/otto-server/src/modules.rs`: provider-neutral staged package bundle.
- Modify session MCP tool filtering at the existing source-based policy point so `meta.source="vault-docs-review"` omits Vault mutation tools.
- Modify `crates/otto-vault/src/{engine,http,types}.rs` and session MCP wiring:
  guarded text-artifact writes used by OpenAPI/full-scan deliverables.

### UI/contracts/docs

- Modify `ui/src/lib/api/types.ts`, `ui/src/lib/api/vault.ts`, `ui/src/modules/vault/DocsAgentsView.svelte`, and `ui/src/modules/vault/docsTemplates.ts`.
- Modify `docs/contracts/api.md` and `docs/features/vault.md`.
- Modify focused specs `ui/e2e/desktop-vault-agents.spec.ts` and `ui/e2e/desktop-vault-agent-runs.spec.ts`.

### Skill packages

- Expand `crates/otto-skills/assets/skills/development/okf-authoring/`.
- Expand `crates/otto-skills/assets/skills/development/vault-repo-docs/`.
- Create `vault-docs-review`, `vault-api-review`, `vault-data-review`, `vault-runtime-review`, and `vault-evidence-review` under the same category.

---

### Task 1: Complete multi-file skill materialization

**Files:**
- Modify: `crates/otto-skills/src/lib.rs`
- Modify: `crates/otto-server/src/modules.rs`
- Modify: `crates/otto-server/src/vault_docs_agent.rs`
- Test: inline unit tests in those modules

**Interfaces:**
- Produces: `otto_skills::copy_bundled_into(name: &str, dest: &Path) -> io::Result<bool>`.
- Produces: `StagedSkillPackages { root: String, files: HashMap<String, Vec<String>> }` from `stage_skill_packages(...)`.
- Consumes later: writer, summarizer, reviewer, and revision prompt builders use `package_prompt(provider, staged, names)`.

- [ ] **Step 1: Write failing bundled-copy tests.** Assert that compiled-in `okf-authoring` copies `SKILL.md` plus a seeded reference/script/example path, rejects unsafe names/paths, and preserves executable script bits.
- [ ] **Step 2: Run RED.** `cargo test -p otto-skills bundled_skill_tree -- --nocapture`; expect failure because the complete package API/resources do not exist.
- [ ] **Step 3: Implement `copy_bundled_into`.** Reuse `seed_dir`, validate the skill name through `bundled_dir`, and never remove an arbitrary caller path.
- [ ] **Step 4: Write failing stage tests.** Exercise Library-first and bundled-fallback sources, assert both `.claude/skills/<name>` and `skills/<name>` views contain all files, and assert non-Claude prompt instructions reference the neutral path instead of inlining the body.
- [ ] **Step 5: Run RED.** `cargo test -p otto-server stage_skill_packages -- --nocapture`; expect missing API/old inline behavior.
- [ ] **Step 6: Implement provider-neutral staging.** Materialize complete package trees, return file manifests, retain Claude `extra_dirs`, and give other providers an explicit package root/resource-routing prompt.
- [ ] **Step 7: Run GREEN.** Re-run both named test filters and `cargo test -p otto-skills`.

### Task 2: Upgrade `okf-authoring`

**Files:**
- Modify: `crates/otto-skills/assets/skills/development/okf-authoring/SKILL.md`
- Create: `references/{spec-v0.1,concept-patterns,linking-indexes-logs,quality-gates}.md`
- Create: `scripts/{validate_okf,audit_bundle}.py`
- Create: `examples/{complete-api-endpoint,complete-data-asset,maintain-before-after}.md`
- Create: `evals/evals.json` and fixture bundles
- Test: `crates/otto-skills/assets/skills/development/okf-authoring/scripts/test_*.py`

**Interfaces:**
- Produces: `validate_okf.py ROOT [--format json|text]` with nonzero exit only for conformance errors.
- Produces: `audit_bundle.py ROOT [--format json|text]` with findings `{rule,path,message,severity}`.

- [ ] **Step 1: Record RED baseline evidence.** Save the observed no-skill/current-skill omissions from the independent API/data/reviewer baseline runs into eval expectations, without embedding model-specific prose.
- [ ] **Step 2: Write failing script tests.** Fixtures cover missing frontmatter/type, reserved-file violations, missing endpoint request/response sections, shallow data assets, and a clean bundle.
- [ ] **Step 3: Run RED.** `python3 -m unittest discover -s .../okf-authoring/scripts -p 'test_*.py' -v`; expect missing modules/scripts.
- [ ] **Step 4: Implement stdlib scripts.** Parse frontmatter conservatively, walk deterministically, support JSON/text, never mutate input, and clearly distinguish conformance errors from quality warnings.
- [ ] **Step 5: Rewrite `SKILL.md`.** Add required `name`, bump version, keep the core workflow/routing/completion contract concise, and link each resource by when-to-read condition.
- [ ] **Step 6: Add focused references/examples/evals.** API example includes request/response/error bodies; data example includes full fields, access paths, indexes/TTL/transactions, impact, and citations.
- [ ] **Step 7: Run GREEN and package review.** Run script tests, JSON-parse evals, `cargo test -p otto-skills`, and the bundled `skills-reviewer` static check.

### Task 3: Add guarded text-artifact writes

**Files:**
- Modify: `crates/otto-vault/src/types.rs`
- Modify: `crates/otto-vault/src/engine.rs`
- Modify: `crates/otto-vault/src/http.rs`
- Modify: `crates/ottod/src/mcp_tools.rs`
- Modify: `docs/contracts/api.md`
- Test: `crates/otto-vault/tests/engine.rs` and route/MCP schema tests

**Interfaces:**
- Produces: `WriteTextFileReq { path, content, if_hash }` and `VaultTextFile { path, size, hash }`.
- Produces: `PUT .../vaults/{id}/file` and `otto_vault_write_file`.

- [ ] **Step 1: Write failing engine tests.** Cover `.yaml/.yml/.json/.d2/.mmd/.txt/.csv`, `.md` rejection, binary/unknown extension rejection, traversal/hidden/symlink escapes, 4 MiB cap, parent creation, hash conflicts, and rescan attachment visibility.
- [ ] **Step 2: Run RED.** `cargo test -p otto-vault write_text_file -- --nocapture`; expect missing method/types.
- [ ] **Step 3: Implement guarded engine write.** Reuse canonical path guards and hashing semantics from note writes without invoking Markdown parsing; write atomically where the existing engine supports it and rescan before returning.
- [ ] **Step 4: Write failing API/MCP tests.** Assert editor role, exact request/response schema, mutation classification, and reviewer-session omission.
- [ ] **Step 5: Implement route/tool wiring.** Keep `otto_vault_write` Markdown-only and expose the new tool only to writable docs-agent sessions.
- [ ] **Step 6: Run GREEN.** Run named Vault engine/server/MCP tests and `cargo test -p otto-vault`.

### Task 4: Upgrade `vault-repo-docs` and full-scan coverage

**Files:**
- Modify: `crates/otto-skills/assets/skills/development/vault-repo-docs/SKILL.md`
- Create: `references/{full-scan-method,api-documentation,datastore-documentation,flows-messaging-workers,evidence-and-citations}.md`
- Create: `scripts/{inventory_repo,audit_repo_bundle}.py`
- Create: `examples/full-scan-manifest.json`, `examples/api-flow-bundle/*`, `examples/datastore-impact-bundle/*`
- Create: `evals/evals.json`, fixtures, and `scripts/test_*.py`
- Modify: `ui/src/modules/vault/docsTemplates.ts`

**Interfaces:**
- Produces: manifest `{version,repo,commit,generated_at,candidates[]}` where each candidate has `{id,kind,name,path,line,evidence}`.
- Produces: coverage rows with `status=documented|irrelevant|generated|uncertain`, `doc`, and `reason`.
- Consumes: `otto_vault_write_file` for `api-openapi.yaml` and other approved text artifacts.

- [ ] **Step 1: Write failing inventory tests.** Seed a compact polyglot fixture containing an Axum route/DTO, SQL migration/query, Redis key, Kafka producer/consumer, and scheduled worker; assert stable candidates and `file:line` evidence.
- [ ] **Step 2: Run RED.** Execute the script unittest discovery; expect missing inventory implementation.
- [ ] **Step 3: Implement conservative inventory.** Use stdlib only, respect common ignored/build/vendor directories, cap file size, emit candidates rather than semantic claims, and expose `--format json`.
- [ ] **Step 4: Write failing bundle-audit tests.** Assert missing coverage rows, missing API bodies/examples/OpenAPI operation, shallow DB access/impact sections, and missed workers are reported; a complete fixture is clean.
- [ ] **Step 5: Implement audit + skill resources.** Add exact full-scan phases, write-order, coverage ledger, API/data/runtime completion contracts, uncertainty handling, examples, and evals.
- [ ] **Step 6: Tighten prepared prompts.** Require inventory + `coverage.md`, the audit command, and final reconciliation for full/focused scans; incremental mode updates the ledger only for changed candidates.
- [ ] **Step 7: Run GREEN and static skill review.** Run script tests, eval JSON checks, TypeScript formatting/type check for the template, and package review.

### Task 5: Add generic and focused reviewer skills

**Files:**
- Create: `crates/otto-skills/assets/skills/development/vault-docs-review/`
- Create: `.../vault-api-review/`, `.../vault-data-review/`, `.../vault-runtime-review/`, `.../vault-evidence-review/`
- Modify: `crates/otto-skills/src/lib.rs` tests

**Interfaces:**
- All reviewer skills output the shared JSON finding array from the design.
- `vault-docs-review` is the default method; focused methods never waive evidence requirements.

- [ ] **Step 1: Add eval fixtures first.** Each package gets positive/negative activation, seeded omission, clean-control, malformed evidence, and convergence cases.
- [ ] **Step 2: Run RED.** Assert `list_bundled()` contains all five and each package has `SKILL.md`, reference/checklist, example, and `evals/evals.json`; expect failure.
- [ ] **Step 3: Author one package at a time.** For each, add concise method, exact review scope, evidence threshold, findings schema/example, clean verdict rule, common false positives, and evals; validate before moving to the next package.
- [ ] **Step 4: Run GREEN.** `cargo test -p otto-skills reviewer_skills -- --nocapture`, parse every eval JSON, and run static skill review for all five.

### Task 6: Add review DTOs, validation, state, and prompts

**Files:**
- Modify: `crates/otto-server/src/vault_docs_agent.rs`
- Modify: `crates/otto-state/src/vault_docs_runs.rs`

**Interfaces:**
- Produces: `ReviewReq`, `ReviewerReq`, `VaultDocsReview`, `VaultDocsReviewRound`, `VaultDocsReviewer`, `VaultDocsRevision`, `VaultDocsFinding`.
- Produces pure helpers: `parse_review_findings`, `all_reviewers_clean`, `next_review_action`, `build_reviewer_prompt`, `build_revision_prompt`.

- [ ] **Step 1: Write failing DTO/default tests.** Legacy payloads deserialize with review skipped; request defaults method/max; invalid counts/ranges/methods reject with actionable messages.
- [ ] **Step 2: Run RED.** `cargo test -p otto-server vault_docs_agent::tests::review_ -- --nocapture`.
- [ ] **Step 3: Implement minimal DTOs/validation.** Use serde defaults and a constant allowlist matching bundled reviewer methods.
- [ ] **Step 4: Write failing state/prompt tests.** Cover valid empty/filled findings, malformed output, same-round clean, exhausted outcome, reviewer evidence/focus/package path, and revision results contract.
- [ ] **Step 5: Implement helpers and active states.** Extend terminal/interrupt logic and `list_unfinished()` to `reviewing|revising`.
- [ ] **Step 6: Run GREEN.** Re-run named server/state tests.

### Task 7: Implement iterative review orchestration and controls

**Files:**
- Modify: `crates/otto-server/src/vault_docs_agent.rs`
- Modify: the session MCP source-filter module discovered by `rg 'otto_vault_write' crates/ottod crates/otto-server`
- Modify: route policy tests if new routes need explicit coverage

**Interfaces:**
- Adds routes:
  - `POST /vault/docs-agents/runs/{id}/review/rounds/{iteration}/reviewers/{index}/retry`
  - `POST /vault/docs-agents/runs/{id}/review/rounds/{iteration}/revision/retry`
- Extends existing cancel/recovery across reviewer/revision sessions.

- [ ] **Step 1: Write failing orchestration tests around pure/testable stage driver boundaries.** Cover reviewer fan-out, clean round, findings→revision→next round, exhaustion, partial failure, and persistence transitions.
- [ ] **Step 2: Run RED.** Execute the named orchestration test filter.
- [ ] **Step 3: Run reviewers concurrently.** Create isolated result files, staged method packages, session metadata/source, visible session IDs, parsed findings, and persistence updates.
- [ ] **Step 4: Resume the final author for revisions.** Reuse single writer/summarizer session ID, send combined findings, require changed-path results, rescan, persist, then start the next round.
- [ ] **Step 5: Implement retry/cancel/recovery.** Reuse capped retry flags, terminate active sessions, and transform active nested states on restart.
- [ ] **Step 6: Enforce read-only reviewer MCP.** Filter Vault write/rename/delete tools when `meta.source` is `vault-docs-review`; test that read tools remain.
- [ ] **Step 7: Run GREEN.** Run all `vault_docs_agent` tests plus policy/source-filter tests.

### Task 8: Add reviewer configuration and visible rounds to the UI

**Files:**
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/api/vault.ts`
- Modify: `ui/src/modules/vault/DocsAgentsView.svelte`
- Modify: `ui/src/modules/vault/vault.svelte.ts` if active-state helpers are centralized there

**Interfaces:**
- Mirrors backend review DTOs and retry functions exactly.
- Form sends no `review` block when disabled.

- [ ] **Step 1: Add failing focused Playwright assertions.** Configure generic + focused reviewers, max rounds 3, submit payload, show `reviewing round 1/3`, findings, revision, clean/exhausted outcomes, retry, and history reload.
- [ ] **Step 2: Run RED.** `npm run test:e2e -- desktop-vault-agents.spec.ts desktop-vault-agent-runs.spec.ts`; expect missing controls/rows.
- [ ] **Step 3: Update types/API.** Add exact unions, nested DTOs, request block, and retry calls.
- [ ] **Step 4: Implement form.** Optional toggle, default generic row, method/provider/model/focus, add/remove cap 4, max iterations default 3/range 1–10.
- [ ] **Step 5: Implement run detail.** Render rounds, findings, revision changed paths, live terminals, states, retry controls, and `done_with_findings` styling; active polling includes reviewing/revising.
- [ ] **Step 6: Run GREEN.** Run `npm run check`, `npm run build`, and the two named specs only.

### Task 9: Contracts, feature docs, and independent reviews

**Files:**
- Modify: `docs/contracts/api.md`
- Modify: `docs/features/vault.md`

**Interfaces:**
- Documents exact request/response bodies and failure/recovery semantics.

- [ ] **Step 1: Update contract and feature guide.** Cover review request/DTOs/states/routes, UI lifecycle, skill roster, complete package delivery, coverage ledger, full-scan API/data gates, and troubleshooting.
- [ ] **Step 2: Run doc/package checks.** `git diff --check`, skill static reviews, eval JSON parsing, and script help/smoke commands.
- [ ] **Step 3: Dispatch fresh independent code review.** Review the branch diff against the design with correctness, test, architecture, and skill-package lenses; provide only requirements, base/head SHAs, and raw artifacts.
- [ ] **Step 4: Fix verified findings test-first.** Re-run the narrow command proving each issue before and after the fix.

### Task 10: Full verification, merge, rebuild, install, and runtime proof

**Files:**
- Modify only files required by verified failures.

- [ ] **Step 1: Run final Rust verification.** `cargo test -p otto-skills -p otto-state -p otto-server` and `cargo clippy -p otto-skills -p otto-state -p otto-server --all-targets -- -D warnings`.
- [ ] **Step 2: Run skill/script verification.** Execute every new `test_*.py`, parse every `evals/evals.json`, run static skill reviews, and forward-test selected generic/API/data cases.
- [ ] **Step 3: Run final UI verification.** `npm run check`, `npm run build`, and only `desktop-vault-agents.spec.ts` plus `desktop-vault-agent-runs.spec.ts`.
- [ ] **Step 4: Commit focused concerns.** Use the repo emoji/conventional style, no invented Jira key, no attribution, and preserve unrelated main changes.
- [ ] **Step 5: Merge into current local main safely.** Fetch current main state, merge without rewriting history, resolve only overlapping feature files, and verify the resulting tree.
- [ ] **Step 6: Rebuild and install.** Run the repository's documented macOS deployment script from the merged main checkout, including the worktree sidecar-binary directory prerequisite if deploying before merge.
- [ ] **Step 7: Prove the installed runtime.** Verify app/daemon process health, probe the new review-capable request contract or bundled reviewer-skill endpoint, and confirm the running app reports the new UI/assets rather than relying only on a deploy log marker.
