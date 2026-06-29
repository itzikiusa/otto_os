# Run with Otto — v2 hardening design (gap closure + full E2E)

> Companion to `run-with-otto-design.md` / `run-with-otto-plan.md` (the v1 build)
> and `docs/features/run-with-otto.md` (the authoritative feature guide).
> This spec covers the work that turns the already-shipped v1 into a *complete,
> fully-tested flagship*: it closes the honestly-documented v1 gaps and brings
> E2E coverage up to "every requirement, end to end".

## 0. Why this spec exists

The flagship **Run with Otto** flow is already built and committed (commits
`8dee073f`, `75e71553`, `d3a7610a`, `5466c100`): `OttoRun` + the `RunStatus`
stage machine (`otto-core/src/run.rs`), the engine/service/scheduler/sources/
context/workspace/channels (`otto-server/src/run_*.rs`), persistence
(`otto-state/src/runs.rs` + migration `0087`), routes (`routes/runs.rs`,
`routes/channel_webhook.rs`), the UI module (`ui/src/modules/run-with-otto/`),
and an E2E spec. All eight entry points resolve; the full pipeline
`source → work item → context packet → worktree → goal-loop|single-agent →
proof pack → AI review → human approval → PR draft` runs.

So this is **not** a from-scratch build. An exhaustive audit (six parallel
code-mapping passes + direct reads of every `run_*.rs`) found the requirements
met at the code level, with good unit coverage in `run.rs` (5), `runs.rs` (5),
`run_sources.rs` (3), `run_engine.rs` (2), `run_channels.rs` (2),
`run_context.rs` (1), and an E2E spec covering awaiting_approval → approve →
completed, reject, detect, workspace scoping, and the launcher UI.

The audit also found **three real gaps** that keep the feature from being
"every requirement fulfilled + full E2E":

1. **Webhook callback is half-built.** `otto_runs.callback_url` is captured on
   launch (`run_service.rs:79`, `channel_webhook.rs:345`) but **never
   delivered** — a webhook caller gets `202 {run_id}` and then nothing. The
   feature doc admits this ("Webhook result callback is minimal"). The outbound
   half exists for the *generic* webhook (`otto-channels::WebhookAdapter`,
   netguard-guarded) but the Run-with-Otto path never uses it.
2. **The safety-critical approval / open-PR gates have zero unit tests.**
   `run_service.rs` (approve/reject CAS, `open_pr` "must be approved AND
   proof passed/waived") has **0** `#[test]`s; `routes/runs.rs` has 0. The
   logic is correct on inspection but unguarded against regression.
3. **E2E gaps:** no `cancel` test, no `open-pr` gating test, and only the
   `channel` source is exercised end-to-end (the other data-backed sources —
   product story / finding / scheduled report — are unit-tested in adapters but
   never driven through the live pipeline).

**Explicitly out of scope** (each already documented as a v1 deferral, and each
a separate feature, not a gap in *this* flow):
- An `otto.run` **MCP tool**. `otto-mcp` is an *outbound* MCP client/control-
  plane (it calls external servers with allowlist/risk gating); Otto has no
  MCP-server surface to host a tool on. The v1 plan explicitly trimmed this
  ("20.11 … no MCP task"). `RunOrigin::Mcp` already exists for when that
  surface lands.
- Deep Slack/Telegram **thread-history** ingestion (channel source = the
  trigger's seed message), Slack **Block-Kit** approval buttons (thread
  `approve`/`reject` replies work), CI/unit-test **failing-test** ingestion
  ("failing test" = a Product QA testcase run), and GitLab/Bitbucket **issues**
  (their PRs work; GitHub issues work).

This keeps the change additive and low-risk: **no working code is rewritten.**

## 1. Requirements → coverage map (the acceptance contract)

Every user requirement, and where it is satisfied + proven after this spec:

| Requirement | Implemented in | Proven by |
|---|---|---|
| Entry: Jira story | `run_sources::resolve_jira` | `run.rs` detect unit test + adapter unit test |
| Entry: Confluence page | `run_sources::resolve_confluence` | `run.rs` detect unit test (URL → page id) |
| Entry: GitHub issue | `run_sources::resolve_github_issue` | `run.rs` detect unit test |
| Entry: GitHub PR | `run_sources::resolve_github_pr` | `run.rs` detect unit + E2E `detect` |
| Entry: Slack/Telegram thread | `run_channels` + `resolve_channel` | `run_channels` unit tests + E2E `channel` run |
| Entry: Product task | `run_sources::resolve_product_story` | **NEW** E2E `story:<id>` run |
| Entry: Review finding | `run_sources::resolve_finding` | adapter path + detect unit test |
| Entry: Failing test | `run_sources::resolve_test` | adapter path + detect unit test |
| Entry: Scheduled-task report | `run_sources::resolve_scheduled_report` | adapter path + detect unit test |
| source item → Otto Work Item | `run_service::launch` → `otto_runs` + `run_engine::project` (Mission Control) | E2E run rows + existing projection |
| → Context Packet | `run_context::build_packet` | `run_context` unit test |
| → isolated branch/worktree | `run_workspace::provision_worktree` (`otto-run/<id>`) | E2E asserts `branch` contains `otto-run/` |
| → Goal Loop *or* single agent | `run_engine::stage_execute` (both modes) | `run_engine` goal-loop construction unit test + E2E single-agent |
| → Proof Pack | `run_engine::stage_prove` (`proof::gate`/`assemble_diff`/`recompute`) | E2E asserts `proof_pack_id` + `proof_status` |
| → AI review findings | `run_engine::stage_review` (`run_review_for_branch`) | E2E asserts `findings_*`; offline review seam |
| → human approval | `run_service::approve` (CAS gate) | **NEW** decision/gate unit tests + E2E approve/reject |
| → PR draft | `run_engine::stage_draft_pr` (`draft_pr_core`) | E2E asserts `pr_draft_json` |
| "one button, not eight modules" | UI launcher + sidebar + ⌘K command + Slack `/run` + webhook + REST | E2E UI test + this doc's surface table |
| outward PR open gated | `run_service::open_pr` (approved + proof passed/waived) | **NEW** `open_pr_block_reason` unit tests + E2E 409 gate |
| webhook round-trip | `channel_webhook::run_inbound` + **NEW** callback delivery | **NEW** callback payload unit test |

The three **NEW** rows are this spec's deliverables.

## 2. Component design

### 2.1 Webhook callback delivery — `crates/otto-server/src/run_callback.rs` (new)

A small, self-contained module mirroring `otto-channels::WebhookAdapter::post`.

```rust
/// Build the compact JSON result a webhook caller receives. Pure (no I/O) so it
/// is unit-testable. Never includes secrets; the body is the run's public shape.
pub(crate) fn build_payload(run: &OttoRun) -> serde_json::Value
// → { "run_id", "workspace_id", "status", "title", "source_kind", "source_ref",
//     "mode", "proof_status", "risk_score", "findings_total",
//     "findings_blocking", "has_pr_draft", "pr_url", "approval_decision",
//     "error", "awaiting_approval": bool, "terminal": bool }

/// Best-effort POST of the result to `run.callback_url`. No-op when the run has
/// no callback URL (every non-webhook origin). SSRF-guarded via
/// otto_netguard::check_url + redirect_policy (a key-holder cannot turn Otto
/// into an SSRF proxy). Records a `delivery` run-event (ok | blocked | error)
/// for transparency. Never fatal — a failed callback never fails the run.
pub(crate) async fn deliver(ctx: &ServerCtx, run: &OttoRun)
```

The reqwest client is built per call with a 20 s timeout and
`otto_netguard::redirect_policy()` (identical posture to `WebhookAdapter`).

**Delivery points** (each self-gates on `callback_url.is_some()`):
- `run_engine::after_transition` → on `AwaitingApproval` and `Completed`
  (right after `post_origin`).
- `run_engine::fail` → on `Failed`.
- `run_service::approve` (reject branch) → on `Rejected`; `run_service::cancel`
  → on `Cancelled`. Both already re-`get` the fresh run and `project` it; the
  callback call slots in beside that.

So a webhook caller receives exactly the milestones it can act on:
awaiting_approval, completed, failed, rejected, cancelled.

### 2.2 Pure approval / open-PR gates — `crates/otto-core/src/run.rs`

Extract the safety logic that currently lives inline in `run_service` into pure,
exhaustively-tested functions (behavior-preserving — the caller maps each result
to the *same* `Error` variant and message it produces today).

```rust
pub enum ApprovalDecision { Approve, Reject }
/// Case-insensitive, trimmed. approve|approved|yes → Approve;
/// reject|rejected|no → Reject; anything else → None.
pub fn parse_decision(raw: &str) -> Option<ApprovalDecision>

pub enum OpenPrBlock { NotApproved, ProofNotPassed, NoDraft, NoRepo }
impl OpenPrBlock { pub fn message(&self) -> &'static str }
/// None == may open. Order matches today's checks: approval → proof → draft → repo.
pub fn open_pr_block_reason(
    approval_decision: Option<&str>,
    proof_status: Option<&str>,
    has_draft: bool,
    has_repo: bool,
) -> Option<OpenPrBlock>
```

`run_service::approve` switches on `parse_decision`; `open_pr` calls
`open_pr_block_reason` and maps `NotApproved/NoDraft/NoRepo → Error::Invalid`,
`ProofNotPassed → Error::Conflict` (exactly as today). No HTTP-status or
message changes — the existing E2E `404`/scope assertions and the new `409`
assertion stay valid.

### 2.3 E2E additions — `ui/e2e/desktop-run-with-otto.spec.ts`

Add to the existing spec (same isolated-daemon harness, `OTTO_E2E=1` seam):
- **cancel:** launch a fresh channel run; `POST /runs/{id}/cancel`; assert
  `status == "cancelled"`.
- **open-pr gate:** `POST /runs/{seededRunId}/open-pr` *before* approval →
  expect non-OK with `run is not approved`; after the existing approve→completed
  test, `POST open-pr` → expect `409` (`proof pack is not passed/waived` — the
  e2e change has a diff but no passing test, so proof is `partial`). This proves
  the outward-action gate without needing a live git provider.
- **non-channel source:** seed a Product story (via the product REST API in the
  harness), launch `{ source_ref: "story:<id>" }`, drive to awaiting_approval,
  assert `source_kind == "product_story"` and the same evidence
  (`branch`/`proof_pack_id`).

If seeding a webhook integration in the harness is cheap, also assert
`POST /webhooks/{ws}/run` returns `202 {run_id}` and the run reaches
awaiting_approval; otherwise the webhook launch funnel stays covered by the
`run_service::launch` path the channel test already exercises, and callback
delivery by the §2.2 payload unit test. (Documented, not hidden.)

### 2.4 UI completeness

`RunDetail.svelte` must expose the human-facing actions the flow promises:
**Approve**, **Reject**, **Open PR** (when completed + gate allows), and
**Cancel** (when active). Audit the component; add only the actions that are
missing, following the existing store/`runWithOttoApi` pattern. No redesign.

## 3. Non-functional constraints (verbatim from repo rules)

- **Contracts first.** Any endpoint/WS change updates `docs/contracts/*.md` and
  `ui/src/lib/api/types.ts` in lockstep. (This spec adds **no** new endpoint and
  **no** typed DTO consumed by the UI — the callback body is webhook-only JSON,
  documented in `api.md` prose + the feature guide.)
- **Migrations are append-only.** This spec adds **no** migration (the `0087`
  schema already has every column, incl. `callback_url`).
- **Secrets never in the repo.** The callback never includes tokens; it is the
  run's public read shape.
- **Loopback-only defaults unchanged; no new listeners.** The only new outbound
  is the SSRF-guarded callback POST.
- **No PR opened without approval + proof passed/waived.** Preserved exactly.
- **Match surrounding code.** Dense, intentional style; pure logic in
  `otto-core`, I/O in `otto-server`, mirror the `WebhookAdapter` for outbound.

## 4. Verification gates (all must pass before merge)

1. `cargo fmt --all` (touched files clean).
2. `cargo build --workspace`.
3. `cargo clippy --workspace --all-targets -- -D warnings`.
4. `cargo test --workspace` (incl. the new `run.rs` + `run_callback.rs` tests).
5. `cd ui && npm run check` (svelte-check + tsc).
6. `cd ui && npm run build`.
7. `cd ui && npx playwright test desktop-run-with-otto --project=desktop-browser`
   (and a broader sanity sweep).

## 5. Risks & mitigations

- **Behavior drift in the gate refactor.** Mitigated by keeping the caller's
  `Error` mapping identical and asserting it with both unit tests and the E2E
  `409`/reject paths.
- **Callback SSRF.** Mitigated by `otto_netguard::check_url` + `redirect_policy`
  (same guard the generic webhook already trusts).
- **E2E flake from real timing.** Mitigated by reusing the existing
  `pollUntil` helper and the deterministic `OTTO_E2E` seam.
- **Damaging working v1 code.** Mitigated by an additive-only change set: one new
  module, pure helpers + their call-site swap, test/doc additions, and (if
  needed) missing UI buttons. No stage logic is rewritten.
