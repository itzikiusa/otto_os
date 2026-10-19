# Engineering 02 — Workflows — iteration 01

Verdict: approve with fixes pending execution verification. Three confirmed major defects repaired; Cargo verification queued with the coordinating agent. Provisional confidence score: **8.5/10**, not 9.8. This is a focused subsystem review, not proof that every integration or every line of the 18,198-line inventory is correct.

Snapshot: `c009da20cc113c8fb79f3bf0600442d0c6afa358`, branch shared with other reviewers. Findings include pre-existing subsystem behavior, not just PR 111's DB diff. File references below identify the repaired working tree. No commits, remote writes, real workflow executions, production database queries, or user-session changes were performed.

## Confirmed findings and repairs

1. **Major — DB workflow steps use an invented actor.** `crates/otto-server/src/workflow_engine.rs:4241`. The old adapter passed `workflow-engine` instead of the resolved run user's ID. Concrete trace: root starts a DB step on an enforced connection → `DbViewerService::run` → `execution_access_snap` → `access::current_user` → `UsersRepo::get("workflow-engine")`; the nonexistent user rejects a query that the real root/owner may execute. The same placeholder misattributes query history in legacy mode. Repair passes `user.id` through the existing DB service without changing its access rules. Regression: `workflow_integration_nodes_use_actor_environment_and_auth`, `workflow_recovery_tests.rs:460`, executes the actual node against an enforced connection backed by isolated ClickHouse HTTP responses and asserts `[[42]]` and row count, not merely a mock invocation.

2. **Major — API Run does not implement promised environment/auth behavior.** `crates/otto-server/src/workflow_engine.rs:4763`, adapter `workflow_api_request.rs:10,38`. The old arm used raw reqwest despite the catalog and `docs/features/workflows.md:367` promising API-client environment substitution/auth. Input `url="{{endpoint}}/answer"` fails URL execution; with a literal URL and bearer auth, the Authorization header is omitted. Repair routes through the existing API-client execution handler, retaining its environment/secret resolution, SSH, settings, history and error behavior. Normalizes legacy object headers and JSON bodies, while accepting API-client row headers/explicit body modes. Runtime output retains `{status, body}` with parsed JSON or text, adding `truncated`/`too_large` flags and a warning log for the existing API-client response bounds. Real node regression requires substituted URL, bearer token and configured header on the wire. A pure request-shape test checks legacy JSON strings and explicit raw body behavior. Existing API-client authorization/secret-host rules are reused, not reimplemented.

3. **Major — resumed loop overwrites completed agent handoff.** `crates/otto-server/src/workflow_engine.rs:4948`, `crates/otto-workflows/src/checkpoint.rs:11`. Concrete trace: inner agent succeeds and writes detailed `step1-answer-iter1.md`; restart occurs before the loop finishes; checkpoint driver returns stored success without replay; caller uses restart time as `attempt_started`; `persist_step` treats original mtime as stale and overwrites it from the shorter machine output. Repair returns explicit checkpoint adoption metadata; the loop adopts an existing handoff and its deliverable pointer without rewriting it, while still regenerating missing handoffs. Semantic failure output is now passed as the persistence error so it cannot become the successful deliverable candidate. Regression `resumed_loop_keeps_checkpoint_handoff_without_replaying_action`, `workflow_recovery_tests.rs:599`, seeds successful checkpoint + original handoff, uses an inner action that would fail if replayed, asserts original bytes and final deliverable, and keeps attempts at one.

## Coverage and traced behavior

- Graph preflight: all of `otto-workflows/src/validation.rs`; empty graph, duplicate/missing IDs, missing edge endpoints, Kahn cycle detection, expression parsing, required parameters, bounded-loop steps and nested-loop rejection. Existing tests cover combined invalid graphs and dynamic URLs. No additional confirmed validation bug.
- Durable retry: all of `retry.rs` and `checkpoint.rs`; explicit zero, transient-class floor, numeric token boundaries, spawn-class filtering, exhausted persisted error, success adoption, semantic gate failures, unsafe interrupted external actions, saved before/after outcomes. Read existing checkpoint regression assertions and evaluated whether replay would fail them.
- Trigger subsystem: all `triggers.rs`; supported event mapping, self-trigger suppression, workspace matching, flat filters, atomic active-run admission, delivery/default helpers. Read schedule input/admission/cursor functions in `workflow_trigger_scheduler.rs`; schedule eligibility and admission tests were inventoried, not executed yet.
- Chat: parser/control entry points and start/control implementation in `chat.rs`; name/binding matching, active-thread targeting, run creation, status, skip and abort. Inspected engine skip gate to reject the initial suspicion that a loop's inner approval could be skipped: engine explicitly guards `parked_at_approval`, so this is **not** a finding.
- Execution/recovery: inspected `classify_resume`, restart-safe allowlist, interrupted-node settlement, main run preflight, branch decisions, scoped prior-state adoption, node cancellation/select and retry paths, loop execution/checkpoints, and DB/API integration arms. Read existing recovery tests including failed-loop stale output and review gate coverage.
- Runtime context: all location matching helpers and prepare-context/Jira extraction; context file registry reload/merge, step persistence, adopted deliverables and final-output behavior. No new path restriction was added.
- Contract cross-check: catalog output/parameter declarations, API DTO, API execution environment/auth/history implementation, DB actor enforcement and native-check exemptions.

## Verification and residual risk

Completed: focused source tracing; `rustfmt` on edited Rust files; scoped `git diff --check` clean.

Queued, not claimed passing:

- `cargo test -p otto-server --lib workflow_integration_nodes_use_actor_environment_and_auth`
- `cargo test -p otto-server --lib resumed_loop_keeps_checkpoint_handoff_without_replaying_action`
- `cargo test -p otto-server --lib api_request::tests`
- `cargo test -p otto-workflows --lib`
- Scoped server clippy/type checking coordinated with the other agents.

Residuals: live provider sessions, real SSH/auth/Keychain integrations and UI trigger flows have not been exercised. API-client reuse uses that engine's bounded response model; added `truncated`/`too_large` output flags and a warning make this explicit. NodeRunState.output is already free-form JSON/TypeScript unknown, so no envelope type changes are needed. The schedule, chat and host runtime paths are traced selectively, not exhaustively. API non-2xx responses remain successful transport outputs with their status, as before; network/configuration failures remain errors. No claim of a 9.8 score is supportable until tests and follow-up review complete.

## Root package verification

`cargo test -p otto-workflows --lib -- --test-threads=2`: **62 passed, 0 failed,
0 ignored**, 1.72s. Log /tmp/otto-product-wide-workflows-lib.log. This closes the
library item above, not server-hosted DB/API integration, resumed-loop execution,
or scheduler admission tests. Cargo released to Pele after this batch. Recovery
and product-publication test sources now live in server tests/unit (module names
unchanged) as part of the LOC extraction; no test behavior intentionally changed.
