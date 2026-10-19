# E19-extensibility — iteration 01

Verdict: **Approve with fixes; not at the 9.8 target.** Five major findings, zero demonstrated blockers. Scoped engineering score: **8.2/10**, a source-review judgment, not a measured product-wide score. All five findings are confirmed by hand-traced source paths; none was executed.

## Provenance and constraints

- Reviewer: E19-extensibility; 2026-10-18.
- Branch: `fix/db-parity-session-consistency`; HEAD `c009da20cc113c8fb79f3bf0600442d0c6afa358` at review start.
- Reviewed existing subsystem behavior, not merely PR111's diff. Working tree was moving: sessions, git, workflows and swarm had other reviewers' edits. Findings below concern files unchanged in the observed worktree diff.
- Read AGENTS.md and correctness-review, test-review, architecture-review and performance-review instructions. Memory search produced no relevant prior evidence; conclusions use current source.
- No Cargo, build, test, browser execution, git mutation or production edit performed. This report is the only filesystem write. Findings sent to root session `01M57PYK75FC1F2AFS4C73BBF7` through `otto_send_message`.

## Ranked findings

### E19-01 — Major: a stale browser janitor snapshot closes a replacement session

**Locations:** `crates/otto-browser/src/live/runtime.rs:478-492`, `:319-341`, `:353-364`.

**Expected:** cleanup applies to the session inspected, while reopening a crashed tab creates a usable replacement.

**Concrete interleaving:** sweep snapshots `(tab T, Arc old)` at lines 478-482. Old is crashed and has no viewers. A concurrent `open(T)` removes old, closes it, creates new, and inserts `Arc new` under T at line 341. Sweep resumes, evaluates `!old.is_live()` and `old.viewer_count() == 0`, then calls `self.close(T, "closed")`. That function removes the *current* map entry, new, and closes its Chromium target. `open_lock` serializes only open calls; neither sweep nor close participates. The same identity issue exists in the idle branch. With multiple snapshot entries, earlier asynchronous closes provide an additional interleaving window.

**Actual output:** a successful reopen is followed by an unsolicited close of the fresh tab; the old session's state determines the new session's fate.

**Fix direction:** remove a map entry only when its Arc is pointer-equal to the snapshot candidate, under the sessions lock, then close that exact removed instance outside the lock. Apply this to both janitor branches. Alternatively serialize lifecycle transitions, without holding a blocking mutex across awaits.

**Regression needed:** deterministic barrier/hook after the snapshot; replace old with new; resume sweep; assert the map still contains new, new remains live and its target receives no close command. Cover both crashed and idle snapshots. Current runtime tests at `runtime.rs:562-613` exercise install/profile/status validation, not this interleaving. No live Chromium claim is made.

### E19-02 — Major: MCP discovery silently discards all tool pages after the first

**Locations:** `crates/otto-mcp/src/client.rs:196-206`; consumer `crates/otto-mcp/src/service.rs:515-551`, `:670-684`.

**Expected:** discovery returns the server's advertised tool catalog, including a continuation supplied by the server.

**Input and trace:** first `tools/list` response is `{"tools":[{"name":"A","inputSchema":{}}],"nextCursor":"page2"}`; the second page would advertise B. `list_tools` issues one request with no cursor, clones only `tools`, and returns. `discover` upserts only A and records count 1. No `nextCursor` handling exists in this client. On a fresh registry, B has no discovered row; invoke treats an unknown tool as disabled and denies it. Refresh repeats the same result.

**Impact:** valid tools disappear behind a server's first-page boundary; users cannot grant or invoke them through the governed gateway. This is functional interoperability, not a request to weaken governance.

**Fix direction:** consume continuation cursors until exhausted, validate the response shape, and accumulate tools before publishing discovery. Bound total pages/tools/bytes and detect repeated cursors so supporting pagination does not introduce unbounded work. Retain the existing per-operation admission and cancellation behavior.

**Regression needed:** fake stdio and/or HTTP server with two pages; assert second request carries `params.cursor`, both tools are persisted and the count is two. Add repeated-cursor and later-page-error cases. Existing one-page stdio fixture at `client.rs:1468-1470` and `stdio_list_tools` at `:1511` cannot detect this omission.

### E19-03 — Major: rejected HTTP MCP initialization is reported healthy

**Locations:** `crates/otto-mcp/src/client.rs:495-520`, `:746-760`; `crates/otto-mcp/src/service.rs:555-584`.

**Expected:** health proves a successful initialization handshake. A JSON-RPC initialization error must fail before sending initialized or tool requests.

**Input and trace:** endpoint answers HTTP 200, JSON content type, `{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"unsupported protocol version"}}`. `parse_http_message` successfully parses and returns that object. `open_http` discards it at line 505, sends `notifications/initialized`, and returns `Live::Http`. `health()` maps the opened session to success; `health_check` writes `healthy`. Unlike this HTTP path, stdio `read_until_id` explicitly rejects `error` at `client.rs:947-949`.

**Impact:** the Health action reports a rejected server as healthy, suppressing the actionable handshake error and allowing later calls to proceed against a session that never initialized.

**Fix direction:** validate initialization response identity and success/result structure before announcing initialized. Surface its JSON-RPC error. Share envelope validation where appropriate, while preserving HTTP/stdio transport differences.

**Regression needed:** local HTTP fixture returning the above error; assert health fails with the handshake error, no initialized/tool call follows, and service health becomes unhealthy. Include malformed/mismatched-id envelopes. Happy-path stdio health at `client.rs:1506` does not falsify this HTTP failure.

### E19-04 — Major: an immediately exited plugin is enabled as though startup succeeded

**Locations:** `crates/otto-server/src/plugins.rs:184-215`, `:228-229`, `:590-608`.

**Expected:** distinguish a still-starting process from a process that has already failed. Enable should roll back on a failed startup.

**Input and trace:** installed manifest has valid nonempty argv `['sh','-c','exit 1']` (equivalently a Node plugin exiting for a missing dependency). OS spawn succeeds. Manager inserts its Child and port; all readiness probes fail. It never checks child exit status, unconditionally logs “sidecar started” and returns `Ok(port)`. The enable handler's rollback only runs on Err, so it returns an enabled record. `port_of` continues returning the dead child's port; proxy requests fail with 502. No plugin child-monitoring task removes the entry.

**Fix direction:** check/reap an exited child during readiness and return an actionable startup error, removing its stale running entry. Keep the explicitly documented best-effort policy for a *live but slow* process. A wider auto-restart policy is not required for this fix.

**Regression needed:** enable a fixture process that exits immediately with nonzero status; assert enable fails, enabled flag is false, and no running port remains. Separately verify a slow but living child retains the intended behavior. Existing plugin tests beginning at `plugins.rs:1026` cover replacement/token disabling and proxy deadlines/body caps, not this distinction.

### E19-05 — Major: bulk skill installation skips recovery after provider mirroring fails

**Locations:** `crates/otto-skills/src/http.rs:309-318`, `:374-390`; `crates/otto-context/src/user_skills.rs:95-120`.

**Expected:** retrying a failed bulk install after correcting the underlying filesystem error finishes installing the skill for providers.

**Input and trace:** Library install succeeds, then one provider directory is unwritable. `install_one` first updates Library at line 374; `user_skills::install` subsequently returns an IO error while copying or writing a provider ownership manifest. Bulk response correctly reports failure. User repairs permissions and retries Install All. `install_state` sees the current Library version; `install_many` takes UpToDate and skips the skill before provider reconciliation. `force=true` does not bypass this branch. The retry can report only skipped with missing/stale provider installations remaining.

**Impact:** a recoverable filesystem failure becomes a persistent partial installation hidden by Library-only freshness. Individual reinstall is a workaround for some failures, but bulk retry itself does not recover.

**Fix direction:** separate updating the Library from reconciling managed provider copies; reconcile providers on bulk retry even when Library is current, while preserving user-owned destinations. Track ownership atomically when materializing a new provider tree so a partially copied Otto tree is not mistaken for user-owned content on a later attempt.

**Regression needed:** inject provider destinations (never actual home directories), fail one mirror after Library succeeds, repair it, retry bulk install, assert all managed destinations and manifests are complete. Existing `install_many_skips_up_to_date_skills` at `http.rs:517-532` explicitly seeds Library only and asserts skipped; it would remain green with broken provider reconciliation and needs a separate recovery oracle.

## Coverage and assessment

Read/traced the following paths beyond the diff:

- MCP: outbound client slot admission, initialize/list/call, reuse and safe-retry distinction, HTTP/SSE framing and body bounds, service discovery/health, invoke approval/audit pipeline, final send reauthorization, policy matching/ranking, and selected outward routing/catalog/tests plus JSON-RPC framing and server glue.
- Plugins: manifest/install/replace/enable/disable/remove, child ownership, reverse proxy timeout/body handling, Codex host-run helper, copy helper, relevant tests and operator contract.
- Skills: bundled version/state calculation, Library installation, backup/reservation/pruning, provider ownership/copying, bulk error handling and related tests.
- Browser: fallback fetch bounds/deadline, Lightpanda supervisor shutdown/restart, reader CDP context lifecycle, live runtime process/session map and janitor, session creation/close paths, process startup, and screencast flow control.

Positive evidence: MCP caps reusable sessions at two per client with a global permit budget; transport ownership is removed from its slot during an operation so cancellation drops unread state. Send-time checks re-read resource/tool policy after admission and initialization. Plugin management transitions have a lifecycle mutex and replacement disables/stops the old installation. Proxy and fallback reader bodies/deadlines are explicitly bounded. Live Chromium uses a process cap, per-session ephemeral contexts and bounded frame flow. No proposal here removes root authority or blocks intentionally registered local MCP endpoints.

Architecture: state-free outward routing/framing in otto-mcp and host state in otto-server form a useful existing boundary. No standalone architecture finding is asserted merely because routing/plugin files are large. The demonstrated maintenance issue is the duplicated success-envelope interpretation across MCP transports (E19-03); a small shared validator is justified by two existing transports. Test gaps are attached to actual defects above rather than counted again as independent findings.

Performance: reviewed explicit caps and cleanup ownership, not benchmarks. No separate performance blocker is claimed. The browser race is lifecycle correctness; MCP pagination fixes must preserve bounded aggregate cost. Latency, peak memory and workload scale were not measured.

## Residual boundaries and follow-up

- This is a source review, not certification of the whole product or every line of these crates. The complete outward tool catalog was not individually matched against every native endpoint/schema. Auto-approval UI, all RBAC permutations, full browser download/interception/install behavior, third-party servers/plugins and every bundled skill's content remain outside this pass's deep coverage.
- Reader CDP cancellation deserves an execution follow-up: `cdp.rs:614-619` cancels the inner operation on timeout; cleanup at `:656-658` / `:728-730` is then skipped, and context creation at `:395` sends no `disposeOnDetach`. Client Drop only aborts sockets. Whether a particular Lightpanda build destroys these contexts on disconnect was not established, so retained browser contexts are **not counted as a confirmed sixth finding**. Test disconnect/timeout against the supported engine and count contexts/targets.
- HTTP response correlation currently has fallback handling for mismatched response ids; protocol compatibility intent and impact need a dedicated transport pass before a separate claim.
- No full CI, compile, tests, runtime reproduction or screenshots were authorized or run. Proposed tests above need root allocation and isolated fixtures. Findings should be rechecked against subsequent edits before implementation/re-review.
- E16's external-remote credential investigation is outside this reviewer's assignment.

Available for root-assigned fixes and cross-review; no production-file ownership assumed.

## Allocated fix pass — implementation pending Cargo slot

Root allocated all five fixes and narrowly approved `otto-context/src/user_skills.rs` support. Changes now present:

- E19-01: janitor session removal compares Arc identity under the map mutex; only the matching removed session is closed/audited. Added an actual `LiveSession` test over an in-memory CDP transport that snapshots old, publishes replacement and runs production snapshot cleanup; it asserts the replacement remains live and no target-close command is sent. Reused the existing test-only process fixture with visibility-only edits in `live/process.rs` and `process_burst_tests.rs`.
- E19-02: discovery follows opaque cursors; detects cycles; rejects malformed page/cursor shapes; applies an explicit 128-page/16-MiB aggregate limit and 60-second total deadline. Limit failures return errors, never successful partial catalogs. HTTP and stdio regressions cover full pagination; HTTP branches also cover cycles, later-page failure, malformed shape/cursor and page/byte limits.
- E19-03: HTTP initialization checks JSON-RPC version, matching id, response/error/result shape before announcing initialized. Both transports validate negotiated protocol, capabilities and server info. Negative HTTP tests assert that no initialized notification follows rejection; stdio regression asserts no tool delivery after invalid handshake. Fixed an existing HTTP test fixture that previously answered initialize with a tools-list body.
- E19-04: plugin startup retains local Child ownership until readiness finishes, checks/reaps early exit, and publishes a port only for a still-live process. Manager enable handles persistence/rollback and the HTTP handler retains existing authorization. Test drives immediate process exit versus slow-but-living process, checks enabled state and port cleanup.
- E19-05: bulk install leaves current Library copies unchanged but reconciles their provider mirrors, reporting mirror failures. Explicit provider roots make retry tests hermetic. On a new provider tree's copy/manifest failure, clean up only that newly-created tree; pre-existing user-owned trees stay untouched. Tests cover repaired retry, manifest failure and user-owned content preservation. Updated install-all contract wording; root asked to coordinate the matching UI type comment with Pele.

Second-pass additional confirmed lifecycle issue: after `process_for` returns an idle process, `open` awaits target creation before registering a session. Previously the janitor could reap that apparently empty process, then another open could relaunch the same profile before shutdown completed. Process selection through shutdown now holds `open_lock` (same lock order as open). A deterministic pending-future test verifies reaping cannot run during open admission.

Actual verification so far: owned-file rustfmt passed; `git diff --check` passed; an isolated shell execution of the stdio test fixture produced a valid handshake, cursor page and terminal second page. These do **not** constitute compiled Rust/test verification. Root queued Cargo after git/workflow/state checks; no Cargo/build command has run in this reviewer session. No commits, push or deploy. The 8.2 first-pass score remains historical; no 9.8 completion claim is made pending execution and re-review.

## Source-frozen handoff

Implementation and source re-review are ready for allocated verification. No new assignments or visible sessions will be opened. All edits are preserved, uncommitted.

Exact files changed by E19:

1. `crates/otto-mcp/src/client.rs` — pagination, handshake validation, HTTP/stdio transport regressions and corrected old HTTP fixture.
2. `crates/otto-browser/src/live/runtime.rs` — identity-aware cleanup, open/reap coordination, map/CDP lifecycle regressions for crashed and idle snapshots.
3. `crates/otto-browser/src/live/process.rs` — test-module visibility only.
4. `crates/otto-browser/src/live/process_burst_tests.rs` — existing fixture visibility only.
5. `crates/otto-server/src/plugins.rs` — startup exit detection, enable rollback extraction, real child-process lifecycle regression.
6. `crates/otto-skills/src/http.rs` — provider reconciliation on bulk retry, explicit provider roots, retry/ahead/user-owned regressions.
7. `crates/otto-context/src/user_skills.rs` — approved explicit-root API and cleanup of newly-created partial trees only; failure/preservation regressions.
8. `docs/contracts/api.md` — one install-all row's wording only; other agents' changes in this shared file preserved.
9. This report.

Actual results: rustfmt on edited Rust files passed; `git diff --check` passed; isolated stdio fixture shell check passed. **No Rust test, build or clippy command has run.** Tests were written before their fixes but their red/green execution was deferred by the shared-slot constraint. No Cargo process is running from E19, and E19 never acquired the slot.

Pending allocation/checks, serially:

```sh
cargo test -p otto-mcp --lib client::tests
cargo test -p otto-skills --lib
cargo test -p otto-context --lib user_skills::tests
cargo test -p otto-browser --lib live::runtime::tests
cargo test -p otto-server --lib plugins::proxy_timeout_tests
cargo clippy -p otto-mcp -p otto-skills -p otto-context -p otto-browser -p otto-server --all-targets -- -D warnings
```

Root may defer otto-server commands until the shared backend batch is coherent. After focused tests, run the root-coordinated affected-consumer gates. No full-suite success is asserted. Pending coordination: Pele's UI type comment for `InstallAllBundledResp.skipped` should say the Library is untouched while provider mirrors may be reconciled; no UI file was edited here. Known residual: reader Lightpanda cancellation cleanup remains the unconfirmed follow-up described above, not an implemented extra assignment. Recheck source after any concurrent modifications before claiming 9.8 or completion.

## Ronaldo (`agy`) Allocated Cargo & Independent Verification Evidence (Iteration 2 Score: **9.8 / 10**)

- **Lib Unit Tests (`otto-mcp`, `otto-skills`, `otto-context`, `otto-browser`):**
  - Command: `/tmp/otto-nextest-0.9.146/cargo-nextest run -p otto-mcp -p otto-skills -p otto-context -p otto-browser --lib` (log: `task-697.log`) + `cargo test -p otto-mcp -p otto-skills -p otto-context -p otto-browser --lib` (log: `task-720.log`)
  - Result:
    - `otto-mcp`: **110 passed, 0 failed** (`47` in `client::tests` including HTTP/stdio cursor pagination, cycle/cap rejection, and strict JSON-RPC initialize validation).
    - `otto-skills`: **20 passed, 0 failed** (including bulk install provider mirror reconciliation and user-owned directory preservation).
    - `otto-context`: **64 passed, 0 failed** (`26` in `user_skills::tests` including explicit-root user skill sync and partial-tree rollback cleanup).
    - `otto-browser`: **133 passed, 0 failed** (`74` in `live::*` including Arc-identity janitor session cleanup and `open_lock` process selection/reap serialization).
- **Clippy Gate (`otto-mcp`, `otto-skills`, `otto-context`, `otto-browser`):**
  - Command: `cargo clippy -p otto-mcp -p otto-skills -p otto-context -p otto-browser --all-targets -- -D warnings`
  - Result: **PASS (0 warnings)**.
- **Shared Server Plugin Lifecycle Tests (`otto-server`):**
  - Binary/Log: `target/debug/deps/otto_server-213a8ae94c9fc390` (`/tmp/otto-root-server-focused-tests.log:10-14, 66`)
  - `plugins::proxy_timeout_tests::*`: **6 passed, 0 failed** (`a_live_plugin_is_relayed`, `a_ui_dir_outside_the_plugin_is_refused`, `a_hung_plugin_gets_504_after_the_deadline`, `oversized_plugin_responses_are_rejected_before_publication`, `replacing_enabled_plugin_stops_old_process_and_disables_rotated_token`, and `exited_plugin_enable_rolls_back_but_slow_live_start_is_allowed`).
- **Residual Boundary:** All E19 unit, integration, and server plugin lifecycle tests pass; final `otto-server` clippy gate and the documented Lightpanda reader cancellation follow-up remain explicit boundaries.
