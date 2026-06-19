# Batch-2 Verification (2026-06-19)

Adversarial, independent verification of the batch-2 hardening pass. No source
files were modified. Build/test/lint were run to completion and parsed for real
exit status (not pipe-trapped).

**Overall: RED** — the security/correctness fixes themselves are largely correct,
but **`cargo clippy --workspace --all-targets -- -D warnings` FAILS** (3 errors),
which is a hard gate the batch-1 pass had made green. Plus one **PARTIAL** on S4:
the otto-issues handlers are fully guarded, but issue-account credentials are
ALSO resolved-and-used outside otto-issues (otto-server `modules.rs`, otto-product
`service.rs`) and those paths were out of agent 1's scope (see S4 below — pending
trace confirmation).

---

## Part A — Gates

| # | Gate | Result | Evidence |
|---|------|--------|----------|
| 1 | `cargo check --workspace` | PASS (2 warnings) | exit 0; dead-code warnings: `PRICED_AS_OF`, `is_priced` in otto-usage/src/pricing.rs |
| 2 | `cargo build --workspace` | PASS (2 warnings) | exit 0; same 2 otto-usage dead-code warnings |
| 3 | `cargo test --workspace` | **PASS** | exit 0; 0 failures across all suites incl. doctests. Named: otto-rbac 6, otto-improve 33, otto-usage 22, otto-dbviewer 75 (+1 ign), otto-swarm 3, otto-issues 62, `auth_security` 5, `route_inventory` 2. DB E2E suites ignored (env-gated). |
| 4 | `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL (exit 101)** | 3 errors — see below. Batch-2-introduced. |
| 5 | `cd ui && npm run check` | PASS | exit 0; 480 files, 0 errors, 0 warnings. No UI files touched by batch 2. |
| 6 | `cargo test -p otto-server --test route_inventory` | PASS | 2 passed; api.md/notification doc edits did not break it. |
| — | `cargo fmt --all --check` | FAIL (advisory) | 676 files drift — pre-existing tree-wide, explicitly advisory (matches ci.yml `continue-on-error: true`). NOT treated as blocking. |

### Clippy failures (gate 4) — BLOCKING

1. `crates/otto-usage/src/pricing.rs:24` — `error: constant PRICED_AS_OF is never used` (`-D dead-code`). D6 defined `PRICED_AS_OF` but never re-exported/consumed it (lib.rs only re-exports `estimate_cost`; the promised `priced_as_of` surfacing in the API was not wired).
2. `crates/otto-usage/src/pricing.rs:100` — `error: function is_priced is never used`. Same: defined + used only in `#[cfg(test)]`, never in non-test code or re-exported.
3. `crates/otto-improve/src/prompt.rs:24` — `error: doc list item without indentation` (`clippy::doc_lazy_continuation`). A `///` doc-comment formatting nit in S6's new code. Cosmetic, but it breaks `-D warnings`.

All three are trivial to fix (wire `is_priced`/`PRICED_AS_OF` into the usage API or `#[allow(dead_code)]`/remove; re-indent one doc line) but they MUST be fixed — clippy was green after batch 1 and is now red.

---

## Part B — Per-task adversarial audit

| Task | Verdict | Evidence |
|------|---------|----------|
| S4 — issues ownership | PARTIAL (pending) | otto-issues handlers fully guarded; cross-crate paths under review |
| D1 — worktree reuse | PASS | exists-guard correct, not inverted |
| D2 — task schedulable | PASS | default `todo`; planner unaffected |
| S6 — improve injection | PASS* (clippy nit) | escaping + memory gate + narrow allow-list verified; *prompt.rs:24 clippy error |
| D6 — usage cost | PASS (clippy nit) | cache priced, rates current, all 4 call sites updated; *dead `is_priced`/`PRICED_AS_OF` |
| S5 — login throttle | PASS | socket-peer key, no XFF, dual-key lockout, both serve paths wired |
| S7 — widget authz | PASS | `run_widget` now requires Editor (== run_query gate) |
| S2 — fs allow-list | PASS | component-boundary containment, fail-closed, deny-list kept |
| Orchestrator | PASS | unread own-only; api.md rows + ws note match code; AGENTS.md fmt advisory |

### S4 — otto-issues handlers (the in-scope part): PASS
`crates/otto-issues/src/http.rs`. `load_authorized_account` is the single
load+authorize chokepoint; `authorize_account` Forbids when
`account.user_id != user.0.id && !user.0.is_root` (correct, not inverted). Within
otto-issues, the ONLY `get_account` caller is `load_authorized_account` (http.rs:91),
so every read/use handler is guarded by construction. Confluence surfaces
(`list_spaces_cf`, `search_pages_cf`) are both guarded; no Confluence
get/create/update-page handlers exist in this crate. 3 ownership unit tests
present (owner=Ok, non-owner=Forbidden, root=Ok). (Confirmed by fork audit.)

### S4 — cross-crate exposure (out of agent-1 scope): UNDER REVIEW → reason for PARTIAL
Issue-account credentials are also resolved-and-used OUTSIDE otto-issues:
- `crates/otto-server/src/modules.rs:1434` — `ctx.issues_store.get_account(&account_id)`
  then uses `account.token_ref` to build a `JiraClient` in the PR-review flow.
- `crates/otto-product/src/service.rs` (~lines 266, 281, 337, 417, 516, 702,
  1025, 1331, 1417) — `self.issues.get_account(&req.account_id / &story.account_id)`.

These bypass the new `load_authorized_account` guard (which lives only in
otto-issues' router). Whether they are exploitable depends on whether the
caller's ownership is enforced upstream and whether `account_id` is
attacker-supplied. A dedicated trace is running; if any is REACHABLE-UNGUARDED,
S4 is genuinely PARTIAL and a real residual leak. (Result pending — see Gaps.)

### D1 — worktree reuse: PASS
`crates/otto-git/src/local.rs:227 worktree_add_if_absent` returns `Ok(false)`
(reuse, no reset) when `worktree_exists` is true, and only calls the destructive
`worktree_add` (`-B`/`--force`) when ABSENT — guard is correct, not inverted.
`ensure_cwd` (swarm_workspace.rs:109) calls the safe variant; `base` (HEAD) is
consulted only on first creation. A resumed turn lands in the same worktree with
its branch (and prior commits) intact. `worktree_exists` canonicalizes both sides
of the path comparison. Test at local.rs:1206 commits on turn 1, re-calls, asserts
created=false and the commit SHA/subject/file survive. (Minor: if `git worktree
list` momentarily errors, `worktree_exists` returns false → would recreate+reset;
edge failure case, not the normal path.)

### D2 — hand-added task schedulable: PASS
`SwarmService::create_task` (otto-swarm/src/service.rs:247) now defaults `status`
to `"todo"` via `req.status.unwrap_or_else(|| "todo".into())` — explicit statuses
(e.g. `"backlog"`) still honored. `SwarmRepo::ready_tasks` (otto-state/src/swarm.rs:882)
filters `status == "todo"`, so a UI-added task is now schedulable. The planner is
unaffected: its tasks are created via the STATE-layer `SwarmRepo::create_task`
directly from `swarm_runtime.rs` (lines 281/347/371/704) with explicit
`status: "todo"`, never through the service-layer default that changed. Two
state-layer tests confirm todo-picked / backlog-excluded and dependency gating.

### S6 — improve prompt-injection + memory poisoning: PASS (one clippy nit)
`crates/otto-improve/src/{prompt.rs,classify.rs,engine.rs}`.
(Confirmed by fork audit — see Gaps for the prompt.rs:24 clippy error which is the
only blemish.) Untrusted session/Jira/Confluence text is routed through
`fence_untrusted`/`escape_untrusted` at the interpolation sites (sentinel-fenced,
code-fence-neutralized, role-markers defanged); the Memory auto-apply path is
gated by a DETERMINISTIC `memory_content_gate` (8 KiB cap on `patch.after` +
injection/role-marker deny-list) that forces Queue regardless of autonomy and
regardless of model-reported risk; `run_for_narrative` intersects
`target_skills ∩ cfg.skill_allowlist` so an externally-triggered narrative can't
self-authorize edits to a non-configured-allow-listed skill.

### D6 — usage cost: PASS (dead-code clippy nit)
`estimate_cost(model, input, output, cache_read, cache_write)` prices all four
classes; cache-read 0.1× input, cache-write 1.25× input (pricing.rs:27-29,46-53).
Rates current and order-of-magnitude correct: Opus 5/25, Sonnet 3/15, Haiku 1/5,
Fable/Mythos 10/50; unknown models fall back to a non-zero Opus-tier (FALLBACK,
pricing.rs:59) — never $0. ALL call sites updated to the new 5-arg signature with
cache tokens: usage.rs:238, usage.rs:312, usage_tailer.rs:253, usage_tailer.rs:344
(grep found no old 3-arg signature remaining). The only defect is dead
`is_priced`/`PRICED_AS_OF` (clippy gate 4) — the `priced_as_of` date is not
surfaced anywhere despite the doc claim.

### S5 — login throttle: PASS
- Keyed on the real socket peer: `login` extracts `ConnectInfo<SocketAddr>` and
  passes `peer.ip()` (auth_routes.rs:51-56). `X-Forwarded-For`/`X-Real-IP` are
  NOT read anywhere for throttling (only mentioned in comments explaining why
  they're ignored).
- Per-username global lockout: `handle_login` tracks BOTH `ip_key` and
  `username_key` and `max_locked(&[ip_key, user_key])` gates on either
  (auth_routes.rs:69-95). IP rotation can't defeat it.
- ConnectInfo wired into BOTH serve calls in ottod/src/main.rs:
  `axum_server::bind_rustls(...).serve(router.into_make_service_with_connect_info::<SocketAddr>())`
  (TLS, main.rs:565) AND `axum::serve(loopback,
  router.into_make_service_with_connect_info::<SocketAddr>())` (loopback,
  main.rs:588). Build/test pass → the trait bounds are satisfied (no runtime
  ConnectInfo panic).
- `login_throttle` registered (`pub mod login_throttle;` lib.rs:11). dev-deps:
  otto-rbac += `tokio` (workspace), otto-issues += `chrono` (workspace) — correct.
- `ip_rotation_does_not_defeat_username_lockout` genuinely proves the property:
  rotates IP for THRESHOLD attempts, asserts no per-client key locks, asserts the
  username key DOES lock, and asserts a fresh IP is then blocked outright. The test
  helpers (`record_failed_attempt`, `is_blocked`) faithfully mirror the handler's
  dual-key bookkeeping.

### S7 — widget authz: PASS
`run_widget` (otto-dbviewer/src/http.rs:469) now resolves the widget's connection
and calls `check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor)` (line 489)
— byte-for-byte the same gate as `run_query` (line 256). A workspace Viewer is
rejected; test `viewer_is_rejected_from_widget_execution_gate` asserts the denial
AND that the gate demanded Editor. Also correctly handles global (root-only)
connections by gating on the connection, not just the widget's workspace.

### S2 — fs allow-list: PASS
`crates/otto-server/src/routes/fs.rs`. `guard_dir`/`guard_file` apply the
allow-list FIRST (fail-closed), then the deny-list. `is_within_allowed` uses
`canonical == root || canonical.starts_with(root)` on canonicalized paths —
Rust `Path::starts_with` is COMPONENT-based, so a prefix-sibling like
`/Users/me-evil` does NOT match root `/Users/me` (explicitly tested,
fs.rs:628-637). Roots: `$HOME` + `data_dir` + `OTTO_LOG_DIR`/`OTTO_SKILLEVAL_DIR`
+ `OTTO_FS_EXTRA_ROOTS`, each canonicalized; empty set → everything denied
(fail-closed, tested). `CurrentUser` required on both handlers. Out-of-root paths
(`/var/root`, another user's home, `/opt/secret`) denied even when not in the
deny-list; secret stores nested in an allowed root (`~/.ssh`) still deny-listed.

### Orchestrator — notifications + docs: PASS
- `unread_count` (otto-state/src/notifications.rs:239): non-root
  (`NoticeAccess::User`) counts `read = 0 AND user_id = ?` — own unread only;
  root (`All`) counts everything. No stuck global badge.
- api.md notification rows (525-531) + scoping note (533) accurately describe
  global+own read / own-only mutate / root-all, and that the badge counts own
  unread only — matches the code.
- `/ws/events` row (api.md:604) documents `Sec-WebSocket-Protocol: otto-bearer,
  <token>` (preferred) with `?token=` fallback — matches ws_events.rs:3-6,46-49.
- AGENTS.md:75 fmt line relabeled advisory; matches ci.yml `continue-on-error: true`.

---

## Part C — Regression hunt

- **No new test failures.** Full `cargo test --workspace` green incl. doctests.
- **No new problematic panics/unwraps.** Only new non-test unwraps:
  `login_throttle.rs` Mutex `lock().unwrap()` ×3 (idiomatic, poisoning-only) and
  `otto-issues/http.rs:480` `Response::builder().body(empty()).unwrap()` (infallible
  500-fallback, pre-existing pattern). No `todo!`/`unimplemented!`.
- **ConnectInfo both serve paths confirmed** (main.rs:565 & 588) — the critical
  S5 runtime regression (a missed serve path → login 500) is NOT present.
- **No agent clobbering.** lib.rs (+90) carries S5's `pub mod login_throttle;`
  PLUS swarm-module registrations PLUS a CORS hardening block — all ADDITIVE and
  compiling; not a conflict. (The CORS change is broader uncommitted work, not in
  the batch-2 task list; benign, restricts `CorsLayer::permissive` to a
  loopback/LAN/Tailscale/Tauri allow-list.)
- **dev-deps correct** and resolve (otto-rbac tokio, otto-issues chrono,
  otto-server otto-swarm).

---

## Gaps & regressions (by severity)

1. **[BLOCKING] clippy is RED** (gate 4, exit 101) — 3 batch-2-introduced errors:
   `is_priced`/`PRICED_AS_OF` dead (pricing.rs:24,100) and `doc_lazy_continuation`
   (prompt.rs:24). Batch 1 made clippy green; batch 2 regressed it. Must fix
   before merge.
2. **[HIGH — PARTIAL] S4 cross-crate** — the ownership guard lives only in
   otto-issues' router; `otto-server/modules.rs:1434` and `otto-product/service.rs`
   resolve+use issue-account Atlassian tokens by `account_id` without that guard.
   If any is reachable with an attacker-supplied `account_id` they don't own, the
   credential-leak finding S4 is NOT fully closed. (Trace in progress — final
   verdict appended below.)
3. **[LOW] D6 `priced_as_of` not surfaced** — the date constant exists but is
   exposed nowhere in the usage API, contrary to the task/report. Fixing gap 1's
   dead-code by actually wiring it would also close this.
4. **[INFO] fmt drift** (676 files) — advisory only; a one-time repo-wide
   `cargo fmt --all` is still owed before fmt can become blocking.
