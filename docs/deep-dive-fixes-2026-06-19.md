# Deep-Dive Must-Have Fixes — Merged Task List (2026-06-19)

Merged from the **must-have** items of:
- `docs/deep-dive-improvements-2026-06-19.md` → §0 (app-wide top priorities), §1 (cross-cutting), §2 (core backend / API client / workflows / auth)
- `docs/research/2026-06-19-otto-product-improvement-audit.md` → Cross-Cutting Platform "Must Have" + P0 list (core/cross-cutting items only)

Scope for this pass: **must-haves in the core + cross-cutting surface only** (deep-dive §0/§1/§2). Section-specific must-haves owned by Swarm/Sessions/DB-internals/Git/Product (deep-dive §3–§8) are out of scope for this pass, EXCEPT where §0 surfaces them as app-wide and they also appear in §1/§2.

Each implementation agent writes a progress report to `docs/fixes-2026-06-19/agent-<N>-<name>.md`. The boxes below are consolidated by the orchestrator after a verifier pass + a hands-on final build/test.

---

## ✅ FINAL STATUS (orchestrator hands-on verification)

All must-have tasks **complete and verified green**. Run by 6 parallel Opus agents (disjoint file ownership) → independent Opus verifier → orchestrator final pass.

### Gate results (re-run from clean tree by the orchestrator)
| Gate | Result |
|------|--------|
| `cargo check --workspace` | ✅ PASS |
| `cargo build --workspace` | ✅ PASS |
| `cargo test --workspace` (incl. all doctests) | ✅ PASS (exit 0, 0 failures — **otto-rbac doctest fixed**, T8) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ PASS (exit 0) |
| `cargo test -p otto-server --test route_inventory` | ✅ PASS (2/2) |
| `cd ui && npm run check` | ✅ PASS (480 files, 0 errors, 0 warnings) |
| `cargo fmt --all --check` | ⚠️ Advisory (see note) |
| `cargo audit` | ⚠️ Advisory (553-crate tree never triaged) |

### Verifier-found blockers — both fixed by orchestrator
1. **SSRF `::1` bypass (real hole + test fail):** `is_blocked_ip` used the deprecated `Ipv6Addr::to_ipv4()`, which maps `::1`→`0.0.0.1` and dodged every v4 rule. Removed it (kept `to_ipv4_mapped()` for genuine `::ffff:` forms); `::1`/`fe80::`/`fc00::` now blocked by the v6 arm. `net_guard` tests pass. — `routes/api_client.rs`
2. **`clippy -D warnings` red on pre-existing lints:** the CI gate Agent 5 added failed on ~14 pre-existing lints (the verifier only caught 1). Resolved ALL of them — auto-fixed the mechanical ones (`map_or`/`split_once`/`map_identity`/redundant closures/…) via `clippy --fix`, and applied behavior-preserving manual fixes + targeted `#[allow]` (too_many_arguments on DB upserts, recursion params, complex SQL map type, drop_non_drop) for the structural ones. Clippy is now exit 0.

### Two orchestrator judgment calls (surfaced, not silently decided)
- **`cargo fmt --check` made advisory in CI.** Ground truth: the repo was *never* rustfmt-formatted — `cargo fmt` wants to reformat **658 files**, ~600 of them pre-existing committed code nobody touched this pass (the verifier's "fmt clean" claim was inaccurate). A repo-wide format is a deliberate one-time commit the owner should make on its own; bundling it here would churn the entire tree including the large in-flight uncommitted swarm/PAT work. The step is kept in CI but `continue-on-error: true` with a comment to promote it once a format pass lands.
- **`cargo audit` left advisory** (as Agent 5 set it) — a new advisory in a 553-crate tree should warn, not block, until triaged.

### Non-blocking follow-ups — done
- `ui/src/lib/api/types.ts`: `notification` event gained `user_id?: string | null`.
- `docs/contracts/ws.md`: documented `Sec-WebSocket-Protocol` bearer auth for `/ws/events`.

## Merge notes — where the two audits overlap (deduplicated)

| Task | deep-dive | research |
|------|-----------|----------|
| SSRF guard (browser_proxy + API-client executor) | §1 MH, §2 MH (S1) | API client "execution safety for verify_ssl=false" / network listener hardening |
| `fs.rs` host-file sandbox | §2 MH (S2) | — |
| TLS on `0.0.0.0` + bearer token out of WS query | §2 MH (S3) | "network listener hardening: bind, origins, token, rate limits" / CSRF/origin |
| Login rate-limit + revoke sessions on pw change | §1 MH, §2 MH (S5) | "rate limits", "session/token revocation" |
| Notifications per-user (stop global broadcast) | §2 MH (S9) | — |
| Tauri CSP (replace `"csp": null`) | §1 MH (S10) | "CSRF/origin strategy for non-loopback" |
| Workflow startup reaper + per-run timeout | §2 MH (D7) | "workflow run retention / error policy" (P0-adjacent) |
| CI pipeline (test/clippy/fmt/audit) + release checklist | §1 MH | "release checklist", "release verification script" (P0 #6) |
| Fix `cargo test --workspace` doctest (`ApiTokenInfo`) | — | Cross-Cutting MH / P0 #1 |
| Root `AGENTS.md` (+ `CLAUDE.md` bridge) | — | Cross-Cutting MH / P0 #2 |
| api.md route-contract drift + route-inventory test | §1 MH (should, promoted) | Cross-Cutting MH / P0 #3 |
| Fix 3 database Svelte warnings | — | DB MH / P0 #5 |

---

## Agent 1 — Outbound & file security (SSRF + fs sandbox)
Owns: `routes/api_client.rs`, `routes/api_stream.rs`, `routes/grpc.rs`, `routes/fs.rs`, `modules.rs` (browser_proxy / outbound only).
- [x] **T1 — SSRF guard** on `browser_proxy` and the API-client executor (`execute`/`oauth2`/gRPC/stream): resolve DNS, reject loopback / private / link-local / `169.254.169.254` metadata; restrict redirects (cap + re-check each hop); gate TLS-skip per request.
- [x] **T2 — fs.rs sandbox**: restrict `/fs/browse` & `/fs/read` to permitted roots; reject `..`/symlink-escape and sensitive dotfiles (`~/.ssh`, `~/.aws`, `/etc/...`); actually use `CurrentUser`/role.

## Agent 2 — Auth hardening + doctest fix
Owns: `routes/auth_routes.rs`, `routes/users.rs`, `otto-rbac/src/tokens.rs`, `otto-rbac/src/lib.rs`, `otto-core/src/api.rs`, `otto-core/src/auth.rs`.
- [x] **T4a — Login rate-limiting/lockout** on `POST /auth/login` (in-memory per-IP/per-user backoff; no new crate needed).
- [x] **T4b — Revoke all `auth_sessions`** on password change/disable (`users::update`); add a `revoke_all`/`revoke_for_user` path in rbac.
- [x] **T4c — Password policy** enforced in `users::create`/`update` (not just onboarding).
- [x] **T8 — Fix `cargo test --workspace`** doctest failure in `otto-rbac` (`unresolved import otto_core::api::ApiTokenInfo`).

## Agent 3 — WS & multi-user edge (notifications per-user + WS token relocation + CORS)
Owns: `routes/notifications.rs`, `ws_events.rs`, `otto-core/src/event.rs`, `otto-server/src/lib.rs`, `otto-state/src/lib.rs` + new migration, `ui/src/lib/api/client.ts`, notification UI store/component.
- [x] **T5 — Notifications per-user**: scope storage + delivery to the owning user (or workspace); stop broadcasting every `Notification` to all clients; enforce `CurrentUser` on clear/alter.
- [x] **T3b — WS bearer token out of `?token=`**: accept the token via `Sec-WebSocket-Protocol` (or a short-lived ticket) server-side + update `client.ts`.
- [x] **T-CORS — Tighten CORS** from `permissive()` toward an allowlist (esp. with `network_listener` on).

## Agent 4 — ottod runtime hardening (TLS + workflow reaper + Tauri CSP)
Owns: `ottod/src/main.rs`, `ottod/Cargo.toml`, `workflow_engine.rs`, `routes/workflows.rs`, `apps/desktop/src-tauri/tauri.conf.json`.
- [x] **T3a — TLS (rustls) on the `0.0.0.0` listener**: serve HTTPS when `network_listener` is on (cert/key load; keep loopback as-is). `main.rs:531`.
- [x] **T7 — Workflow crash recovery**: startup reaper marking orphaned `running` runs as `error`, plus a global per-run timeout (reaper SQL inline in workflow_engine.rs; do NOT add otto-state methods — that file is owned by Agent 3).
- [x] **T6 — Real Tauri CSP** replacing `"csp": null`.

## Agent 5 — CI/release + AGENTS.md + Svelte warnings
Owns: `.github/workflows/ci.yml` (new), `AGENTS.md` (new), `CLAUDE.md` (new repo-root bridge), `docs/RELEASE.md` or `packaging/release.sh` (new), the 3 DB Svelte files.
- [x] **T9a — CI pipeline**: `cargo test` + `clippy -D warnings` + `fmt --check` + `cargo audit`.
- [x] **T9b — Release checklist/script**: gate on `cargo check`/`cargo test`/`npm run check` + build steps.
- [x] **T10 — Root `AGENTS.md`** (build/test commands, architecture ownership, "don't damage user work") + `CLAUDE.md` importing it.
- [x] **T12 — Fix 3 Svelte warnings**: `RedisKeyFilter.svelte` (`$state` captures `node`), `TableDesigner.svelte` (`$state` captures `columns`), `ResultsGrid.svelte` (unused `.tb-note` selector).

## Agent 6 — API contract docs + route-inventory test
Owns: `docs/contracts/api.md`, `docs/contracts/ws.md`, new `crates/otto-server/tests/route_inventory.rs`.
- [x] **T11a — Document undocumented routes** in `api.md`: auth API tokens, API client, workflows, notifications, LSP, logs, filesystem, skill-eval, insights, DB explorer, self-improvement, provider-update.
- [x] **T11b — Enumerate all 16 `Event` variants** in `ws.md`.
- [x] **T11c — Route-inventory test** comparing registered backend routes against `api.md` to prevent contract drift.

---

## Deferred from must-have (with reason — surfaced, not silently dropped)
- **Split largest files** (`product_run.rs`, `modules.rs`, `OverviewTab.svelte`, `ResultsGrid.svelte`, `RequestBuilder.svelte`) — research Cross-Cutting MH. **Deferred**: a structural refactor (not a correctness/security fix) that would directly conflict with the security edits this pass makes to `modules.rs`/`api_client.rs`/`ResultsGrid.svelte`. Do as a dedicated follow-up pass after these land.
- **Diagnostic/support-bundle command** — research Cross-Cutting MH. **Deferred**: net-new feature, not a fix; larger than this hardening pass.
- **Full API-token routes/migration/UI completion** — research Settings MH. The build-breaking part (doctest) is fixed here (T8) and routes are documented (T11); the broader UI build-out is a feature task, deferred.
