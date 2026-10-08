# R05 — API client

Reviewed the full API-client domain at merged baseline `a0bd718b9fbc008d72c164ce643a24ed78e6368c`, with repairs on shared branch `review/quality-20261008`. These are existing-domain findings, not an assertion that PR #94 introduced them. Other reviewers' concurrent changes are excluded except where consumer compilation exercised them.

## Verdict and four scores

The reproduced defects below are repaired. This domain is not demonstrated to meet 9.8. Scores are evidence-based estimates after the repairs, not statistical measurements.

| Vertical | Score /10 | Evidence and ceiling |
|---|---:|---|
| Correctness | 9.0 | Reproduced ownership races, script replay divergence, invalid GraphQL acceptance and a tonic readiness panic; focused regressions pass. Live OAuth providers, gRPC TLS and native WebKit remain untested in this pass. |
| Performance | 9.2 | Existing 3,000-request and 500-step browser gates pass with measured frame/input costs. Reflection now has cumulative byte/message/count/time bounds. No multi-client WAN or native-memory stress measurement. |
| Architecture | 8.7 | Engines separated from Axum; bounded caches and request/tab ownership are explicit. Three request preparation paths and substantial component-local asynchronous state still impose concrete drift risk. |
| Design / UX | 9.0 | Real WS connect/send/switch disconnect behavior tested, delayed file upload no longer damages another tab; light/dark desktop and phone states captured. Native shortcuts, keyboard-only and assistive technology not certified. |

## Reproduced findings and repairs

### R05-01 — Major correctness / UX: stream connection belonged to the workspace, not its request tab — fixed

Locations: `ui/src/lib/stores/apiStream.svelte.ts:43,154`; `ui/src/modules/api/RequestBuilder.svelte:572` (line numbers may shift with concurrent work).

Trigger: connect request A to a WebSocket, switch to request B in the same workspace, then send from B's composer. Previously the singleton stream retained A's upstream connection; B showed/sent A's traffic. The stream now retains both workspace and tab ownership. Selecting another owner closes the relay and clears its visible messages/status; the new tab must connect explicitly. Retry preserves the same owner. This intentionally supports one active stream at a time.

Evidence: store regression and a real local WebSocket server driven through the UI. The browser proves A receives and sends, changing to B closes the upstream, removes A's message, disables the composer and enables Connect. This is not a mocked transport assertion.

### R05-02 — Major correctness: asynchronous imports/schema loads published into a different owner — fixed

Locations: `ui/src/lib/stores/apiClient.svelte.ts:1044,1527,1541,1579`; `ui/src/modules/api/RequestBuilder.svelte:278,483,519`; `ui/src/modules/api/ImportDialog.svelte`.

Triggers: start curl parsing or collection file reading in workspace A then switch to B; start GraphQL introspection then change tab, URL or environment; load a schema in A then show another GraphQL tab; start reading a proto/multipart file then change target draft/row. Previously the callback used whichever draft/workspace was current at completion. Loaded GraphQL schema also lacked an owner and remained visible in unrelated tabs. Server reflection had the analogous same-tab URL-edit race.

Repairs capture workspace/draft identity before awaits; imports delegate through a workspace-owned store method; GraphQL schema is tagged by workspace/tab/URL/auth/environment and hidden when that context differs. File readers require the same draft/row identity before applying data. Reflection/describe ignore results and errors once their draft changes.

Evidence: actual-store VM regressions reproduce curl and GraphQL stale publication and the loaded-schema leak; delayed file tests cover workspace changes. A browser-level delayed `File.text()` test originally returned A's proto from B's draft, then passed after the repair. The multipart path uses the same guard but does not have a separate delayed FileReader browser regression. The final gRPC endpoint-edit browser check is recorded below.

### R05-03 — Major correctness: automation polling crossed workspace boundaries — fixed

Location: `ui/src/lib/stores/apiClient.svelte.ts:1826`.

Trigger: start an automation, change workspace before the start response or a delta response arrives. The original callback installed A's run into B and kept polling. A captured workspace epoch plus monotonic run generation now gates start, delta, errors and final publication. Superseded polling wakes and exits. This does not cancel an already-authorized daemon run merely because the user navigates away.

Evidence: actual-store test failed with `run-a` installed in workspace B before the guard, then passed. Existing delta/progress handling tests remain green.

### R05-04 — Major performance / correctness: reflection had unbounded streaming wait and aggregate work — fixed

Locations: `crates/otto-apiclient/src/grpc.rs:786,802,868,918`.

The two reflection streams could wait forever or append an arbitrary number of service names/descriptors; a per-message decoder cap did not bound the stream total. For M messages of B bytes, cumulative decode/retention work was O(M×B), with unlimited duration. A stream error was swallowed by `while let Ok(Some(...))`, permitting incomplete results.

Reflection now has a total 20-second deadline covering connection and both RPCs; counts every response (including irrelevant/duplicate messages) against 2,048 messages / 8 MiB encoded bytes; deduplicates service names and caps unique services at 1,024 and descriptor files at 4,096. Errors propagate. The invoke deadline is established before preparation and reused for channel readiness and the actual call, so reflection does not start a fresh 60-second invocation budget. The 8 MiB wire budget does not claim that decoded heap usage is exactly 8 MiB.

Evidence: real local HTTP/2 peer fixtures hold the stream open, send 1,025 services, send 2,049 messages and exceed 8 MiB with bounded fixtures. Before repair the stall hit the test's outer 22-second deadline and caps were not enforced; after repair all pass. No multi-GB allocation was attempted.

### R05-05 — Major correctness: healthy reflection panicked before its second RPC — fixed

Location: `crates/otto-apiclient/src/grpc.rs:913`.

After the service-list RPC consumed tonic's readiness reservation, the descriptor RPC reused `Grpc` without calling `ready()`. The real h2 service-list fixture triggered tonic's `buffer full; poll_ready must be called first` panic. The second RPC now acquires readiness again. A successful two-response fixture produces the sample descriptor pool; removing just the readiness repair made that test panic again. This explicit mutation check shows the regression exercises the production boundary.

### R05-06 — Major correctness: server replay undid variable deletion and silently accepted bad GraphQL variables — fixed

Locations: `crates/otto-server/src/routes/api_client.rs:2013,2238,3656,3856,3941` and shared `parse_graphql_variables` immediately below.

`merge_string_vars` inserted script output keys without removing absent input keys, so `pm.environment.unset('token')` left the old token available to later automation steps. The returned script variables are a full snapshot; reconciliation now preserves deletions. A failed post-script does not publish its partial/empty variable snapshot.

Saved and automation execution parsed variables with `.ok().unwrap_or({})`, silently replacing malformed JSON and accepting arrays/null/scalars. A shared parser now accepts blank/absent as `{}` and requires a JSON object otherwise, before scripts or outbound execution. Saved execution returns an input error; automation records a failed step.

Evidence: actual script engine unset test failed with the old token still present; saved/automation fixture failed because malformed JSON reached SSRF/network validation instead of variable validation. Both pass after repair; the handler test covers malformed JSON, array, null and scalar inputs. Forty API handler tests pass. The existing inline test module was moved intact to `crates/otto-server/tests/unit/api_client.rs`, retaining private-module access and names; no LOC ratchet was raised.

## Architecture finding remaining

### R05-07 — Minor: duplicated request preparation keeps execution modes vulnerable to drift

Locations: interactive `ui/src/lib/stores/apiClient.svelte.ts:1430`; saved `api_client.rs:1990`; automation `api_client.rs:3654`; transport orchestration in `RequestBuilder.svelte:498`.

The original replay defects are concrete evidence of the cost: validation and script semantics changed independently across interactive, saved and automation execution. The current repair centralizes the two Rust GraphQL parsers and fixes variable snapshot reconciliation, but does not unify all preparation. Future work should extract a small prepared-request pipeline for the two server paths and share conformance vectors with the browser runner. Preserve actor-specific secret binding, cookie jars and confirmation semantics; do not build a generic transport framework or move all UI execution into one monolith. Component-local async work should use one consistent owner token rather than accumulating ad hoc checks.

## Coverage and cost review

This is a risk-oriented domain review, not a claim to have hand-traced every line.

- HTTP execution: interactive per-tab slot/controller snapshots; saved execution and automation request/env/script/auth preparation; unsafe-method/new-host confirmation; pinned outbound validation; actor-scoped cookie jars; bounded response reads and secret-scrubbed history. Raw response cache has 10-minute lifetime and 128 MiB aggregate budget with user/workspace checks and janitor; UI retains at most 20 idle response slots and 64 full saved-request cache entries.
- Secrets/OAuth: opaque markers, host binding, saved-request secret adoption, authorization callback PKCE/one-use flow, bounded flow store and expiry, actor reauthorization and request fingerprint/revision checks before persistence. Callback token fetch has pinned network checks, no redirect following and bounded response. No live provider credentials used.
- gRPC: proto compilation/descriptor caches, endpoint normalization, pinned addresses/TLS channel cache, reflection's two-RPC lifecycle, unary/server-stream truncation and metadata. Cache entries are bounded; source-reviewed client-streaming rejection.
- Scripts: isolated server child, bounded concurrency/timeout/output/memory controls and environment clearing; browser worker execution limits and cleanup. Tests cover syntax/runaway scripts and request/response mutation.
- SSE/WS: incremental frame scanning, bounded frame buffering, close/cancel/retry lifecycle; browser event ring and batch publication; response console windowing. New browser evidence specifically exercises WebSocket, not a live SSE upstream.
- UI/persistence: API page/editor/sidebar, summary-only tree and demand loading, per-workspace tabs, dirty/save/import boundaries, history snapshots/deltas and search, GraphQL variables, response rendering, automation editor/progress, environment editing and secret rows, cookie/storage views. Broad scale fixtures exercise real rendering and API fetch behavior.

## Executed verification and performance

Environment: macOS shared workspace, Node 22.22.3; isolated Playwright desktop Chromium slot `r05-review`, API7815/UI5195; throwaway daemon data and file secrets, `OTTO_E2E_SWEEP_ORPHANS=0`. No live port 7700 or user profiles touched. Browser daemon was the parent's rebuilt debug binary with earlier R01/R02 repairs; final Rust API fixes are verified through tests, not a rebuilt browser daemon in this pass.

- `node --test unit/apiClientOwnership.test.ts unit/apiStream.test.ts unit/apiHistory.test.ts unit/scriptRuntime.test.ts unit/scriptRunner.test.ts unit/apiGraphqlVars.test.ts`: **75 passed**, 0 skipped, 14.75s. VM tests execute the actual transpiled store; they are not proof of Svelte rendering by themselves.
- `cargo test -p otto-apiclient --lib -- --test-threads=1`: **25 passed**, 20.50s. Serial execution avoids the existing global channel-dial counter interfering with parallel fixtures.
- `cargo test -p otto-server --lib routes::api_client::tests -- --test-threads=1`: **40 passed**, 1.69s; compilation was separate from test time. The original two new tests were observed failing before repair.
- Named browser `desktop-api-review-ownership.spec.ts` + `desktop-api-scale-perf.spec.ts`: **12 passed** in 30.5s before the proto regression; final ownership rerun: **2 passed** in 6.9s, including proto red→green. The added endpoint-edit case and final shared checks are appended below.
- 3,000 saved requests: windowed tree; search frame p50/p95 16.7ms, maximum 16.8ms across 44  samples, no long-task entries. URL handler 51 samples: p50 0.8ms, p95 1.1ms, max1.3ms; including next frame p95 1.7ms.
- 200KB minified JSON body 27 samples: handler p50 3ms, p95 5.6ms, max11.8ms; including frame p95 6.6ms, max13ms. These are this machine/run measurements, not production guarantees.
- 20 automation steps over 3,000 requests remain under 3,000 DOM nodes. A 500-step run made 6 delta fetches over 1,234ms; final completion displayed in 58ms. Cached page reentry made no redundant refetch; opening a request fetched only that full row; response editor identity and splitter single-persist gates pass. No LSP socket opened for API editors.

Durable evidence: `../evidence/R05/` contains red/green Rust/Node/browser logs and `stream-loaded-light.png`, `stream-empty-dark.png`, `stream-empty-phone.png`. Final screenshots show 1440×900 desktop and 390×844 phone; dark/phone images were inspected. Phone no-horizontal-overflow assertion passed. The screenshot scope is WS loaded/disconnected state, not every API panel. Disabled header buttons in the dark screenshot remain visually pale; this is shared-chrome debt for the integrated design review, not a new API style change.

## Remaining verification limits

No native Tauri/WebKit, live OAuth authorization or refresh provider, HTTP proxy/SSH tunnel, gRPC TLS server, live SSE reconnection, screen reader, keyboard-only flow, RTL/tablet sweep or concurrent-client throughput test was executed here. Performance E2E uses deterministic synthetic request collections and mocked automation completion; it does not measure remote service latency or daemon throughput. Existing suite names and source caps are not substituted for those measurements. Shared CI-equivalent checks and fresh daemon integration remain the root reviewer's gate. No commit, push, deployment or outward publication performed.

## Final handoff checks

- Added gRPC endpoint-edit browser case: **1 passed**, 3.7s case time. It holds reflection completion, edits the endpoint, then verifies no old service/method is adopted. Browser lease released to R07.
- Parent-requested `cargo test -p otto-server --lib state_archive::`: **8 passed**, 2.94s, after the final Rust API changes compiled. Cargo lease released to R04.
- `npm run check`: UI guards, Svelte and all TypeScript projects pass (0 errors/warnings). A repeat after the final two component ownership guards is pending at handoff unless appended below.
- Focused `rustfmt --check` and `git diff --check` pass.
- Root-owned remaining integration gates: server/apiclient clippy, `it` route inventory/policy coverage, fresh daemon build and overall shared branch verification.
