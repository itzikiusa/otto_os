# R01 — Foundation, persistence, identity and access

Review snapshot: merged `a0bd718b9fbc008d72c164ce643a24ed78e6368c`; main parent `196048df5bbd55a691b093ea0a3fa456f0912674`. Review branch: `review/quality-20261008`. Review date: 2026-10-08. This is a whole-domain assessment with explicit sampling limits, not a claim that every one of the 413 assigned tracked paths was line-reviewed. The ownership manifest is `../ownership.json`, assignment R01.

Correctness, performance and architecture were separate passes using their respective skills. Settings/share were assessed against the repository design guidelines. Production code was read-only for this reviewer. Coordinator-authorized mutations are the cache regression test, `tests/foundation_integrity.rs`, and its `tests/it.rs` registration. The coordinator owns the cache production fix and subsequent repairs. Concurrent R02/R03 edits are not attributed to the merged snapshot.

## Assessment

| Vertical | Snapshot assessment / 10 | Confidence | What prevents 9.8 |
|---|---:|---|---|
| Correctness / bugs | 7.3 | High for reported traces; medium for entire domain | Membership partial commit; non-restorable portable snapshots; enabled approval rules after restore; identity/credential disagreement; data-directory ownership gap. |
| Performance | 8.0 | Medium | Confirmed refresh-dependent cache memory growth; representative large-state export/restore, retention and contention workloads were not measured in this review. |
| Design | 8.1 | Medium | Archive safety depends on disconnected manual classification lists; identity changes do not commit the bearer and visible identity together; saved and live listener states share one representation. |
| UX / usability | 7.7 | Medium for behavior; low for rendered accessibility | Misleading identity and network state, unsent settings marked saved, and no fresh browser/native/assistive-technology evidence yet. |

These scores describe the merged snapshot and reviewed behavior, not the repaired branch. They are engineering assessments, not measurements inferred from test counts. The cache fix removes one material performance problem; it does not justify a domain-wide 9.8. Existing CI totals (5271 Rust / 1708 UI) are baseline context only.

## Findings

### R01-01 — High: a failed full membership replacement commits removals

**Source:** `crates/otto-server/src/routes/workspaces.rs:164` (remove loop through 169, insert loop through 172); `crates/otto-state/src/workspaces.rs` member mutation methods execute separately against the pool.

**Trigger:** an administrator submits a replacement list containing an unknown user and omitting the existing administrator/member. This is reachable through `PUT /workspaces/{id}/members`; no crafted SQL or concurrent race is required.

**Trace:** authorization and workspace existence succeed. The handler deletes omitted memberships immediately. Inserting the unknown user then fails its foreign key. No enclosing transaction rolls the earlier deletions back. A request that reports failure has removed access, potentially including the initiating non-root workspace administrator.

**Evidence:** `/tmp/r01-sqlite-repro.py` applies all 179 migration files to isolated SQLite with foreign keys on, then follows the exact deletion/insertion sequence: `FOREIGN KEY constraint failed`, memberships afterward `[]`. The production-handler Axum regression is `foundation_integrity::failed_membership_replacement_preserves_existing_members`; it uses a real `ServerCtx::for_tests`, real schema/repositories and the actual handler, injecting only the authenticated user extension. Execution status is recorded below.

**Fix:** implement the full replacement as one repository transaction, validate requested users/duplicates as appropriate, and return a domain validation error. Assert failed replacement leaves the complete old list unchanged and successful replacement persists the complete requested list. Avoid an O(current × requested) membership comparison by collecting requested IDs once, although atomicity is the material issue.

**Provenance:** existing defect; this handler did not change in PR94.

### R01-02 — High: portable Canvas snapshots contain references to omitted sessions

**Source:** `crates/otto-server/src/state_archive/schema.rs:113`; `crates/otto-state/migrations/0093_canvas_scene_refs.sql:7`; export through `crates/otto-server/src/routes/backup_git.rs:684` and `state_archive.rs:144`.

**Trigger:** export a Git/portable snapshot after attaching a Canvas scene to an agent session, then preview/import it into a fresh profile.

**Trace:** portable table selection includes `canvas_scene_refs` but excludes `sessions`. The non-null `session_id` foreign key remains in the exported row: export sanitization, import user remapping and runtime sanitization do not remove/remap it. Restore inserts the row and the final foreign-key check fails. Preview catches that error and leaves `can_restore=false`. This is not merely a schema-closure conjecture: the actual export and import transformations were traced.

**Evidence:** a migration-derived FK inventory finds `canvas_scene_refs.session_id -> sessions.id` as the portable included-to-excluded dependency. The new `foundation_integrity::portable_archive_with_session_attached_canvas_restores_on_fresh_profile` calls real `build_snapshot(portable=true)`, asserts the Canvas content is exported and sessions are excluded, then calls real `preview_restore`/`restore_snapshot` against another isolated schema. Existing archive tests used full snapshots and did not seed a scene-ref row.

**Fix:** omit runtime session references from portable snapshots (with an explicit exclusion note), preserving scenes and full-archive references; alternatively export a deliberate coherent session subset. Make the regression exercise full export → preview → restore, not just a hand-built archive. Cover older portable snapshots containing orphan refs with a documented cleanup/error policy.

**Provenance:** existing defect, not PR94 regression.

### R01-03 — High: full restore reactivates MCP automatic approvals

**Source:** `crates/otto-server/src/state_archive/schema.rs:68` and `:331`; `crates/otto-state/src/mcp_auto_approve.rs:110`; `crates/otto-server/src/mcp_auto_approve.rs:132`.

**Trigger:** restore a full archive containing an enabled global MCP auto-approval rule. Per-tool rules may carry `allow_irreversible=1`.

**Trace:** the full export exclusion list omits other permission bindings and MCP allowlists but includes `mcp_auto_approve_rules`. The inert-runtime conversion never sets this table's `enabled` to zero. Restore retains/remaps `created_by`, but disabling imported users does not make the rule inactive: live resolution selects enabled global/workspace/session rules without a creator-disabled check. A restored enabled rule therefore applies to later governed calls. Existing destination users with matching usernames are also remapped, so disabled-imported-user handling cannot be used as a safety argument.

**Consequence:** approval behavior changes through restore without renewed per-rule consent, including irreversible tool calls if that flag was saved. This conflicts with the archive's stated exclusion of active permission bindings and its inert-service preview promise. No tool was invoked in this review.

**Evidence:** `foundation_integrity::restored_archive_does_not_activate_mcp_auto_approval` exports an actual enabled rule, previews/restores the full archive and asks the production `McpAutoApproveRepo::list_applicable` what future calls can use. It permits either safe omission or disabled preservation, asserting no active rule.

**Fix:** explicitly exclude this authorization table and reject imported active rules, or import it disabled with clear review UI. Sanitization must apply to import as well as export so old archives are safe.

**Provenance:** existing defect; seam owned jointly with future R10 review.

### R01-04 — Medium: auth-cache refresh accumulates duplicate reverse-index entries

**Source:** merged `crates/otto-rbac/src/cache.rs:170-176` (`Vec::push`); lazy expiry at `:154-164` removes the primary cache entry only.

**Trigger/cost:** a continuously used bearer expires from the ten-second cache and is reinserted on its next request. Each refresh pushes another copy of the same token hash onto the per-user vector. One continuously active bearer can retain 8,640 copies/day, approximately 0.76 MB/day in 64-byte hashes plus String storage before allocator overhead. User eviction revisits every duplicate. Cost grows with daemon lifetime rather than distinct credentials; login/logout churn is not required.

**Evidence:** actual `AuthCache` regression `cache::tests::repeated_expiry_refresh_keeps_reverse_index_bounded` expires/reinserts one token 100 times without wall-clock sleeps and confirms revocation still removes it. On merged behavior it failed with reverse cardinality 101 versus expected 1, while the preceding eviction assertions passed. Command: `cargo test -p otto-rbac --lib cache::tests::repeated_expiry_refresh_keeps_reverse_index_bounded -- --exact`; `/tmp/r01-auth-cache-red.log`, exit 101.

**Fix/status:** coordinator changed `ByUser` to `HashSet<String>`. This reviewer independently inspected the diff: it preserves existing per-user and process-wide eviction semantics while bounding repeated refresh to one hash. Coordinator reports all 61 library tests green in `/tmp/postmerge-r01-auth-cache-green.log`. This is the only finding with a production repair reviewed here so far. The test proves bookkeeping cardinality, not a process RSS benchmark.

**Provenance:** existing defect, not PR94 regression.

### R01-05 — Medium: failed impersonation transitions leave visible identity and bearer inconsistent

**Source:** `ui/src/lib/stores/auth.svelte.ts:309-313`, `:330-343`; shell banner is derived from `auth.isImpersonating` in `ui/src/App.svelte:1066`.

**Trigger:** the token-switch request succeeds but the subsequent `/auth/me` request returns 503, both when starting and stopping impersonation.

**Trace/evidence:** the store changes the bearer first, then awaits identity/capabilities. A failed `loadMe` leaves the prior `me` and `realUser` displayed. `/tmp/r01-auth-identity-repro.mjs` executes the actual production AuthStore using the repository source harness with a failing transport. Results:

```
Failed impersonation start: token=imp, displayed=root, banner=false, rootControls=true
Failed impersonation stop: token=admin, displayed=guest, banner=true, rootControls=false
```

This proves asynchronous store behavior; it is not a rendered-browser claim. Server authorization still applies to the actual bearer, so this is not described as an auth bypass. However, a person can issue an action as the admin while the UI still represents the guest after a failed stop.

**Fix:** make credential and visible-identity transition coherent: stage/verify the new identity, or enter an explicit blocked transition/recovery state until loaded. Roll back and revoke a failed start where appropriate. On stop restore trustworthy saved identity or block actions until refreshed. Add negative-path tests for `/auth/me` and capabilities failures and stale concurrent responses, not only happy start/stop.

**Provenance:** existing defect, not PR94 regression.

### R01-06 — Medium: safety posture reports desired configuration as the live listener

**Source:** `crates/otto-server/src/routes/audit.rs:43-63`; `ui/src/modules/settings/TrustSafety.svelte:258-272`; `crates/ottod/src/main.rs:574-598`.

**Trigger:** disable an already-running network listener in settings, then open Trust & Safety without restarting the daemon. The inverse also occurs when enabling it before restart or after a failed bind.

**Trace:** `security-posture` reads the persisted `network_listener.enabled`; the UI labels it “Off”, “Only this Mac can connect” and “Loopback only”. The listening socket is created once at boot and remains open until daemon shutdown. Daemon settings correctly say restart is required, but this other surface removes that qualification. The contract documents a settings-derived response, which explains the implementation but does not make the live-safety copy true.

**Fix:** represent actual bound port and desired next-boot configuration separately, using the runtime listener status already exposed through `transport::network_listener_port()`. Show restart-required state. Update the API types and contract with the distinction, plus a regression where desired=false and actual=Some(port). No real listener was started for this review.

**Provenance:** existing defect; source-traced, no screenshot/runtime socket test.

### R01-07 — Medium: Daemon settings mark edits made during save as persisted

**Source:** `ui/src/modules/settings/Daemon.svelte:154-176`; port/sandbox/grace controls at `:234-259` and `:293` remain editable while saving.

**Trigger:** send port 7701, change the input to 7702 while PUT is pending, then let the request finish. The same pattern applies to sandbox/session settings.

**Trace/evidence:** request body is correctly captured before the await, but `savedListener`, `savedSandbox` and `savedSessions` are assigned from current draft variables after the await. `/tmp/r01-daemon-save-repro.mjs` extracts and runs the real TypeScript `save()` function under a deferred transport:

```
{submitted:7701,currentDraft:7702,markedSaved:7702,dirtyByProductionFormula:false}
```

The server has 7701 while the form claims 7702 is clean. Save becomes unavailable and navigation no longer warns about the unsent change. The success toast also uses the unsent value.

**Fix:** capture submitted values and advance the saved baseline only to those values, leaving later edits dirty; or disable all editing controls during submission. Preserve the submitted-value approach for consistent feedback even if UI controls are disabled. Add a deferred-response test and rendered slow-save test.

**Provenance:** existing defect; actual save-function reproduction, not browser proof.

### R01-08 — High: single-instance guard identifies the port, not the data directory

**Source:** `crates/ottod/src/main.rs:329-338`, `:347-383`; `crates/ottod/src/config.rs:18-31`; `crates/ottod/src/housekeeping.rs:30-38`.

**Trigger:** two daemon processes use the same `OTTO_DATA_DIR` but different `OTTO_PORT` values (for example a developer starts a custom-port daemon without isolating its data directory).

**Trace:** configuration selects these independently. Startup calls a successful bind on the second port its “single-instance lock”. The running marker only reads/overwrites a text file. There is no profile filesystem/advisory lock in ottod or the boot/database path. The second process then reaches offline compaction, module construction, recovery and scheduling over the live profile. The concrete ClickHouse seam calls `reclaim_dir` (`crates/otto-usage/src/clickhouse.rs:1025`), verifies that the PID references the same data directory, and sends TERM/KILL; it does not prove that the owning daemon is dead. This is exactly the side effect the startup guard's comment intends to prevent.

**Consequence:** simultaneous profile ownership can stop the first daemon's usage server, run recovery/schedulers twice, and violate the offline-maintenance assumption. No destructive scenario was executed.

**Fix:** acquire and retain an OS advisory lock keyed to the canonical data directory before any mutation; fail clearly if another daemon holds it. Keep port conflict detection as a separate error. Verify with two harmless lock-only child processes over a temp directory and distinct-port configurations, plus symlink-alias coverage. Do not test by starting two real daemons on user data.

**Provenance:** existing source-confirmed defect, not PR94 regression; actual dangerous process behavior intentionally unexecuted.

## Structural observations and positive evidence

Archive safety is schema-discovered but permission exclusions, portability and runtime deactivation are maintained as separate hand-written lists. R01-02 and R01-03 demonstrate the resulting future maintenance cost: adding a table or dependency can silently change restore semantics. A single explicit per-table policy (portable inclusion, import permission, runtime transform, dependency treatment) plus a schema-inventory assertion would make unclassified tables a review/test failure. This is a concrete refactor direction attached to demonstrated defects, not a demand to build a generic migration framework.

The read/write pool router is deliberately conservative, routes raw acquisition and write transactions to the writer, and uses `BEGIN IMMEDIATE` for read-then-write operations. Archive preview rolls back a real transaction; commit is guarded by a preview token derived from relevant target state. File restoration uses descriptor-relative opens, rejects symlink descendants, creates leaves exclusively, syncs writes and limits rollback to owned matching bytes. Git snapshot publication stages a new file and checks expected hashes. These are meaningful protections inspected in source; crash-injection and adversarial filesystem races were not freshly exercised.

Maintenance keeps the original database through a staged swap and confirms only after reopen/migration, including explicit rollback markers. Append-only schema rules and migration compatibility tests exist. The three PR94 migrations inspected (0178–0180) add a nullable snapshot field/indexes and do not remove old columns. Detailed session/archive semantics belong to the R02 and R08 seams; additive shape alone is not semantic proof.

Login throttling bounds tracked key count, pins real-account counters and fails closed under saturation. SSRF address classification and resolver paths, sandbox profile path/network emission, auth root/workspace guards, resource-access evaluation, grant caches and keychain encryption/caching were sampled. No unverified network exploit or native Keychain guarantee is claimed.

## Coverage and boundaries

**Deeply traced paths:** auth request extraction/authentication/cache lifetime and revocation callers; workspace CRUD/membership and user/grant/access-group/resource-policy routes; archive table selection, sanitization, owner remapping, preview, restore and file publication; database open/migration/repair and offline-compaction recovery; auth UI bootstrap/impersonation; Daemon, full backup, safety posture, account/admin setup.

**Read or materially sampled:**

- `otto-core`: auth/domain/API definitions consumed by handlers, `paths.rs`, secret traits. Most other domain/API definitions were not independently line-reviewed.
- `otto-state`: `db.rs`, `pool.rs`, `maintenance.rs`, workspaces/users/settings/grants, resource-access/group repositories, network/provider/project repositories and selected queries, retention policy and batching sections, archive-related schemas. All 179 migrations were executed by the isolated SQL evidence harness and their FK graph inspected; selected security/Canvas/runtime/new migrations were read. Execution is not line review of every historical migration.
- `otto-rbac`: cache and core authentication, token issuance/revocation/impersonation call chains and tests; resource-access evaluation. The entire long token test module was not reread line by line.
- `otto-keychain`: encryption/file-store migration and secret-cache behavior; `otto-netguard`: IP classification/resolution; `otto-sandbox`: profile network/path emission. Native Keychain prompts, encrypted-store crash recovery and actual Seatbelt execution remain unverified.
- `ottod`: config, boot ordering and listener ownership, network listener lifecycle, running marker; server boot/open/build/recovery/background-task sections and shutdown signal. Full desktop packaging is R13, inward/outward MCP entry points R10.
- Server routes: workspaces/users/projects/provider_accounts/network_profiles/access_groups/grants/audit/admin_sessions/onboarding in depth; settings/auth_routes/impersonate/email_sender/share/backup/backup_git/resource_access and guards sampled by relevant failure paths. Full OTP/tunnel end-to-end, every policy mapping and every backup Git conflict mode remain unverified.
- UI: `auth.svelte.ts`, `Settings.svelte`, `Daemon.svelte`, `FullBackup.svelte`, `SecretsStore.svelte`, `Users.svelte`, `AdminSessions.svelte`, `TrustSafety.svelte`, `SharePage.svelte`; auth/access adapters and shell banner seams. Settings subsections not listed were inventory-only or shallowly sampled.
- Tests: cache tests, archive tests/fixtures, `authBoot.test.ts` and source harness, representative auth/RBAC/admin/router integration fixtures; selected access E2E source. Existing suite names alone are not execution evidence.

**Performance boundary:** cache growth is established by cardinality and an explicit allocation model, not a timed load test. Admin-session listing still loads the entire historical session set and renders all filtered entries, but it is a manually refreshed admin surface; this review does not inflate that cold path into a measured blocker. Archive size is bounded at 256 MiB and individual assets at 64 MiB, but serialization/copy amplification, reader snapshot duration and large-table import query count need realistic dataset measurements. Retention and migration contention at scale were not benchmarked. No Cargo clean, concurrent Cargo builds or real daemon were used.

**UI boundary:** there are no fresh light/dark/mobile/RTL/keyboard/screen-reader screenshots from this reviewer yet. Source-based feedback/identity findings are valid within their stated evidence, but cannot certify visual quality or target-size/focus behavior. Named candidate follow-ups are `desktop-platform-settings-recovery`, `desktop-backup-archive`, `desktop-backup-git`, `desktop-ux-r4-access`; a browser lease must be granted first. No full Playwright suite is requested.

## Executed evidence and pending gates

- Cache RED test: failed exactly on 101-versus-1 reverse cardinality; `/tmp/r01-auth-cache-red.log`.
- Cache fix: independent diff review completed; coordinator's 61-test green log `/tmp/postmerge-r01-auth-cache-green.log` (not rerun by this reviewer).
- Isolated Python SQLite migration/foreign-key reproduction: `/tmp/r01-sqlite-repro.py`, SQLite 3.38.3, no user database touched.
- Actual production AuthStore failure harness: `node /tmp/r01-auth-identity-repro.mjs`, successful execution demonstrating identity/bearer mismatch.
- Actual extracted Daemon save-function harness: `node /tmp/r01-daemon-save-repro.mjs`, successful execution demonstrating an unsent draft marked clean.
- Real Rust archive and HTTP-handler regressions: `cargo test -p otto-server --test it foundation_integrity:: > /tmp/r01-foundation-integrity-red.log 2>&1`; RED confirmed: 0 passed, 3 failed, 183 filtered; tests completed in 2.00 s after a 1m40s build. The failed membership request left `[]`; portable preview reported missing `canvas_scene_refs` references; full restore returned an enabled global rule with `allow_irreversible=true` from the production applicable-rule query. All failures hit the intended behavior assertions, not setup/compilation. The macOS linker emitted its existing large unwind-table warning. No production fixes for these three regressions were applied by this reviewer.

The original scope remains larger than this inspected subset. In particular, the many feature-specific state repositories need their domain reviewers' caller/contract evidence reconciled with this foundation review, and historical migration rollback on representative old populated schemas requires the dedicated compatibility suite. A complete repository verdict must retain these gaps rather than treating ownership assignment as verified coverage.

## Repair pass (working branch, separate from snapshot findings)

Coordinator authorized this reviewer to implement R01 repairs after the initial RED evidence. Current changes:

- R01-01: `WorkspacesRepo::replace_members` owns one transaction and rejects duplicate IDs; handler delegates the full replacement. Foreign-key failure becomes 400 while transaction rollback preserves prior access. Integration regression now also checks a duplicate replacement and a successful role change.
- R01-02: portable exports omit `canvas_scene_refs` and disclose that exclusion; full archives retain them. Older portable archives with orphan references deliberately remain rejected during preview with the existing actionable missing-reference diagnostic; no silent link deletion. Integration coverage checks all three cases.
- R01-03: both export and import sanitization force `mcp_auto_approve_rules.enabled=0`. The test now simulates a legacy archive by re-enabling its exported row before import, then checks the production applicability query is empty. Fixture target is the real irreversible `merge_pr` tool; no call is executed.
- R01-04: coordinator's independently reviewed HashSet repair remains unchanged.
- R01-05: bearer changes clear visible identity/capabilities and enter a loading state until verified; failure enters the existing offline recovery/retry flow. Late `/auth/me` responses are token-checked. Existing capabilities loading already checks token identity. Added start/stop 503 and stale response/new-sign-in regressions.
- R01-06: posture reports the actual bound listener plus `network_listener_restart_required`, with Rust/TypeScript/contracts updated together. UI preserves the actual exposure warning while giving the restart notice a separate block.
- R01-07: save baselines and success feedback use submitted values, leaving later draft edits dirty. Deferred production-save test covers listener, sandbox and session grace.
- R01-08: new `ottod/src/profile_lock.rs` acquires a nonblocking exclusive `flock` on canonical-profile `ottod.lock` before boot mutation, retains it through shutdown and never unlinks the inode. `CLOEXEC` prevents child programs retaining it. The old diagnostic marker is a separate `ottod.running` file. Lock-only child-process tests cover same profile/different port, symlink alias and release; they never run daemon boot. Compatibility limit: older binaries do not take this advisory lock, so stop old daemons during upgrade; the unchanged loopback bind still detects same-port overlap.

Fresh UI verification after repairs: `node --test ui/unit/authBoot.test.ts ui/unit/daemonSettingsSave.test.ts`: 13 passed, zero failed (`/tmp/r01-ui-fixes-green.log`). Negative transition and draft-save tests first failed on their intended assertions (`/tmp/r01-auth-transition-red.log`, `/tmp/r01-daemon-settings-red.log`). `npm run check` passed with zero Svelte errors/warnings (`/tmp/r01-ui-check.log`); the added named E2E spec also passed the dedicated E2E TypeScript check.

Fresh browser verification: `OTTO_E2E_SLOT=r01foundation OTTO_E2E_PORT=17811 OTTO_E2E_PW_PORT=5181 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=.../target/debug/ottod npx playwright test e2e/desktop-foundation-integrity.spec.ts --project=desktop-browser --workers=1`: 2 passed in 4.7 s (`/tmp/r01-foundation-browser.log`). This uses route fixtures with the actual UI and an isolated daemon harness, so it proves rendered UI state and draft preservation, not the repaired backend binary. Screenshots `/tmp/r01-trust-safety-1440-light.png` and `/tmp/r01-trust-safety-390-dark.png` were viewed; both fit without horizontal document overflow. The file-secret warning belongs to the deliberate isolated E2E fixture. A capture refresh is queued after the coordinator requested waiting for the loaded Refresh button and a separate restart-notice block. Broader keyboard/RTL/native/accessibility journeys remain outside this two-test evidence.

Rust GREEN verification for the repair batch remains queued behind the coordinator's Cargo lease; do not mark these backend fixes complete before it runs. No migration file or migration baseline was changed. The original domain scores remain snapshot scores until the repair and integration evidence is complete; even successful narrow regressions do not close the unreviewed-domain and scale-measurement gaps above.

### Coordinator verification update

The production-handler/archive/posture regression suite now passes: `cargo test -p otto-server --test it foundation_integrity::`, 4 passed, 0 failed (3.12 s execution; build 2m33s). Log: `/tmp/postmerge-foundation-green.log`.

Final settings browser capture after the notice-block/loading-wait adjustment passes: named `desktop-foundation-integrity.spec.ts`, 2 passed (3.7 s), `/tmp/postmerge-foundation-browser.log`. Screenshots retain the same paths above. Root inspected desktop/light and phone/dark layouts.

Independent lifecycle review moved the profile lock from `run()` into `main()` immediately after configuration: ownership now precedes log mutations and remains held through `runtime.shutdown_timeout`. The earlier placement released it before blocking-task shutdown. That small placement refinement still requires the queued ottod check/lock tests. Other broader Rust gates listed above remain pending; these results are not a completed whole-domain score.

### Coordinator auth-cache follow-up

Two additional cache regressions reproduced against the first repair: a pending DB lookup repopulated a token after revocation, and 4,128 distinct expired token contexts remained retained. `cargo test -p otto-rbac --lib cache::tests::` reported 9 passed / 2 failed; evidence `/tmp/postmerge-r01-cache-fill-red.log`.

The follow-up captures invalidation generation before the DB read, serializes cache publication with eviction, and retains at most 128 token/user invalidations. A fill affected by an intervening revoke, or older than retained history, cannot populate the cache. Unrelated revocations retain valid fills. Contexts and reverse memberships are bounded at 4,096; capacity triggers expired-entry cleanup and a safe cache-clear fallback, while lazy expiry removes reverse membership immediately. Extra regressions cover history overflow, live-entry saturation and repeated read-time expiry. These limits bound retained entries, not arbitrary external request concurrency. A request already validating when revocation occurs may finish; it cannot restore future cache hits.

Production implementation is installed; full RBAC tests and clippy are queued with R11. No passing claim is made before those results.

Auth-cache follow-up verification: full `cargo test -p otto-rbac --lib` passed67/67 in21.54s, log `/tmp/postmerge-r01-cache-fill-green.log`. The temporary unfenced implementation is gone. Package/workspace clippy remains queued with final integration.

Additional archive journey: named `desktop-backup-archive.spec.ts` passed1/1 in5.0s using isolated slot r01archive, real fixture records/Vault files and the existing R04-built daemon containing R01 archive repairs. Verified additive preview/restore, credential exclusion, and tampered-file rejection. Log `/tmp/postmerge-r01-archive-browser.log`; this is not a large-archive performance benchmark.
