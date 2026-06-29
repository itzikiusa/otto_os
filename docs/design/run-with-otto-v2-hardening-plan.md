# Run with Otto — v2 hardening Implementation Plan

> **For agentic workers:** implement task-by-task, TDD, commit per task.
> Companion to `run-with-otto-v2-hardening-design.md`.

**Goal:** Turn the already-shipped Run with Otto v1 into a complete, fully-tested
flagship by closing three real gaps (webhook callback delivery, untested
approval/open-PR gates, E2E gaps) — additively, without rewriting working code.

**Architecture:** Pure gate/decision logic lands in `otto-core` (unit-tested);
the webhook callback is a new self-contained `otto-server` module mirroring
`otto-channels::WebhookAdapter` (SSRF-guarded, best-effort); E2E grows on the
existing isolated-daemon harness.

**Tech Stack:** Rust (axum/sqlx/tokio), `otto-netguard`, reqwest; Svelte 5 +
Playwright.

## Global Constraints

- No new migration (the `0087` schema already has `callback_url` + every field).
- No new REST endpoint; no new typed UI DTO. Callback body is webhook-only JSON,
  documented in `docs/contracts/api.md` prose + `docs/features/run-with-otto.md`.
- Behavior-preserving refactor: the gate helpers map to the EXACT `Error`
  variants/messages `run_service` produces today.
- SSRF: callback uses `otto_netguard::check_url` + `redirect_policy`.
- No PR opened without `approval_decision == "approved"` AND proof
  `passed|waived`. Loopback-only defaults unchanged.
- Run `cargo fmt` on touched files; `cargo clippy --workspace --all-targets -D
  warnings` and `cargo test --workspace` must pass; `ui`: `npm run check`,
  `npm run build`, Playwright `desktop-run-with-otto` green.

---

### Task 1: Pure approval-decision + open-PR-gate helpers (otto-core)

**Files:**
- Modify: `crates/otto-core/src/run.rs` (add helpers + `#[cfg(test)]` cases)

**Interfaces — Produces:**
- `pub enum ApprovalDecision { Approve, Reject }`
- `pub fn parse_decision(raw: &str) -> Option<ApprovalDecision>`
- `pub enum OpenPrBlock { NotApproved, ProofNotPassed, NoDraft, NoRepo }` with
  `pub fn message(&self) -> &'static str`
- `pub fn open_pr_block_reason(approval_decision: Option<&str>, proof_status: Option<&str>, has_draft: bool, has_repo: bool) -> Option<OpenPrBlock>`

- [ ] **Step 1 — failing tests** in `run.rs` tests module:
  `parse_decision` accepts `"approve"/"Approved"/" yes "`→Approve,
  `"reject"/"no"`→Reject, `"maybe"`→None; `open_pr_block_reason`:
  not-approved→`NotApproved`; approved+`Some("partial")`→`ProofNotPassed`;
  approved+`Some("passed")`+draft+repo→`None`; approved+passed+no-draft→`NoDraft`;
  approved+passed+draft+no-repo→`NoRepo`; `message()` strings match the v1
  copy (`"run is not approved"`, `"proof pack is not passed/waived — cannot open a PR"`,
  `"run has no PR draft"`, `"run has no repo"`).
- [ ] **Step 2** — `cargo test -p otto-core run::tests` → FAIL (undefined).
- [ ] **Step 3** — implement the enums + functions.
- [ ] **Step 4** — `cargo test -p otto-core` → PASS.
- [ ] **Step 5** — commit `feat(core): pure approval-decision + open-pr gate helpers`.

### Task 2: Route run_service through the pure gates (no behavior change)

**Files:**
- Modify: `crates/otto-server/src/run_service.rs` (`approve` uses
  `parse_decision`; `open_pr` uses `open_pr_block_reason`, mapping
  `ProofNotPassed→Error::Conflict`, others→`Error::Invalid`).

- [ ] **Step 1** — swap `approve`'s `match decision.as_str()` for
  `match parse_decision(&req.decision)`; keep the two CAS arms + the `_ =>
  Error::Invalid("decision must be 'approve' or 'reject'")` for `None`.
- [ ] **Step 2** — swap `open_pr`'s four inline guards for one
  `if let Some(b) = open_pr_block_reason(run.approval_decision.as_deref(),
  run.proof_status.as_deref(), run.pr_draft_json.is_some(), run.repo_id.is_some())
  { return Err(match b { OpenPrBlock::ProofNotPassed => Error::Conflict(...), _
  => Error::Invalid(...) }); }` then proceed to draft parse + provider.
- [ ] **Step 3** — `cargo build -p otto-server` → OK.
- [ ] **Step 4** — `cargo test -p otto-server` → PASS (no regressions).
- [ ] **Step 5** — commit `refactor(server): run_service uses otto-core gate helpers`.

### Task 3: Webhook callback delivery module (otto-server)

**Files:**
- Create: `crates/otto-server/src/run_callback.rs`
- Modify: `crates/otto-server/src/lib.rs` (or wherever modules are declared) to
  add `mod run_callback;`

**Interfaces — Produces:**
- `pub(crate) fn build_payload(run: &OttoRun) -> serde_json::Value`
- `pub(crate) async fn deliver(ctx: &ServerCtx, run: &OttoRun)`

- [ ] **Step 1 — failing test** in `run_callback.rs` tests: `build_payload`
  for a run at `AwaitingApproval` → object with `"status":"awaiting_approval"`,
  `"awaiting_approval":true`, `"terminal":false`, and `run_id`/`source_kind`/
  `findings_total` present; for a `Completed` run with `pr_draft_json` set →
  `"has_pr_draft":true`, `"terminal":true`. (Build the `OttoRun` literal in-test.)
- [ ] **Step 2** — `cargo test -p otto-server run_callback` → FAIL.
- [ ] **Step 3** — implement `build_payload` (pure, see design §2.1) and
  `deliver` (no-op when `callback_url` empty; `otto_netguard::check_url`;
  reqwest client with 20 s timeout + `redirect_policy()`; POST JSON; record a
  `delivery` `NewRunEvent` with ok/blocked/error; never panics/fails the run).
- [ ] **Step 4** — `cargo test -p otto-server run_callback` → PASS.
- [ ] **Step 5** — commit `feat(server): webhook callback delivery for Run with Otto`.

### Task 4: Wire callback delivery into the engine + lifecycle

**Files:**
- Modify: `crates/otto-server/src/run_engine.rs` (`after_transition`:
  `AwaitingApproval`/`Completed`; `fail`: `Failed`)
- Modify: `crates/otto-server/src/run_service.rs` (`approve` reject arm →
  `Rejected`; `cancel` → `Cancelled`)

- [ ] **Step 1** — in `after_transition`, after the `match new_status` origin
  posts, add `run_callback::deliver(ctx, &run).await` for `AwaitingApproval` and
  `Completed` (re-fetch already done — `run` is fresh).
- [ ] **Step 2** — in `fail`, inside the `if let Ok(fresh)` block, add
  `run_callback::deliver(ctx, &fresh).await`.
- [ ] **Step 3** — in `run_service::approve` reject arm and `cancel`, after the
  `project(ctx, &fresh)` call, add `crate::run_callback::deliver(ctx, &fresh).await`.
- [ ] **Step 4** — `cargo build -p otto-server` + `cargo clippy -p otto-server
  --all-targets -- -D warnings` → OK.
- [ ] **Step 5** — commit `feat(server): deliver Run-with-Otto callbacks at gate + terminal`.

### Task 5: E2E — cancel, open-pr gate, non-channel source

**Files:**
- Modify: `ui/e2e/desktop-run-with-otto.spec.ts`
- Check: `ui/e2e/seed.ts` for a product-story seed helper; add one if absent.

- [ ] **Step 1** — add `cancel` test: launch a fresh channel run, `POST
  /runs/{id}/cancel`, assert `status === 'cancelled'`.
- [ ] **Step 2** — add `open-pr gate` test: before approval `POST
  /runs/{seededRunId}/open-pr` → `expect(r.ok()).toBeFalsy()`; after the
  approve→completed test, `POST open-pr` → `expect(r.status()).toBe(409)`.
- [ ] **Step 3** — add `product story source` test: seed a story (product REST),
  launch `{ source_ref: 'story:<id>', repo_id }`, `pollUntil` awaiting_approval,
  assert `source_kind === 'product_story'` + `branch` contains `otto-run/`.
- [ ] **Step 4** — run
  `npx playwright test desktop-run-with-otto --project=desktop-browser` → PASS.
- [ ] **Step 5** — commit `test(e2e): cancel, open-pr gate, product-story source for Run with Otto`.

### Task 6: UI completeness — RunDetail actions

**Files:**
- Modify: `ui/src/modules/run-with-otto/RunDetail.svelte` (+ `lib/api/runWithOtto.ts`
  / store if a call is missing)

- [ ] **Step 1** — read `RunDetail.svelte`; confirm Approve/Reject (at gate),
  Open PR (completed + gate ok), Cancel (active) are present.
- [ ] **Step 2** — add only the missing actions, wired through `runWithOttoApi`
  (`approve`/`cancel`/`open-pr`), matching the existing pattern.
- [ ] **Step 3** — `cd ui && npm run check` → PASS.
- [ ] **Step 4** — commit `feat(ui): complete RunDetail lifecycle actions` (skip if nothing missing).

### Task 7: Docs + contracts

**Files:**
- Modify: `docs/features/run-with-otto.md` (§7: callback "now delivered", with
  the payload shape), `docs/contracts/api.md` (webhook `/run` row: note the
  callback POST + payload), `docs/contracts/ws.md` (unchanged — note only).

- [ ] **Step 1** — update §7 of the feature guide (remove "minimal"; document
  the delivery points + payload fields).
- [ ] **Step 2** — add the callback note to the `POST /webhooks/{ws}/run` row in
  `api.md`.
- [ ] **Step 3** — commit `docs: Run with Otto webhook callback delivery`.

### Task 8: Full verification gate

- [ ] `cargo fmt --all` ; `cargo build --workspace` ; `cargo clippy --workspace
  --all-targets -- -D warnings` ; `cargo test --workspace`.
- [ ] `cd ui && npm run check && npm run build`.
- [ ] `cd ui && npx playwright test desktop-run-with-otto --project=desktop-browser`
  + a sanity sweep (`pages.spec.ts`).
- [ ] Fix any failure before proceeding; re-run until green.

### Task 9: Merge + deploy (local only)

- [ ] Merge `worktree-run-with-otto` → `main` (local, **no push**).
- [ ] Rebuild release `ottod`; swap installed/bundled binary; `launchctl
  kickstart`. Rebuild + sign the Tauri app; force-quit + relaunch + activate the
  running app on the new build (per the "deploy: do it myself" rule).
- [ ] Verify the running daemon serves the new build (health + a run launch).

## Self-Review (spec coverage)

- §0 gaps → T3/T4 (callback), T1/T2 (gate tests), T5 (E2E). ✓
- §1 acceptance map "NEW" rows → T1 (gate unit), T3 (callback unit), T5 (E2E
  story/open-pr/cancel). ✓
- §2.1 callback → T3+T4; §2.2 gates → T1+T2; §2.3 E2E → T5; §2.4 UI → T6. ✓
- §3 constraints (no migration, no DTO, behavior-preserving) honored in T2/T3. ✓
- §4 gates → T8; deploy → T9. ✓
- No placeholders; helper signatures consistent between T1 (define) and T2 (use).
