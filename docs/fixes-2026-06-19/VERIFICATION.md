# Independent Verification — Deep-Dive Must-Have Fixes (2026-06-19)

Adversarial verification of the 6-agent hardening pass. No source files were
modified during this verification; only build/test/lint commands were run
(they write to `target/` only). Evidence is by direct code reading +
re-running every gate from a clean tree.

**Overall: RED** — one real, reproducible defect blocks the test + clippy gates.

- `cargo test --workspace` **FAILS**: a single test, `net_guard_blocks_internal_addresses`,
  fails because the SSRF guard does **not** block IPv6 loopback `::1`. This is a
  real security bypass (not just a test-tightness issue) and a real gate failure.
- `cargo clippy --workspace --all-targets -- -D warnings` **FAILS** on a
  **pre-existing** `clippy::derivable_impls` lint in `otto-core/src/domain.rs`
  (not introduced by this pass) — but it still means the CI gate Agent 5 added
  does not pass on the current tree.

Everything else verified PASS. Both blockers are small, surgical fixes.

---

## Part A — Build / Test Gate Table

| # | Command | Result | Notes |
|---|---------|--------|-------|
| 1 | `cargo check --workspace` | **PASS** | Clean; the cross-file `user_id` ripple is fully wired. |
| 2 | `cargo build --workspace` | **PASS** | Finished, no errors. |
| 3 | `cargo test --workspace` | **FAIL** | Exactly 1 failing test: `routes::api_client::tests::net_guard_blocks_internal_addresses` (`crates/otto-server/src/routes/api_client.rs:1625`, panic `::1 should be blocked`). All other test targets pass. |
| 3a | `cargo test -p otto-rbac --doc` | **PASS** | `ok. 0 passed; 0 failed` (compiles; no doctests). T8 satisfied. |
| 3b | Workspace doctest phase | **PASS** | Every crate's `Doc-tests …` = ok, 0 failed (otto_core, otto_rbac, otto_server, … all green). |
| 4 | `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL** | 1 error: `clippy::derivable_impls` at `crates/otto-core/src/domain.rs:1172` (`impl Default for Autonomy`). **Pre-existing** (commit `faef6a0`, 2026-06-14; `domain.rs` not modified this pass), but the gate Agent 5 added does not pass. |
| 5 | `cargo fmt --check` | **PASS** | No unformatted files at all (incl. pre-existing swarm files). |
| 6 | `cd ui && npm run check` | **PASS** | `COMPLETED 480 FILES 0 ERRORS 0 WARNINGS`. |
| 7 | `cargo test -p otto-server --test route_inventory` | **PASS** | `2 passed; 0 failed` (`every_registered_route_is_documented`, `documented_paths_are_well_formed`). |

---

## Part B — Per-Task Correctness Audit

### Agent 1 — Outbound & file security

| Task | Verdict | Evidence |
|------|---------|----------|
| T1 SSRF — `is_blocked_ip` coverage | **PARTIAL (one real bypass)** | `api_client.rs:67-100`. Correctly blocks 127.0.0.1, 10/172.16/192.168, 169.254.169.254, 169.254/16, 100.64/10 CGNAT, 0.0.0.0, broadcast/multicast/doc, and v4-mapped `::ffff:127.0.0.1` (unwrapped via `to_ipv4_mapped`). **BUG:** for IPv6 it does `to_ipv4_mapped().or_else(\|\| v6.to_ipv4())` (line 70). The deprecated `to_ipv4()` maps `::1` → `0.0.0.1` (verified empirically), which is neither loopback nor private, so `is_blocked_ip(::1)` returns **false** → IPv6 loopback is NOT blocked. The author's own unit test catches this and fails. Fix: drop the `.or_else(\|\| v6.to_ipv4())` fallback (the V6 arm already blocks `::1` via `is_loopback()`). |
| T1 SSRF — guard called on every outbound path | **PASS** | `build_and_send` (execute + automation) pre-flights `check_url` right after var-substitution (`api_client.rs:881-883`); `oauth2_token` (`api_client.rs:668`); SSE `serve_sse` (`api_stream.rs:154` + `redirect_policy` 161); WS `serve_websocket` (`api_stream.rs:307`, before `connect_async`); gRPC `invoke` (`grpc.rs:296`) and reflection path via `connect_channel` (`grpc.rs:511`, before every dial); `browser_proxy` (`modules.rs:2423` + `redirect_policy` 2362). Redirect policy re-validates each hop & fails closed (`api_client.rs:187-198, 156-182`). No outbound user-URL fetch bypasses the guard. |
| T2 fs sandbox | **PASS** | `fs.rs`. `browse`/`read_file` bind `CurrentUser(user)` and `canonicalize()` **before** `guard_dir`/`guard_file` (lines 257-263, 357-363), so `..`/symlink escapes resolve first; a symlink into `~/.ssh` is caught by its target. Deny-list covers `~/.ssh /.aws /.gnupg /.kube /.docker /.config/{gcloud,gh} /.azure /.password-store` and `/etc /private/etc /root /var/root /proc /sys`, plus secret filenames/exts (`id_rsa`, `credentials`, `.env`, `.pem`, …). Deny-list (not allow-list) → legitimate browsing preserved. `User` is consulted (passed to guards); root vs non-root intentionally does not relax the deny-list (documented). |

### Agent 2 — Auth hardening + doctest

| Task | Verdict | Evidence |
|------|---------|----------|
| T4a login rate-limit | **PASS** | `auth_routes.rs`. Real per-`(client-ip\|username)` counter: 5 failures / 15-min window → 15-min lockout → 429 + `Retry-After` (`check_locked` 88, `record_failure` 107, `clear_failures` on success 124, `prune_expired` 129). Wired into the live `login` handler (`164-185`): locked check before auth, `clear_failures` on success, threshold-crossing attempt itself answered with 429. Bounded map (`MAX_TRACKED_KEYS=10_000`). Not dead code. |
| T4b revoke-on-change | **PASS** | `otto-rbac/tokens.rs:144` `revoke_all_for_user` = `DELETE FROM auth_sessions WHERE user_id = ?` (kills session + api/PAT rows). Called from `users::update` when password changed OR `disabled==Some(true)` (`users.rs:78-81`) AND from `users::remove` (soft-delete=disable, `users.rs:101-102`). Both call sites present. |
| T4c password policy | **PASS** | `otto-rbac/passwords.rs:11,15` `MIN_PASSWORD_LEN=10` + `validate_password`; re-exported (`lib.rs:15`). Called in `users::create` (`users.rs:36`) AND `users::update` on password change (`users.rs:61`). Matches onboarding's rule (`onboarding.rs:14` also `=10`). Minor: onboarding keeps a local constant rather than calling the shared helper (values match; non-blocking). |
| T8 doctest root cause | **PASS** | `ApiTokenInfo` is `pub` at `otto-core/src/api.rs:74` and imported normally `use otto_core::api::ApiTokenInfo;` at `tokens.rs:8` — **not** inside a doctest, **not** marked `ignore`/`no_run`. Real fix (proper `pub` export), not hidden. `otto-rbac --doc` + workspace doctests pass. |

### Agent 3 — WS & multi-user edge

| Task | Verdict | Evidence |
|------|---------|----------|
| T5 notifications per-user | **PASS** | Migration `0031_notifications_user.sql` adds `user_id TEXT`, indexes it, replaces the global unique index with per-`(user_id, source_key)` unique. Repo (`notifications.rs`) scopes by `NoticeAccess`: `User(id)` reads `WHERE user_id IS NULL OR user_id = ?` (204-229), mutates `WHERE … AND user_id = ?` (mark_read 254, mark_all_read 276, dismiss 296, clear 319) — a non-root user cannot read/clear another user's notices, nor mutate global notices. Dedupe is NULL-safe `user_id IS ?` (`create` line 141) — real. REST handlers enforce `CurrentUser` + `access_for` on every list/mutate (`routes/notifications.rs`). `ws_events.rs:138-143` delivers owned notices only to that user (root sees all); global (`None`) → everyone. |
| T3b WS bearer out of `?token=` | **PASS** | `ws_events.rs:40-82`: `/ws/events` reads token from `Sec-WebSocket-Protocol` (`otto-bearer, <token>`), validates BEFORE upgrade, echoes `otto-bearer` only when subprotocol used; legacy `?token=` kept as fallback. `client.ts:134-142` `wsConnect` opens `new WebSocket(url, [WS_BEARER_SUBPROTOCOL, token])` (no `?token=`). `events.svelte.ts:39` uses `wsConnect('/ws/events')`. Shared `wsUrl()` (terminal, `client.ts:112`) left **untouched** with its `?token=` — terminal not broken. |
| T-CORS | **PASS** | `lib.rs:95-158`. Allowlist via `AllowOrigin::predicate` (not `permissive()`): Tauri scheme + loopback (any port) + `*.localhost` + Tailscale `*.ts.net` + RFC-1918 LAN. Methods pinned; headers = Authorization+Content-Type; `allow_credentials` stays off (bearer, not cookie) → no credentialed-wildcard footgun. Arbitrary public origins rejected; malformed origin fails closed. |

### Agent 4 — ottod runtime hardening

| Task | Verdict | Evidence |
|------|---------|----------|
| T3a TLS on 0.0.0.0 | **PASS** | `ottod/main.rs:537-572`: when `network_listener` enabled → `axum_server::bind_rustls(addr, tls).serve(...)` (560) — never plain HTTP. `load_or_make_tls_config` (633) loads/auto-generates self-signed cert; installs ring provider (642). On TLS failure → `tracing::error!` and the task is simply not spawned (569-571) — **no silent HTTP fallback**. Loopback listener unchanged (`axum::serve(loopback, …)` 580). Graceful shutdown bridged (`Handle::graceful_shutdown` from `shutdown_rx`, 552-558); `network_task.await` drains before PTY kill (585). `ottod/Cargo.toml:41-45` has axum-server (`tls-rustls-no-provider`), rustls+ring, rustls-pemfile, rcgen, sha2 — all resolve (build green). |
| T7 workflow reaper + timeout | **PASS** | `workflow_engine.rs:40-52` `reap_orphaned_runs` = `UPDATE workflow_runs SET status='error' … WHERE status IN ('pending','running')`. **CALLED at startup** at `ottod/main.rs:274`, right after the review/skill-eval recovery, with logging. Global per-run timeout `RUN_WALL_CLOCK_TIMEOUT=30min` checked inside the run loop (`workflow_engine.rs:168`, breaks → marks error at 245). Wired, not dead. |
| T6 Tauri CSP | **PASS** | `tauri.conf.json:26`: `csp` is a real string (no longer null). `connect-src` includes `http://127.0.0.1:7700 ws://127.0.0.1:7700 https/wss` + `ipc: http://ipc.localhost` (Tauri 2 IPC) + `localhost:7700`; `script-src 'self'`, `'self'` assets; `object-src 'none'`. Won't break the app (covers daemon http+ws, IPC, blob/data, browser-proxy via `frame-src http(s):`). |

### Agent 5 — CI / release / docs / Svelte

| Task | Verdict | Evidence |
|------|---------|----------|
| T9a CI pipeline | **PARTIAL** | `.github/workflows/ci.yml` (91 lines) exists; references real commands — `ui/package.json` has `check` (`svelte-check + tsc`) and `build` (`vite build`); `ui/package-lock.json` is git-tracked (so `npm ci` works). YAML jobs coherent. **But** the `cargo clippy … -D warnings` step it runs **fails today** on the pre-existing `derivable_impls` lint (BLOCKER 2) — the gate is red on first run. |
| T9b RELEASE.md | **PASS** | `docs/RELEASE.md` (125 lines) exists; no Makefile in repo (release correctly uses `npx … @tauri-apps/cli`); referenced `packaging/{make-cert,sign,dmg}.sh` all exist. Gates on cargo + `npm run check` + build/sign/dmg. |
| T10 AGENTS.md + CLAUDE.md | **PASS** | `AGENTS.md` (130 lines) lists **all 20** crates, real build/test commands, notes the desktop app is a separate workspace, and has a "Do NOT damage user work" section (line 110, incl. "ask before irreversible/outward-facing actions"). `CLAUDE.md` (7 lines) is a thin bridge with the `@AGENTS.md` import (line 7). |
| T12 Svelte warnings | **PASS** | Real reactivity fixes, not suppression (no `svelte-ignore`/`eslint-disable`): `RedisKeyFilter.svelte` → `let draft=$state('')` + `$derived(node.id filter)` + `$effect` re-sync (14-17); `TableDesigner.svelte` → extracted `rowsFromColumns()` + `rows=$state([])` + `$effect` keyed on `table` with `seededFor` guard (26,40-43); `ResultsGrid.svelte` → `tb-note` selector removed (grep count 0). `npm run check` = 0/0. |

### Agent 6 — API/WS contracts + route-inventory test

| Task | Verdict | Evidence |
|------|---------|----------|
| T11a api.md routes | **PASS** | Spot-checked previously-undocumented routes all present: `/fs/browse` (api.md:570), `/fs/read` (571), `/notifications` (525), `/workflows/*` (469), `/connections/{id}/db/*` (218), `/auth/tokens` (#87, 152), `/insights/*` (547), `/workspaces/{id}/skill-evaluations` (416). Route-inventory test independently confirms 0 undocumented of 261 registered. |
| T11b ws.md Event variants | **PASS** | All **16** `Event` variants in `event.rs` are documented in ws.md (each snake_case `type` tag — `session_status` … `swarm_status` — appears ≥2×). Code has exactly 16 variants; all covered. |
| T11c route-inventory test | **PASS** | `tests/route_inventory.rs`: walks `crates/**/*.rs`, extracts `.route("…")` literals (whitespace/newline-tolerant scanner), asserts each appears verbatim in api.md, lists undocumented on failure, has a **sanity floor** (`routes.len() >= 100`) so a broken extractor fails loudly, and a second test checks paths are absolute + brace-balanced. Passes 2/2. Logic sound. |

---

## Part C — Gaps & Regressions (ordered by severity)

### BLOCKER 1 — SSRF guard does not block IPv6 loopback `::1`
- **Where:** `crates/otto-server/src/routes/api_client.rs:70` — `v6.to_ipv4_mapped().or_else(\|\| v6.to_ipv4())`.
- **Why it's a real bug:** the deprecated `Ipv6Addr::to_ipv4()` maps the entire `::/96` block (including `::1`) to an IPv4 address. `::1` becomes `0.0.0.1`, which passes every v4 block check → `is_blocked_ip(::1) == false`. An attacker can reach the daemon's own services over IPv6 loopback via any guarded outbound path (api-client execute, gRPC, SSE/WS stream, browser-proxy) by targeting `http://[::1]:<port>/`.
- **Caught by:** the author's own unit test `net_guard_blocks_internal_addresses` (assert `::1 should be blocked`) — which **fails**. This is also the sole `cargo test --workspace` failure.
- **Fix (1 line):** remove the `.or_else(\|\| v6.to_ipv4())` fallback. The V6 match arm already blocks `::1` via `is_loopback()`. `to_ipv4_mapped()` alone is the correct unwrap for `::ffff:` v4-mapped addresses (its test case still passes).
- Note: Agent 1's report claimed the tests "should build cleanly" once the ripple landed, but the agent never actually ran them (the crate didn't compile at the time). The test was authored correctly and now exposes the bug.

### BLOCKER 2 — `clippy -D warnings` (the new CI gate) fails on pre-existing lint
- **Where:** `crates/otto-core/src/domain.rs:1172` — `impl Default for Autonomy` triggers `clippy::derivable_impls`.
- **Pre-existing:** introduced in `faef6a0` (2026-06-14); `domain.rs` is not modified in this working tree. **Not** introduced by this pass.
- **Impact:** Agent 5 added `cargo clippy --workspace --all-targets -- -D warnings` to CI without first confirming the repo is clippy-clean, so the very first CI run would fail. Either fix the lint (derive `Default` + `#[default]` on `Autonomy::Tiered`, as clippy suggests) or the gate is non-functional.
- **Severity:** the fix is trivial and the warning is not a correctness issue, but the CI Agent 5 shipped is currently a red gate.

### Non-blocking follow-ups (from the task brief)
- `ui/src/lib/api/types.ts` notification `user_id`: **DONE** (`types.ts:205` — `{ type: 'notification'; notice: Notice; user_id?: string \| null }`).
- `docs/contracts/ws.md` mention of `Sec-WebSocket-Protocol`: **OPEN** — ws.md does not mention the subprotocol token path (grep found no `Sec-WebSocket-Protocol`/`otto-bearer`). Cosmetic doc gap; the code is correct.

### Regression hunt — clean
- Cross-file `user_id: None` ripple: all 6 `NewNotice {…}` sites carry `user_id` (monitor.rs ×4 lines 239/295/575/702, activity.rs:362, skill_eval.rs:930) and the single `Event::Notification` construction forwards it (`state.rs:163-165`). `cargo check --workspace` clean confirms no missing fields.
- No new `unimplemented!`/`todo!`/`panic!` in changed security files. The `.unwrap()`s are standard `Mutex::lock().unwrap()` (auth_routes) and pre-existing graph invariants (workflow_engine) — not introduced regressions.
- Security-guard logic uses correct boolean composition: `is_blocked_ip` blocks on `\|\|` (any-match), fs `guard_file` denies on `is_denied_dir \|\| is_denied_file`, redirect policy fails closed. No `&&`/`\|\|` inversion, no early-return that bypasses a check.
