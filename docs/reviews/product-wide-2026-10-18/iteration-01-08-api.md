# E08-api — iteration 01

Verdict: **Approve with fixes; below the 9.8 target. Seven major findings, no claimed blocker.** Scope score **7.4/10** (correctness 7.0, test confidence 7.0, architecture 8.0, performance 8.5). These are source-review judgments, not measured production scores.

## Provenance and method

- Reviewed 2026-10-18 on `fix/db-parity-session-consistency`, observed HEAD `c009da20cc113c8fb79f3bf0600442d0c6afa358`.
- Whole-subsystem review rather than changed-lines-only review. Findings may predate PR111.
- Read repository AGENTS.md and correctness-review, test-review, architecture-review, performance-review skills. All findings below are **confirmed by hand trace**, not executable reproduction.
- No builds, Cargo, tests, commits, git mutations, production changes, or real remote requests. Only this report was written. Other agents' live edits were left alone; the principal files cited here were clean in the scoped status check during review.
- Early confirmed findings were delivered with ID E08-api to root session `01M57PYK75FC1F2AFS4C73BBF7` using `otto_send_message`.

## Findings

### E08-01 — Major: form editor percent-encodes placeholders before substitution

Locations: `ui/src/modules/api/RequestBuilder.svelte:250-258`; `crates/otto-server/src/routes/api_client.rs:3060-3068,4312-4314`.

Input: active environment `token=abc`; choose Form (URL-encoded), add `token` with value `{{token}}`, Send.

Trace: `encodeForm` calls `encodeURIComponent` on the value, producing `token=%7B%7Btoken%7D%7D`. `prepare_request`'s form branch substitutes the encoded string; `substitute_once` returns unchanged because it has no literal `{{`. The server receives a form value of **`{{token}}`**, not `abc`. This affects ordinary environment/session-driven form authentication and payloads, not only unusual input.

Fix direction: decode form rows, substitute keys/values, then encode once for the wire (or preserve structured form rows through execution). Preserve literal percent, plus, ampersand, and repeated keys.

Required regression: editor-to-echo-server form send with `{{token}}`, `abc+&=%` as its value; assert the decoded upstream value exactly equals `abc+&=%`. A substitution-helper test alone misses the encoding boundary.

### E08-02 — Major: multipart substitution corrupts its JSON container

Locations: `crates/otto-server/src/routes/api_client.rs:3075-3078,2837-2879`; UI producer `ui/src/modules/api/RequestBuilder.svelte:250-256`.

Input: environment `note=a"b`; multipart text field `note={{note}}`. The UI correctly serializes `[{'key':'note','type':'text','value':'{{note}}'}]` as real JSON with double quotes.

Trace: `prepare_request` substitutes the entire JSON string before `build_multipart` deserializes it. Its value becomes `"value":"a"b"`, invalid JSON. Rather than report the parse failure, `build_multipart` falls through to legacy `k=v&...` parsing. For this example there is no `=`, so it sends an empty-valued field whose name is the entire malformed JSON. Newlines/backslashes can similarly invalidate or change text. The inner per-field substitution already exists but is reached too late.

Fix direction: pass the original multipart container to the parser, substitute parsed text keys/values/filename as appropriate, and reject malformed JSON-shaped input rather than reinterpret it as a legacy form. Keep file base64 separate from text substitution.

Required regression: echo multipart text values containing quotes, newline, backslash, and ampersand via variables; assert exact field names/values and unchanged file bytes. Assert malformed JSON returns a clear error without sending.

### E08-03 — Major: interactive scripts cannot read the active environment

Locations: `ui/src/lib/stores/apiClient.svelte.ts:1431-1453,1498-1502`; `ui/src/lib/api/scripts.ts:76-90,160-166`; contrast `crates/otto-server/src/routes/api_client.rs:2024-2051,3680-3693`.

Input: plain active environment `tenant=abc`, no session variables, pre-script `pm.request.headers.upsert({key:'X-Tenant',value:pm.environment.get('tenant') || 'missing'})`.

Trace: interactive execution snapshots only `this.runtimeVars` into the Worker. `pm.environment` reads that map, so the sent header is `missing`. Saved-request execution resolves the environment first and passes the merged map into the script, producing `abc`. Post-response script reads have the same discrepancy. This is a plain, non-secret variable case; exposing Keychain secrets to the browser is unnecessary to demonstrate or fix it.

Fix direction: establish consistent script variable scope; seed interactive reads from the captured plain environment plus runtime overrides, publish only explicit writes back to runtime scope, and document/handle secret access separately. Preserve execution ownership and environment snapshot rules already present.

Required regression: same saved request/script and plain environment through interactive Send and saved execution; assert identical header values and post-script reads, with runtime override precedence and no copying of every environment key into runtime state. Existing ownership test `ui/unit/apiClientOwnership.test.ts:123-136` only checks dispatched environment ID, not script input values.

### E08-04 — Major: gRPC ignores the shared environment-variable surface

Locations: `ui/src/modules/api/RequestBuilder.svelte:534,562-565,810-824`; `crates/otto-server/src/routes/grpc.rs:35-61`; `crates/otto-apiclient/src/grpc.rs:334-347,431-460,495-506,707-708`.

Input: select environment with `base_url=https://grpc.example.com:443` and `token=abc`; use `{{base_url}}`, metadata `authorization: Bearer {{token}}`, or a string message field containing `{{tenant}}`.

Trace: the common builder shows variable resolution guidance, but gRPC Invoke and Reflection send raw draft values without environment ID/runtime overrides. The thin routes forward them directly to the gRPC engine; the request type has no environment fields. URI parsing rejects the placeholder URL; with a literal URL the metadata/body placeholder remains literal. HTTP's substitution code is never involved.

Fix direction: resolve variables server-side for both reflection and invoke using the same workspace environment semantics, including existing secret-host rules when stored secrets are used. Update request contracts and caller types together. Do not remove privileged/local endpoint support.

Required regression: real local reflection/unary fixture with environment URL, metadata and message variables; assert resolved upstream values and runtime override precedence. Include a missing variable case with an explicit outcome. Proto persistence coverage (`ui/unit/apiClientOwnership.test.ts:69-76`) does not exercise execution substitution.

### E08-05 — Major: history replay silently uploads an empty file after stripping its bytes

Locations: `crates/otto-server/src/routes/api_client.rs:1775-1810,2850-2856`; `ui/src/lib/stores/apiClient.svelte.ts:2037-2080`; `ui/src/modules/api/RequestBuilder.svelte:237-256`.

Input: send multipart file `hello.txt` containing `hello` (base64 `aGVsbG8=`), then open its history row and Send the restored request.

Trace: history capping deliberately replaces the file value with `""`, retaining the filename. It does not set `cut` for this removal, so a small request has no `request_truncated` flag. The history loader uses the stripped body directly, without an incomplete-body/file warning or restoration. The multipart sender successfully decodes empty base64 and uploads **zero bytes under `hello.txt`**. Saved-request lookup only helps credential restoration, not file content. The file chooser displays the retained filename.

Fix direction: represent stripped file content explicitly as unavailable; tell the replay user to reselect it and prevent accidental send until resolved. Do not retain all binary uploads indefinitely merely to fix replay. Also consume `request_truncated` for ordinary capped bodies: the loader currently ignores that flag too.

Required regression: send/store/load/replay multipart fixture; assert an incomplete-content state and no outbound send until a file is selected. Separately load a >64 KiB truncated request body and assert it is visibly incomplete. Existing masked-credential history test at `ui/unit/apiClientOwnership.test.ts:227-244` does not cover missing bodies.

### E08-06 — Major: overlapping gRPC calls publish stale results and cannot be stopped

Locations: `ui/src/modules/api/RequestBuilder.svelte:555-572,602-603,620-631,799-803`; `ui/src/lib/stores/apiClient.svelte.ts:603-608`.

Input: invoke slow call A, edit the message, press Enter in the URL or Cmd-Enter to invoke B; let B complete before A.

Trace: the button is disabled while `grpcInvoking`, but both shortcut handlers call `send()` without a busy check. `invokeGrpc` has no guard, AbortController, or execution-generation token. Both completions call `setTabResult` using only tab ID and workspace equality. A's later completion overwrites B's newer result. The first completion also resets the shared `grpcInvoking` boolean while the other call is still running. `canStop` excludes gRPC, and `stopRequest` does not cancel it. The server's 60-second deadline bounds execution but does not solve result ownership.

Fix direction: route gRPC through per-tab execution ownership comparable to HTTP, with a cancellation signal and generation check; gate shortcuts consistently or explicitly replace the old run. Use the long request lane for these potentially 60-second RPCs, as HTTP does.

Required regression: deferred A/B invoke responses completed in reverse order, asserting only B is shown; Stop, tab close, and workspace round trip must suppress obsolete publication. Check tab-specific busy state with two gRPC tabs. Existing proto/reflection ownership E2E checks do not cover overlapping Invoke.

### E08-07 — Major: automation history opens a different, empty request for replay

Locations: `crates/otto-server/src/routes/api_automation_runs.rs:207-213`; `ui/src/lib/stores/apiClient.svelte.ts:2037-2073`; existing credential rehydration helper is invoked at `2053-2056`.

Input: saved POST with JSON body `{"name":"alice"}`, a required header and bearer auth; execute it as an automation, then open that run's history entry in the editor.

Trace: the automation history writer stores only source/run/request IDs and dataset/step metadata in `request`; it does not store method, headers, body, body mode or auth. The history loader fetches the saved request when its ID is known, but uses it only to restore masked credentials. Missing snapshot auth defaults to `{type:'none'}` (no mask to refill), headers/query to `[]`, body mode to `none`, body to `''`. Method/URL fall back to history row fields. The resulting draft is POST to the original URL with **no original body, headers or auth**; Send executes that changed request. Dataset substitutions and pre-script changes are not reconstructible from this row either.

Fix direction: distinguish result-only automation history from complete replay snapshots. Offer an explicitly labeled open-saved-request action, or retain a bounded, redacted effective request snapshot and mark any missing data. Do not present current saved-request data as the exact past execution.

Required regression: automation POST → history → open; assert exact retained replay fields or an explicit non-replayable state. Include dataset variables/pre-script mutations and a subsequently edited/deleted saved request. The normal HTTP history test does not cover this alternate producer.

## Architecture and performance assessment

The gRPC/script leaf engines are appropriately separated from Axum auth/state, and HTTP request preparation is reused by interactive, saved and automation sends. The actionable architectural cost is execution-policy drift: gRPC bypasses the shared variable and per-tab execution policies, while interactive scripts independently assemble variable scope. Adding another execution option currently requires updating several paths with different semantics. Reuse small request-resolution and execution-ownership units across real callers; no broad engine rewrite is justified by this pass.

Reviewed bounds include HTTP download 25 MiB + one byte and 512 KiB displayed text, 64 KiB retained history body, gRPC 60-second execution and stream message/byte caps, descriptor/channel caches with count limits, script-child concurrency/deadline/memory limits, and history scalar summaries with bounded list limits. These are meaningful protections; I found no additional proven performance blocker. gRPC's interactive-lane routing is noted with E08-06; browser socket contention was not load-tested. History text search scans and cache sizes were inspected but not benchmarked.

## Coverage, test confidence, and remaining boundaries

Read/traced: HTTP request construction/settings/auth handoff, environment resolution/substitution, multipart/form body encoding, script Worker and server child execution, saved execution and automation step flow, history insertion/capping/list/detail/replay, gRPC describe/schema construction/invoke/reflection/channel path, per-tab response ownership and cancellation, automation cancellation select/drop path, relevant API/feature contracts, state history query shapes, and selected unit/E2E tests. Automation cancellation explicitly warns that an already dispatched remote side effect cannot be undone; that is correct and is not reported as a bug.

Existing tests provide useful coverage for HTTP tab ownership, canceled/stale pre-scripts, variable-write merging, credential masking on normal history replay, delayed proto uploads and reflection, script budgets, gRPC reflection caps/readiness, and channel reuse. They do not establish the seven cross-boundary cases above. Test execution was expressly prohibited; no pass/fail or coverage-percentage claim is made.

Residual/unreviewed boundaries: full OAuth/PKCE lifecycle, import/export and Git sync, secret-store implementations and all redaction branches, SSE/WebSocket relay internals, exhaustive protobuf well-known/oneof/binary-metadata compatibility, real TLS/SSH/proxy behavior, OS child-process cleanup under stress, HTTP disconnect propagation through Hyper/Axum, exhaustive state migrations, and pixel/accessibility checks. Root should allocate these to their respective reviewers or a follow-up pass. This report is not a full-product certification, and cannot justify a 9.8 score without fixes and executed regression evidence.

No E16 git credential claim is made: that special investigation belongs to E16, outside E08's scope. Available for coordinated fixes and cross-review after ownership/test allocation.

## Allocated fix pass — implementation and verification in progress

Root allocated all seven functional fixes to E08. Changes are local and uncommitted; no production/remote actions. API UI functional sections were announced to Pele, who owns presentation. E20 workspace DTO/contracts and the workflow API adapter are untouched.

- E08-01: form pairs decode → substitute → encode. Secret-host detection and automation missing-variable detection now inspect decoded form variables too, preserving policy when this previously broken substitution starts working.
- E08-02: multipart containers parse before field substitution. Invalid containers/base64 fail explicitly. File bytes do not undergo text substitution.
- E08-03: interactive scripts read captured plain environment + runtime values, while only explicit writes become runtime overrides. Keychain values remain daemon-side.
- E08-04: additive route wrappers carry environment/runtime/confirmation options to Invoke and Reflection. Shared workspace environment and host-binding helpers are used; JSON string values substitute structurally. Engine remains transport-only. Contracts and TS DTOs updated.
- E08-05: history marks stripped files `missing_content:true` and truncated; exact decoded byte size accounts for base64 padding. UI preserves missing markers until reselection, blocks send/save of incomplete history, recognizes legacy stripped-file rows, and blocks truncated multipart containers. Actual selected empty files remain valid.
- E08-06: gRPC invokes use per-tab controllers, generation ownership, cancellation, long-lane requests and shared Stop. Old completions cannot overwrite replacements or publish after tab closure.
- E08-07: metadata-only automation entries cannot execute/save as empty requests. Explicit `openHistorySavedRequest()` opens the current saved definition without labeling it as a historical execution.

Observed verification so far: five new UI regressions failed against the old implementation (53 pass / 5 fail), then all 58 API ownership tests passed after fixes. Expanded API ownership/history/script runner/runtime selection passed **78 tests**. Unit TypeScript check passed from `ui/`; direct Svelte compilation of RequestBuilder returned zero warnings. Scoped diff whitespace check and owned Rust formatting passed. One repeat TypeScript command was launched from the repo root and failed to locate the UI tool; rerunning from `ui/` passed. Rust regressions are written, including unchanged HTTP 422 response semantics, but **not run yet**: root's shared Cargo slot has been requested and not granted. No Rust compile/test success is claimed.

Independent second-pass trace added protections for encoded-form secret binding/missing-variable validation, truncated multipart JSON, valid empty files, explicit current-saved opening, background gRPC tabs and tab closure. Presentation integration and Cargo verification remain required before declaring the allocated work complete or assigning a post-fix score.

## Final source-frozen handoff — E08

All seven allocated functional fixes are implemented locally. No new sessions or subagents were opened. The temporary stop instruction was superseded by the user's instruction to finish existing assignments; this handoff freezes the existing assignment for root/Ronaldo/Pele validation. No commits, stash, revert, push, deploy, or Cargo commands were performed. **Cargo slot was never acquired and none is held.**

Exact E08 files (other agents' edits in shared files must be preserved):

1. `crates/otto-apiclient/src/grpc.rs`: public transport request fields / serializable metadata for route-level environment resolution.
2. `crates/otto-server/src/routes/api_client.rs`: form decoding/substitution, multipart parsing and explicit invalid/missing-content errors, history file markers and correct sizes, shared substitution/conflict helpers, encoded-form secret/missing-var checks.
3. `crates/otto-server/src/routes/grpc.rs`: additive workspace wrappers, environment/override resolution, structural JSON value substitution, existing secret-host policy, agent response/error scrubbing, regression tests.
4. `crates/otto-server/tests/unit/api_client.rs`: six new regression tests for form wire encoding, multipart escaping/file bytes, malformed/missing content, history markers, encoded secret/missing-variable detection, and unchanged HTTP 422 response semantics.
5. `ui/src/lib/api/types.ts`: `GrpcReflectReq` and `GrpcInvokeReq` additive DTOs only. E20 owns workspace DTO changes elsewhere in this file.
6. `ui/src/lib/stores/apiClient.svelte.ts`: plain-environment script reads with explicit-write persistence, gRPC per-tab cancellation/generation ownership, incomplete history send/save guards, explicit current-saved opening; deleted automation origins never fall back to unrelated same-URL requests.
7. `ui/src/modules/api/RequestBuilder.svelte`: gRPC delegation/Stop/reflection options, preservation of missing-file markers until reselection, minimal existing-style incomplete-history notice and explicit saved-request action. Pele was notified before functional editing and before this minimal presentation addition; later visual styling belongs to Pele.
8. `ui/unit/apiClientOwnership.test.ts`: ten new behavior regressions; API test stub exposes the normal non-confirmation result for `newHostConfirmHost`.
9. `ui/unit/apiFormReplay.test.ts` (new): three tests exercising the actual component parse/encode functions, ensuring unrelated form edits cannot erase missing-file markers and selected empty files stay valid.
10. `docs/contracts/api.md`: two API-client paragraphs describing additive gRPC options, substitution/policy, form and incomplete-history behavior. E20 workspace and workflow API-node paragraphs are untouched by E08.
11. This report.

Final actual checks:

- `cd ui && node --test unit/apiClientOwnership.test.ts unit/apiFormReplay.test.ts unit/scriptRuntime.test.ts unit/scriptRunner.test.ts unit/apiHistory.test.ts`: **83 passed, 0 failed**, final output `/tmp/e08-ui-complete.log`.
- `cd ui && node node_modules/typescript7/bin/tsc -p tsconfig.unit.json --pretty false`: **passed**, including after the last changes.
- Direct Svelte compiler invocation on `RequestBuilder.svelte`: **passed, zero warnings**, including the notice/action markup.
- `rustfmt --edition 2021 --check` on the four owned Rust files: **passed**.
- Scoped `git diff --check`: **passed**.
- An intermediate new gRPC error-detail test failed because Node VM errors stringify as `Error: connection refused`, unlike same-realm `Error.message`. The assertion now checks the preserved detail suffix, and the final 83-test run passes. No production behavior was weakened to satisfy it. A few commands used the wrong working directory and failed to locate their tools/tests; corrected invocations above passed.

Pending commands, only after root allocates the shared Cargo slot:

```sh
cargo test -p otto-server --lib routes::api_client::tests
cargo test -p otto-server --lib routes::grpc::tests
cargo test -p otto-apiclient
cargo clippy -p otto-server -p otto-apiclient --all-targets -- -D warnings
```

Root should also run the app type/UI guard gate (`cd ui && npm run check`) and the named existing API ownership/durability E2E specs when the isolated daemon/browser slot is available. No full E2E suite was requested or run. The Rust tests have not been compiled or executed; cross-agent compilation failures may need coordination. No post-fix score of 9.8 is asserted without those gates and independent cross-review. All edits are preserved for root/Ronaldo/Pele; no E08 process or test remains running.

## Ronaldo (`agy`) Allocated Cargo & Independent Verification Evidence

- **Standalone Package Unit Tests (`otto-apiclient`):**
  - Command: `cargo test -p otto-apiclient --lib` (log: `task-720.log:103-132`)
  - Result: **25 passed, 0 failed, 0 ignored** (`20.03s`).
- **Clippy Gate (`otto-apiclient`):**
  - Command: `cargo clippy -p otto-apiclient --all-targets -- -D warnings`
  - Result: **PASS (0 warnings)**.
- **Shared Server Route Tests (`otto-server`):**
  - Binary/Log: `target/debug/deps/otto_server-213a8ae94c9fc390` (`/tmp/otto-root-server-focused-tests.log:16-64`)
  - `routes::api_client::tests::*`: **46 passed, 0 failed** (including `form_variables_are_decoded_substituted_then_encoded`, `encoded_form_variables_keep_secret_binding_and_missing_variable_checks`, `multipart_substitutes_fields_without_corrupting_container_or_files`, `malformed_or_missing_multipart_content_is_not_sent`, `history_marks_stripped_file_content_even_for_small_requests`, `http_non_success_status_remains_a_response_not_an_execution_error`, `script_variable_unset_survives_server_replay`, `invalid_graphql_variables_fail_saved_and_automation_replay`, `an_invalid_tunnel_proxy_fails_instead_of_going_direct`, `oauth_secrets_bind_to_the_saved_token_endpoint`).
  - `routes::grpc::tests::*`: **3 passed, 0 failed** (`grpc_resolves_workspace_environment_and_enforces_secret_host_binding`, `message_variables_preserve_json_escaping_and_nested_types`, `wrapper_accepts_legacy_requests_and_workspace_resolution_options`).
- **UI Unit, TypeScript & Full Check Verification:**
  - Command: `cd ui && node --test unit/apiClientOwnership.test.ts unit/apiFormReplay.test.ts unit/apiHistoryBanner.test.ts unit/scriptRuntime.test.ts unit/scriptRunner.test.ts unit/apiHistory.test.ts && npx tsc -p tsconfig.unit.json --noEmit`
  - Result: **PASS** — `ui/unit/apiHistoryBanner.test.ts` now invokes `compile` from `'svelte/compiler'` directly on `ui/src/modules/api/RequestBuilder.svelte` (`generate: 'client'`), asserts `compiled.warnings.length === 0` and emitted client code for `incomplete-history` / `replayIssue()` / `openHistorySavedRequest()`, and verifies `svelte/compiler` throws on invalid `{#if ... as issue}` syntax.
  - Full UI check & full UI unit suite: **PASS (`0` Svelte errors/warnings)**.
- **Workspace Nextest & Consumer Clippy Gate:**
  - `scripts/check.sh --all --check --no-ui` (`/tmp/otto-product-wide-workspace-gate-r2.log`): **5973 passed, 0 failed, 124 skipped** (`246.796s`).
  - Final consumer clippy (`/tmp/otto-product-wide-final-consumer-clippy.log`): **PASS (0 warnings)** across `otto-apiclient`, `otto-server`, and `ottod`.
- **Post-Verification Score:** **9.8 / 10**.
