# Test-quality review — iteration 1 (2026-10-21)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, e2e specs that assert nothing meaningful.

Method: read-only (grep / sed / git log). No cargo, npm or e2e runs. Every
finding below was confirmed by reading the code at the cited lines.
`ui/src/modules/rooms/games/**` excluded.

## Score

Start 10.0. Confirmed open findings: 0 blocker, 2 major, 7 minor, 0 nit.

10.0 − (2 × 0.3) − (7 × 0.1) = 10.0 − 0.6 − 0.7 = **8.7 / 10**

## Previous findings

None. This is iteration 1 of this run. The 2026-10-07/08 `round-*-tests.md`
reports belong to an earlier effort and were not re-scored.

## Areas covered

- Route/auth test harnesses: `router_mount.rs`, `route_inventory.rs`,
  `policy_coverage.rs`, `rbac_matrix.rs`, `auth_security.rs`, `grants_api.rs`;
  `build_router` layering in `crates/otto-server/src/lib.rs`; `public_routes`
  and `module_routers` root vec.
- Recent PRs #111–#114: agy MCP reconcile/retire (`otto-sessions/src/mcp.rs`),
  git `merged_into` (`otto-git/src/local.rs` + `local/tests.rs` + e2e
  `desktop-git-branch-landed.spec.ts`), usage tailer native/agy/grok tests.
- Sweep 2026-10-11 fixes: agent cwd confinement (`check_agent_cwd` + the
  `ui_control.rs` route test), needs-you attention (`monitor.rs`,
  `routes/activity.rs` tests), agent-session reply protocol, "Always archive"
  unit tests.
- Agent UI control: `ui/src/lib/stores/uiControl.svelte.ts`,
  `ui/src/lib/uiCommands.ts` `confirmWrite`, `desktop-agent-ui-control.spec.ts`,
  `ui_control.rs`.
- Resource access: `routes/resource_access.rs` + `resource_access_tests.rs`,
  `desktop-resource-access.spec.ts`; reviewed DB change execution
  (`otto-dbviewer/src/service/changes.rs` + `tests/resource_access.rs`).
- Rooms terminal sharing (`rooms/terminal.rs`, `rooms_api.rs`,
  `manager/tests/mod.rs` room input/authority).
- Keychain master-key cell and sealing tests (`otto-keychain/src/control.rs`).
- MCP outward read-only SQL gate (outer classifier untested, but
  `otto-dbviewer/tests/mcp_query.rs` covers the engine gate, so no finding).
- Migrations 0183/0187 and their tests; `migration_compat.rs`.
- Support bundle (`routes/capabilities.rs`) and `otto-core/src/redact.rs`.
- E2E hygiene sweep: conditional `isVisible()`/`count()` branches, swallowed
  `.catch`, `waitForTimeout` density, CI gate vs advisory split
  (`ui/e2e/gate-specs.ts`, `.github/workflows/ci.yml`).
- UI unit hygiene: real-time `setTimeout` waits, source-text regex tests.

## Areas NOT covered yet (next iteration)

- `crates/otto-mcp/src/outward/exec.rs` `route_for` (2,086 lines; only
  `seg`/timeouts are unit-tested): per-tool argument → route mapping.
- `crates/otto-mcp/src/http.rs`, `otto-k8s/src/monitor/http.rs`,
  `otto-brokers/src/http.rs`, `otto-vault/src/http.rs`, `otto-swarm/src/http.rs`
  (large route files with no inline tests; check their integration tests).
- `otto-server/src/game_rooms/registry.rs` (server side of room games;
  possibly being rewritten by the games team).
- `crates/otto-server/src/state_archive/schema.rs` (684 lines, no inline tests).
- Workflow engine and swarm runtime: scheduler/verifier recovery tests.
- Product (`otto-product`) watcher and studio turns.
- UI stores with no unit coverage: `swarm.svelte.ts` (1,198 lines),
  `git.svelte.ts` paths beyond auto-fetch/ownership, `transcript.svelte.ts`,
  `sftp.svelte.ts`, `brokers.svelte.ts`, `clipHistory.svelte.ts`.
- Unit-test files that assert on source text (54 files read `src/`): only
  `autoSecurityPins.test.ts` was assessed.

## Findings

### test-quality-1-01 — major — no test pins which routes answer without credentials

- File: `crates/otto-server/tests/router_mount.rs:83-111` (with
  `crates/otto-server/src/lib.rs:169-214`, `crates/otto-server/src/routes/mod.rs:81-140`,
  `crates/otto-server/src/modules.rs` `module_routers` root vec)
- Evidence: `router_mount` already sends every inventoried `(METHOD, path)` to
  the real `build_router` with **no credentials**, and its header says "A
  mounted route answers with ITS behavior: 401 from the auth chokepoint for the
  protected API". But `probe` only distinguishes matched from unmatched: any
  status other than 405, the `no such route` JSON 404 or the SPA placeholder
  returns `Outcome::Matched` (line 111). A 200 counts the same as a 401.
  `build_router` merges `routes::public_routes()`, `rooms::public_routes()`,
  `game_rooms::public_routes()` and the `root` module routers outside the
  `auth_middleware` route_layer. A route moved from `protected` into one of
  those (or a new router added to the root vec) becomes reachable without a
  token, and nothing fails. `route_inventory` checks code against docs,
  `policy_coverage` checks `policy_for` (which never runs on public routes),
  and `rbac_matrix` uses a stub router.
- Fix: in `router_mount`, collect the status of every `/api/v1` probe that
  matched. Assert it is `401` unless `(method, template)` is in a pinned
  `PUBLIC_API` allowlist: `/health`, `/meta`, `/auth/login`,
  `/onboarding/root`, `/api-client/oauth2/callback`, `/ingest/*`,
  `/share/verify`, `/share/extend`, `/workflows/{id}/webhook/{token}`,
  `/webhooks/{workspace_id}`, the run-with-otto webhook, `/room-join`,
  `/game-room-join` and the plugin host routes. Mirror this for the root
  routes: WS upgrades and `/plugins/{slug}/ui/*` only. Fail with the diff, so
  adding a public route becomes a reviewed allowlist edit.

### test-quality-1-02 — major — the "Allow writes on ‹connection› for this session" memory is untested

- Files: `ui/src/lib/uiCommands.ts:524-551` (`confirmWrite`),
  `ui/src/lib/stores/uiControl.svelte.ts:183, 270-283, 296-316`
- Evidence: `confirmWrite` returns `true` **without a dialog** when
  `uiControl.writesAllowed(session, connId)` holds (line 527). Only
  unguarded, non-outward scopes are "rememberable" (line 526). The memory lives
  in localStorage (`otto_ui_write_allow`) and survives reloads. It is cleared
  only by `forgetWrites` on revoke (`setGrant(false)` line 183,
  `session_meta_updated` with the grant off line 275, `session_removed`
  line 281). No unit test imports `uiControl` or `uiCommands`
  (`rg 'uiControl|ui_control' ui/unit` finds nothing). The one unit test near
  this (`databaseWriteOwnership.test.ts:160`) stubs `confirmWrite`. The e2e
  (`desktop-agent-ui-control.spec.ts:522-543`) clicks only **Cancel**: it never
  ticks the box, never checks that a later write skips the dialog, never checks
  that the skip ends after Stop or a re-grant, and never checks that a guarded
  connection never offers it. That spec is not in `gate-specs.ts`, so it is
  advisory. The following would all pass CI:
  - a refactor that drops a `forgetWrites` call, so an agent re-granted later
    writes to the database without confirmation;
  - one that marks guarded (production) connections rememberable.
- Fix: move the remember/forget decision into a pure module, or keep it in
  the store and give `lsGet`/`lsSet` an injectable shim, which the
  source-harness tests already use. Add unit tests for:
  - remember → skip;
  - `session_meta_updated` with `enabled:false` → asks again;
  - `session_removed` → asks again;
  - `setGrant(false)` → asks again;
  - `guarded:true` and `outward:true` → never rememberable, never skipped;
  - a different scope → still asks.

  Add one e2e step that ticks the box, checks the second write runs without a
  dialog, then Stops and re-grants and checks the dialog is back.

### test-quality-1-03 — minor — agy `.agents/mcp_config.json` is never exercised by `retire_user_server`

- File: `crates/otto-sessions/src/mcp.rs:996-1030` (loop at line 999); the only
  test is `retiring_direct_credentials_preserves_other_servers_and_preview_is_readonly`
  (line 1069)
- Evidence: PR #114 made `retire_user_server` walk both `.mcp.json` and
  `agy_mcp_path`, but the test only reconciles into `.mcp.json` via
  `reconcile_managed_servers`. These are all untested:
  - removing a tracked entry from the agy file;
  - the "untracked entry → Err" refusal for the agy file (the step that keeps
    a direct credentialed server from surviving enforcement for agy sessions);
  - preview leaving the agy file byte-identical.

  Reverting the loop to `.mcp.json` only would pass. The route
  (`routes/resource_access.rs:139-165`) relies on this to retire direct
  servers before activating governed access.
- Fix: extend the retirement test to also run
  `reconcile_managed_servers_agy(root, &config)`. Then assert:
  - preview leaves both files unchanged;
  - apply removes `private` from both files and keeps `keep`;
  - a hand-added untracked `private` in `.agents/mcp_config.json` makes
    preview return `Err` naming that path.

### test-quality-1-04 — minor — `check_agent_cwd` covers only one of its bases; no cross-workspace or worktree cases

- File: `crates/otto-sessions/src/manager.rs:2844-2894`; test
  `crates/otto-sessions/src/manager/tests/mod.rs:605`
- Evidence: the agent-credential confinement allows four bases: the
  workspace root, the caller's own cwd, the repos registered in **this**
  workspace (`list_repos(&ws.id)`), and any path sharing a git common dir with
  a base. Its single unit test inserts a repo for `ws.id` and checks
  registered/inside/unrelated. The route test (`ui_control.rs:979`) checks only
  `/`, `CARGO_TARGET_TMPDIR` and `$HOME`. Untested cases:
  - a repo registered in **another** workspace must be refused. Swapping
    `list_repos(&ws.id)` for an all-repos listing passes every test.
  - a worktree of a registered repo is allowed, and a worktree of an
    unregistered repo is refused (the `resolve_git_common_dir` branch).
  - a relative path is refused.
  - a symlink inside a base pointing outside is refused (the canonicalisation
    order).
- Fix: add these cases to the existing test. Use a second workspace with its
  own repo row, and `git worktree add` in a tempdir for the common-dir pair.

### test-quality-1-05 — minor — e2e assertions hidden behind an instantaneous visibility check

- Files: `ui/e2e/desktop-usage.spec.ts:149-154`,
  `ui/e2e/desktop-session-header.spec.ts:183-189`
- Evidence: `desktop-usage` first asserts the usage engine is available, then
  `if (await reportBtn.isVisible().catch(() => false)) { …Report view and
  Download HTML assertions… }`. `isVisible()` does not wait. The tab strip
  renders only once the async `usage.status` loads (`UsagePage.svelte:441`,
  `{#if usage.status?.available || admin}`). So the Report assertions are
  skipped whenever the status lands after the heading, and also if the Report
  tab regresses away. `desktop-session-header` wraps every details-chip
  assertion in `if (await chip.isVisible())`, so a regression that drops the
  chip passes.
- Fix: replace the guards with `await expect(reportBtn).toBeVisible()` and
  `await expect(chip).toBeVisible()`. If the chip is conditional by design,
  decide the condition from the seeded data and assert it either way.

### test-quality-1-06 — minor — real-clock waits in vault store unit tests (flaky positive, vacuous negative)

- File: `ui/unit/vaultStore.test.ts:250, 309`
- Evidence:
  - Line 250 waits a wall-clock 700 ms, then asserts `calls === 2` and
    `dirty === false`. The retry timer is `500 * 2 ** busyRetries` ms
    (`ui/src/modules/vault/vault.svelte.ts:1005`), so the retried save must be
    scheduled, run and settle within a 200 ms margin. Under
    `node --test 'unit/**/*.test.ts'` running in parallel on a loaded runner
    this fails intermittently.
  - Line 309 waits 450 ms "past the 400 ms draft debounce" and then asserts
    nothing was stored. If the debounce has simply not fired yet, the test
    passes without checking anything.
- Fix: use `node:test`'s `mock.timers.enable({ apis: ['setTimeout'] })` and
  `mock.timers.tick(500)` / `tick(400)`. For line 309, after the tick, assert
  that the debounced flush ran (a positive signal) before asserting the agent
  text was not stored.

### test-quality-1-07 — minor — fixed 300 ms sleep before asserting the UI-control disconnect

- File: `crates/otto-server/tests/ui_control.rs:652-656`
- Evidence: `drop(main); drop(side); tokio::time::sleep(300ms)`, then it
  asserts `ui_visible == false`, `documents` empty and `no_ui_client`. These
  hold only once the daemon has processed both WS closes. On a busy CI runner
  (`it` is one binary run by nextest), 300 ms is not a bound. A slow close
  turns this into a flake, and the file already polls elsewhere (`gets(…)`).
- Fix: poll `ui_state` until `ui_visible == false` with a deadline of about
  10 s, the same way `wait_until` is used in `otto-sessions/tests/persist_restart.rs`,
  then make the remaining assertions.

### test-quality-1-08 — minor — security behaviour "pinned" by regex over component source

- File: `ui/unit/autoSecurityPins.test.ts:9-33`
- Evidence: the test says "nothing else tests them below e2e". It asserts
  `assert.match(fn, /userActivation/)`, `/OPEN_GAP_MS/`,
  `/ev\.source !== frameEl\.contentWindow/`, `/if \(!slugOk\) return;/`. Any
  of these passes while the behaviour is broken: the token can sit in a
  comment, or the check can exist without returning, or be placed after the
  `postMessage`. The tested behaviour matters:
  - the report-frame link bridge only opens http(s) URLs on a user gesture
    under a rate limit;
  - the plugin frame drops foreign `message` sources;
  - `otto:init` (with the plugin token) is never posted for an invalid slug.
- Fix: extract `onFrameMessage` (ReportDetail) and the PluginFrame message and
  init logic into plain `.ts` functions that take the frame window, the event
  and a clock. Test them behaviourally:
  - a foreign source is ignored;
  - `javascript:` is refused;
  - no gesture means no open;
  - a second open inside the gap is refused;
  - an invalid slug means no `postMessage`.

  Keep the sandbox-attribute regex. It checks markup, which is fine.

### test-quality-1-09 — minor — the support bundle (an export meant to leave the machine) has no test

- File: `crates/otto-server/src/routes/capabilities.rs:640-695`
- Evidence: `GET /support-bundle` is documented as "Everything sensitive is
  passed through `otto_core::redact::redact_json`". No test calls it
  (`rg support_bundle` finds only `policy.rs` and the policy snapshot). In
  fact only `settings` is redacted. `recent_audit` (lines 666-676) serialises
  `AuditEntry` rows as they are: `target` (a username or connection name),
  `detail` (free-form JSON) and `ip`. `redact.rs` has 4 unit tests on
  synthetic input. Nothing checks the assembled bundle, so a stored
  secret-shaped setting or an email-bearing audit target reaching the bundle
  would not fail anything.
- Fix: add an `it` suite test with `ServerCtx::for_tests`:
  1. Seed one setting with a nested `{"smtp": {"password": "…"}}`, one with a
     bearer-token string value, and one audit row with an email `target` and
     a `detail.token`.
  2. Call the handler as root and assert none of the raw values appear in
     `serde_json::to_string(&bundle)`.
  3. Assert a non-root caller gets 403.

  The test will fail today on the audit rows. Either redact them or narrow
  the doc comment.
