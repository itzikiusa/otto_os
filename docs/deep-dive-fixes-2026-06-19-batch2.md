# Deep-Dive Fixes — Batch 2 (2026-06-19)

Driven by user code-review of batch 1. Closes the remaining **§0 app-wide security-critical / data-loss** items (S4, D1, D2, S6, S7, D6) plus tightening of the two batch-1 items the user verified as **partial** (S2, S5) and the doc/behavior drift.

Priority order (user-ranked): **S4 → D1 → D2 → S6 → S7 → D6 → tighten S2/S5 + docs**. Run as parallel Opus agents (disjoint files) → verifier → orchestrator final pass. Each agent reports to `docs/fixes-2026-06-19/batch2-agent-<N>-*.md`.

## ✅ FINAL STATUS — all green (orchestrator hands-on)
| Gate | Result |
|------|--------|
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ exit 0 |
| `cargo test --workspace` | ✅ exit 0 (48 suites, 0 failures) |
| `cd ui && npm run check` | ✅ 480 files, 0/0 |
| `cargo fmt --check` | ⚠️ advisory (pre-existing tree-wide) |

Per task: **S4 PASS** (was PARTIAL — verifier found unguarded cross-crate paths in modules.rs/otto-product/otto-git; a dedicated closure agent added one `authorize_owner` chokepoint in otto-core and guarded every credential-use path owner-or-root, with tests). **D1, D2, S6, D6, S5, S7, S2, orchestrator docs/notifications: PASS.**

Verifier-found blockers, all fixed by orchestrator: clippy red on 3 batch-2 lints (`pricing.rs` dead `PRICED_AS_OF`/`is_priced` → re-exported as public API **and** surfaced `priced_as_of` in `/usage/status`, closing the D6 LOW gap; `prompt.rs:24` doc lint → blank doc line; and the S4 `user`-param pushed `run_pr_review_inner` to 8 args → `#[allow(too_many_arguments)]`).

## Agent 1 — S4: Cross-user Jira/Confluence credential leak (TOP PRIORITY)
Owns: `crates/otto-issues/src/http.rs` (+ service if needed).
- [ ] Apply the `account.user_id != user.id && !is_root` ownership guard (already present on create/update/delete) to **every read/use handler**: get_issue, search_issues, list_projects, get_issue_full, get_comments, do_transition, assign_issue, add_comment, attachment proxy, create/update issue, Confluence get/search/page ops, etc. No authed user may act with another user's Atlassian identity. TLS network listener is live → this is the last critical security hole.

## Agent 2 — D1 + D2: Swarm actually works across turns
Owns: `crates/otto-server/src/swarm_workspace.rs`, `crates/otto-swarm/src/service.rs`, `crates/otto-state/src/swarm.rs`, `crates/otto-git/src/local.rs`.
- [x] **D1**: `ensure_cwd` calls `worktree_add` (`git worktree add --force -B <branch> <base>`) every turn → resets branch to base HEAD, discarding the agent's commits. Reuse the existing worktree when its dir exists; only `-B`/create on first creation.
- [x] **D2**: `create_task` defaults `status:"backlog"`; `ready_tasks` only selects `"todo"`; nothing promotes backlog→todo, so hand-added tasks never run. Default schedulable tasks to `todo` (or add the transition), preserving any explicit planner status.

## Agent 3 — S6: Self-improvement prompt-injection → memory poisoning (highest-risk logic)
Owns: `crates/otto-improve/src/{prompt.rs,classify.rs,engine.rs}`.
- [ ] Escape/sanitize untrusted session/Jira/Confluence text before interpolating into the improve prompt (`serde_json::to_string` or hard sentinels + an explicit "untrusted" guard).
- [ ] Stop trusting LLM self-reported risk for Memory auto-apply: gate Memory edits behind a deterministic check (size cap + reject injection/role markers) or route through the same allow-list/approval as skills, so `MEMORY.md` can't be poisoned. Also close `run_for_narrative` supplying its own allow-list.

## Agent 4 — D6: Usage cost is materially wrong
Owns: `crates/otto-usage/src/pricing.rs`, `crates/otto-server/src/routes/usage.rs`, `crates/ottod/src/usage_tailer.rs`.
- [ ] Price cache tokens (read ≈0.1×, write ≈1.25×) — change `estimate_cost` to take cache read/write + per-model cache rates; update all call sites.
- [ ] Refresh pricing to current (Opus $5/$25, Haiku $1/$5, Sonnet $3/$15, add Fable 5/Mythos 5 $10/$50) + a non-zero fallback for unknown models + a `priced_as_of` date.

## Agent 5 — S5 tighten + auth/token tests
Owns: `crates/otto-server/src/routes/auth_routes.rs`, `crates/ottod/src/main.rs`, `crates/otto-rbac/src/tokens.rs` (test additions only), new `crates/otto-server/tests/auth_security.rs`.
- [x] **S5 bypass**: `client_ip` trusts `X-Forwarded-For`/`X-Real-IP` with no trusted proxy in front → attacker rotates XFF for a fresh key, never hits lockout. Use the real socket peer (`ConnectInfo<SocketAddr>`; wire `into_make_service_with_connect_info::<SocketAddr>()` on **both** the loopback `axum::serve` and the `axum_server::bind_rustls` serve in main.rs); ignore XFF unless a trusted-proxy setting exists; **also throttle on username alone** so header rotation can't help.
- [ ] Tests: token lifecycle (mint/list/revoke + auth + revoke-on-password-change/disable/delete) and login lockout behavior.

## Agent 6 — S7 + S2 tighten (authz)
Owns: `crates/otto-dbviewer/src/http.rs`, `crates/otto-server/src/routes/fs.rs`.
- [x] **S7**: `run_widget` requires only `Viewer` but executes the stored statement through the same path that requires `Editor` for `run_query`. Require `Editor` (or enforce read-only on widget execution).
- [x] **S2 tighten**: the deny-list sandbox is bypassable. Add an allow-list of permitted roots (user HOME subtree + configured workspace/project/extra dirs) and reject canonicalized paths outside them, keeping the existing dotfile/secret deny-list as defense-in-depth, without breaking the legitimate folder picker.

## Orchestrator (me) — docs + notification behavior (user notes 1, 3, 4) — ✅ DONE
- [x] `docs/contracts/api.md`: corrected the `/notifications` rows to actual per-user scoping (global+own read, own-only mutate) + a scoping note; updated the `/ws/events` row to document `Sec-WebSocket-Protocol` (preferred) with `?token=` fallback so event-WS auth isn't shown as `?token=`-only.
- [x] Notification unread for non-root: `unread_count` (otto-state/notifications.rs) now counts the member's OWN unread only — global notices show in the list but no longer create a badge they can't clear (root still counts all).
- [x] `AGENTS.md`: `cargo fmt --all --check` relabeled advisory (matches `ci.yml` `continue-on-error`), with a note to land a repo-wide format pass before promoting to blocking.

---
## Batch-1 status correction
S2 and S5 were marked done in batch 1 but the user verified them **partial** (S2 deny-list bypassable; S5 XFF-rotation lockout bypass). Re-opened above. S1/S3/S9 confirmed pass.
