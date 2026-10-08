# R04 — Connections, database engines and workbench

Status: scoped review and five repairs verified, including the Redis transport follow-up. Reviewed merge snapshot `a0bd718b9fbc008d72c164ce643a24ed78e6368c` on shared branch `review/quality-20261008`. Findings below existed at that snapshot; this pass has not attributed their original introduction to PR #94. Other reviewers’ concurrent changes are outside this verdict unless explicitly named.

## Verdict and scores

Approve the verified fixes; do not claim the domain meets 9.8. Cell truncation provenance now passes native decoder tests and the real Mongo browser flow. Redis console and metadata replies now have a shared pre-parser receive budget; protocol-peer tests exercise limits, recovery and TLS configuration.

| Vertical | Evidence-based score /10 | Evidence and ceiling |
|---|---:|---|
| Correctness | 9.2 | Mongo write safety exercised in the real workbench; Redis pipeline limits, active/queued cancellation, cache retirement/reconnect, no write replay and DB isolation exercised with isolated protocol peers. Native failure coverage remains sampled. |
| Performance | 9.0 | Sparse JSON import and Redis pre-display allocation amplification repaired. Per-operation byte/node/depth limits include pipeline replies; no process-wide RSS or remote-latency throughput measurements. |
| Design | 8.9 | Driver/service/HTTP and engine edit adapters are coherent seams. RESP framing, transport ownership and TLS have separate modules; source/result attribution still relies on separate client/server parsers. |
| UX | 8.8 | Real cell edit → pending → review → write → refresh, projected edit, malformed JSON/discard, computed-identity refusal and lossy-result refusal/recovery pass. Desktop light/dark states inspected; keyboard-only, screen-reader, mobile and SSH failure journeys not certified. |

Scores describe the inspected evidence, not percentages of defect-free code. They reflect the stated coverage limits and are not inferred from green test counts.

## Confirmed findings and repairs

### R04-01 — Blocker, correctness: computed Mongo identities became write targets — FIXED

Location: `ui/src/modules/database/edit-mongo.ts:68`; executor seam `crates/otto-dbviewer/src/drivers/mongodb.rs:1019`.

The original edit-target detector accepted the prefix `db.users.find(` regardless of the remaining chain/projection. A query such as `db.users.find({}, {_id: {$literal: "other-user"}, name: 1})` returns a computed `_id`, which the Mongo adapter then used for `updateOne`, `deleteMany` and `replaceOne`. With another real document using that identifier, the edit could target that document. A later aggregate call was likewise not excluded by the prefix match. The backend forwards the parsed projection into the real find operation.

Repair: require a complete find call and an allowed sort/limit/direct projection chain; computed, nested/positional projections, explain, second operations, aliases and multi-statements stay read-only. Normal unprojected find, top-level scalar inclusion/exclusion projections, ObjectId filters and direct SQL SELECT projections retain editability. The small parser deliberately refuses unknown JavaScript syntax instead of asserting provenance it cannot prove.

Evidence: Node regression observed failing on original production adapter; passes after repair. Actual Mongo browser run returned the computed `other-document` identity and verified the edit action was absent with an explanatory note. SQL-to-Mongo direct and aliased/computed SELECT controls run through the production SQL parser in the Node harness.

### R04-02 — Blocker, correctness: JSON edits replaced projected documents — FIXED

Location: `ui/src/modules/database/edit-mongo.ts:294`; callers `ui/src/modules/database/EditFlow.svelte.ts:958` and `DocEditor.svelte`.

The JSON editor's original `replaceOne` body was the displayed row. After a find/SELECT projection, fields excluded from the result were therefore deleted from the stored document on Save. This was not an explicit request to delete those undisplayed fields.

Repair: the document editor generates one `updateOne` with changed/new displayed top-level fields in `$set` and explicitly removed displayed fields in `$unset`. It leaves all undisplayed fields alone, excludes `_id`, and does not create a mutation for an unchanged document. Nested/positional projections remain read-only because replacing a partially projected object could still delete its unseen children. Review and helper copy now describe updates to displayed fields.

Evidence: Node regression failed with original `replaceOne` output and passes with exact `$set`/`$unset` statement assertions. Real browser fixture queried only `{status:1}`, changed status to paid, then queried the complete document and verified its omitted `items`/`qty` fields still existed. The existing complex-cell edit and malformed-JSON/discard flows also passed.

### R04-03 — Major, performance: sparse JSON import expanded a small file into a huge matrix — FIXED

Location: `crates/otto-dbviewer/src/import.rs:72`; reachable through `service.rs:2849` (`import_from_path`).

Input size is N records and K distinct top-level keys across those records. The original union lookup was linear for every field, followed by N×K `Value` slots filled mostly with NULL. The 100 MiB input-file bound did not constrain this expansion: 10,000 one-field records with different keys use a small input but produce 100 million slots (about 3.2 GB of slot storage at 32 bytes, excluding strings/maps). This is a root-authorized local import path, not an unauthenticated endpoint.

Repair: hash membership preserves first-seen column ordering without repeated linear scans; check the fixed row count against each growing union before allocating the dense matrix. A 2,000,000-expanded-cell ceiling returns an actionable split/reduce-fields error. Consume object maps instead of cloning them again. This ceiling bounds the dense JSON/NDJSON matrix near 64 MiB of `Value` slots; it is not a claim that all import memory is 64 MiB or that CSV has been made streaming.

Evidence: a 1,500-record/<100 KB fixture originally allocated 2.25 million cells and failed the rejection assertion; after repair it is rejected during union construction. Small sparse rows still preserve first-seen order and NULL padding. All 16 import tests pass. No multi-GB allocation was attempted. The before test group took 0.06s and after group rounded to 0.00s, which is too small/coarse for a general throughput claim.

### R04-04 — Major, correctness: shortened cells lacked write provenance — FIXED

Locations: `crates/otto-dbviewer/src/types.rs` (`cap_cell`, `QueryResult`); MySQL/Postgres decode chunks; Mongo `docs_to_result`; Redis `bounded_reply_to_result`; ClickHouse streaming `CompactStream`; `ui/src/modules/database/EditFlow.svelte.ts:188`.

Strings larger than 1,048,576 characters are recursively shortened into ordinary display strings. Previously no result metadata distinguished such a value from complete stored content. Editing a sibling in a JSON object or writing back a displayed document could persist the shortened value. Row pagination alone is different: it can retain complete rows and must remain editable.

Repair: optional `cells_truncated` flag is set by the actual capping operation, carried across asynchronous decoding and ClickHouse prepared-stream results, and mirrored in TypeScript/contracts. The common edit target and Copy-as-INSERT guards refuse lossy results with an inline explanation. No sentinel string matching is used: a real stored string resembling a truncation suffix stays ordinary data. A narrow query returning complete short fields can still be edited.

Direct production EditFlow regression passes: a shortened-cell result clears a stale edit target; row-pagination-only results and actual marker-looking stored strings retain editability. Temporarily disabling only this guard makes that regression fail (`users !== null`); restoring it passes. Production Mongo/Redis/ClickHouse decoder regressions pass in the 461-test package run. The real-Mongo browser fixture with a >1 MiB nested bio plus sibling name passes: the wire flag is true, both cell and document edit actions are absent, a narrow complete status projection can be edited through review, and a native aggregate verifies the original full bio length and sibling name remain unchanged. The same new case fails on the prior daemon at the missing wire flag, so the integration assertion distinguishes the repair.

### R04-05 — Major, performance: Redis reply materialization preceded display caps — FIXED

Original location: `crates/otto-dbviewer/src/drivers/redis.rs:327` and `:349`. Repair: `drivers/redis/framing.rs:17`, `:62`, `:84`, `:168`; `drivers/redis/transport.rs:29`, `:89`, `:157`, `:252`; `drivers/redis/tls.rs:10`. Metadata error propagation: `drivers/redis.rs:283` and `:616`.

`query_async::<RedisValue>` formerly decoded the whole response before display caps, both for arbitrary console commands and collection metadata previews. N values of S bytes cost O(N×S) retained payload plus per-value allocations regardless of grid max_rows. A 1-million-item reply of 1 KB values is about 1 GB of payload alone. SCAN COUNT is a hint and a fixed preview element count does not bound individual value size. Installed redis 1.7.1 parser/codec had a depth limit but no received-byte/value-count bound; combine's 4,096-element initial allocation hint did not constrain the eventual reply.

Repair: a plaintext AsyncRead wrapper validates RESP framing before passing each at-most-8-KiB chunk to redis-rs. Every operation has a 32 MiB wire-byte ceiling, 100,000 value-node ceiling and 64 aggregate-level ceiling. Declared bulk/aggregate lengths are checked before their completed header reaches the allocating parser. The framer retains only its bounded length header and aggregate stack, rather than duplicating the response. It accepts the RESP2/RESP3 shapes supported by the installed parser, including errors, maps, attributes and pushes, across arbitrary fragmentation and multiple frames per read.

A per-cached-connection request gate makes the shared operation budget include **all pipeline replies**, while retaining pipelined network batching. A frame-only budget was insufficient because redis-rs accumulates pipeline replies. The driver task has explicit abort ownership without an Arc cycle. Cancelling an active request or receiving a fatal/over-budget response retires every clone; cancelling a queued request leaves the active one intact. The next cache acquisition reconnects with the same AUTH/SELECT/database/TLS configuration. A failed write is never automatically replayed. The actionable size error suggests narrower GETRANGE/range/SCAN reads and warns that earlier writes may already have applied; no user command is rewritten. Metadata list/detail helpers now propagate retired-connection failures rather than returning misleading unknown/null results.

TLS retains native system roots by default, explicit CA bundles, client certificates/keys, verification policy, explicit server-name overrides and the original tunnel hostname for SNI. Tests use generated certificates and ephemeral loopback peers: verified custom CA + mandatory client authentication + tunnel SNI + SELECT succeeds; wrong CA and wrong hostname fail. No certificate/private-key files were committed. The implementation is confined to dbviewer; dependencies already existed in the workspace lockfile and only their dbviewer edges were added.

Evidence: 30 Redis tests pass, including 15 new framing/transport regressions. Tests feed huge declarations **without allocating the declared payload**, fragment valid replies, reject deep/numerous values, enforce cumulative pipeline bytes/nodes, preserve cache reuse and DB1/DB2 isolation, assert exactly one failed SET was received, and exercise actual Driver list/detail recovery after oversized TYPE/LRANGE/HSCAN replies. A deliberate mutation resetting operation budgets at each top-level frame makes the aggregate-pipeline regression fail; restored code passes the full 475-test database package gate. This is a mutation red/green proof, not a claim that the complete old transport ran the new tests unchanged.

Cost model after repair: accepted reply wire content is at most 32 MiB per active operation, plus at most 100,000 decoded value nodes, parser buffers, decoded representations and result-conversion copies. This is **not** a 32 MiB total-memory claim. The framer has O(depth) state and scans each received byte once; bulk payload scanning advances by slice length. The gate serializes operations sharing one configured logical DB/socket, so throughput under parallel long-running requests may differ; pipelines still batch commands. Distinct cached connections can be active concurrently and no global process memory budget was introduced. Native system-trust handshakes, actual SSH forwarding and WAN throughput remain unmeasured; no 9.8 performance claim follows from the limits alone.

## Open findings

### R04-06 — Minor, architecture: edit provenance is inferred twice

Locations: `ui/src/modules/database/edit-sql.ts:101`, `edit-mongo.ts:68`, and `crates/otto-dbviewer/src/drivers/mongodb.rs:2184` (line numbers shift after follow-up).

Adding an accepted Mongo command/projection grammar currently requires the executor and the independent browser editability grammar to remain consistent. This pass demonstrated a concrete drift failure, so the future cost is supported. Keep the fail-closed client check now; a later result-origin descriptor from the parsed executor (collection, identity provenance, field projection completeness) would centralize that knowledge across editing, copying and comparison. Do not replace the five existing engine adapters with a speculative generic SQL abstraction.

## Coverage and seams inspected

This was a domain-wide risk sweep, not a claim to have hand-traced every line of every owned file.

- Query service: write/read-only gates, detached query outcomes/count+byte TTL bounds, connection lifecycle/single-flight cache, cancellation ownership, import/export, metadata/completion and schema graph seams.
- Engines: MySQL/Postgres chunk decoding and session setup/limits; ClickHouse native/HTTP rowset normalization and streaming caps; Mongo parser/find/projection/result conversion, graph sampling and BSON identity; Redis logical-db pinning, refused connection-state commands, SCAN/type batching and full-reply conversion.
- Connections/SSH: profile visibility and secret export ownership, configured endpoint/TLS parsing, SFTP pool actor/config key and capacity, transfer authorization/publication states, SSH listener readiness/retry and stderr draining. Read source; did not establish a new remote SSH host or access production credentials.
- UI: database page/connection selection and comparison scoping; result grid/edit adapters; per-tab pending state; weak result-view caches and bounded search text; completion gate; document editor; import/export dialogs; dashboard/widget polling; SFTP list windowing and transfer state; connection import/network profile forms. Runtime depth is concentrated on the real Mongo editing flow.
- Server assistant/drafter/approved-change seams were sampled for schema scope, read-only execution and native approval linkage; not an exhaustive adversarial agent or access-policy audit.

Positive, source-backed structure: five drivers share orchestration rather than exposing driver-specific HTTP routes; lifecycle cancellation gates cached initialization; ready result/finished-job caches have explicit byte/count lifetimes; Mongo graph sampling bounds table count and concurrent samples; Redis metadata uses SCAN and pipelined TYPE; grid view state separates column preferences from result-identity-dependent pending edits; chart/widget background work uses a shared two-query gate and hidden-aware backoff.

## Executed verification

Environment: macOS workspace, Rust 1.99.0, Node v22.22.3 (CI differs: Node 26.10.0), desktop Chromium 1280×800. Browser used rebuilt root-provided daemon, isolated slot `r04`, API7844/UI5244, throwaway data directory, `OTTO_E2E_SWEEP_ORPHANS=0`. Development Mongo `otto-dbv-mongo` was initially stopped; only that existing fixture was started, no volumes removed. Random scratch collection names; fixture documents only.

- `node --test unit/dbMongoEditTarget.test.ts unit/dbEditScope.test.ts`: 9 pass, after seeing the new bug assertions fail on original code.
- `node --test unit/db*.test.ts`: 146 pass / 0 fail / 0 skipped, 0.917s after provenance follow-up (initial run was 145 pass).
- `npm run check`: UI guards (1,042 files), Svelte 0 errors/0 warnings, all TypeScript configs pass; repeated after provenance follow-up and passed again. E2E TypeScript gate also passes the expanded fixture.
- `cargo test -p otto-dbviewer --lib sparse_json_import -- --nocapture`: expected red 1 pass/1 fail, before allocation repair.
- `cargo test -p otto-dbviewer --lib import::tests -- --nocapture`: 16 pass.
- `cargo test -p otto-dbviewer --lib`: 461 pass / 0 fail / 6 ignored, 13.67s after provenance follow-up (initial run: 458 pass, 13.94s). Includes engine parsing, access, lifecycle, conversion/budget tests; ignored external/scale tests are not counted as executed evidence.
- `cargo clippy -p otto-dbviewer --all-targets -- -D warnings`: pass, including fresh post-provenance run (9.65s).
- `cargo test -p otto-connections -p otto-ssh --lib`: connections 80 pass / 1 ignored (13.42s), SSH 36 pass / 0 ignored (3.02s). Synthetic SFTP latency benchmark remains ignored.
- First `desktop-db-json-edit.spec.ts` invocation: 4 failures at fixture precondition because Mongo was stopped. No product result inferred.
- New oversized-nested-Mongo case against the prior daemon binary: expected red, real fixture insert/find succeeded and `cells_truncated` was absent (677ms case). This confirms the regression reaches the native query path before the new build.
- After fixture startup, named `desktop-db-json-edit.spec.ts --grep 'JSON cell' --project=desktop-browser --workers=1`: 1 pass, 10.4s total / 9.0s case. Included new real projected-document and computed-identity checks.
- Final fresh build: `cargo build -p ottod` succeeds in 2m39s. One linker warning reports `__eh_frame` >16MB and compact-unwind offsets; this is not a warning-free build. No runtime exception-performance claim was made.
- Final `desktop-db-json-edit.spec.ts --grep 'JSON cell|oversized nested Mongo' --project=desktop-browser --workers=1`: 2 pass, 14.4s total (9.0s and 3.0s), against that fresh daemon.
- Screenshots inspected: `ui/e2e/.artifacts/r04-mongo-computed-identity-light.png` and `ui/e2e/.artifacts/r04-mongo-truncated-dark.png`. Real desktop light computed-id refusal and dark large-cell read-only grid, respectively; the dark grid remains contained at 1280×800. Footer explanations are ellipsized at this width. No native/mobile visual certification from these screenshots.

### Redis transport follow-up verification

- `cargo test -p otto-dbviewer --lib drivers::redis:: -- --nocapture`: 30 pass, 0 fail, 0.09s; 15 new tests cover framing and real redis-rs transport/Driver paths with in-memory or ephemeral TCP/TLS peers.
- Deliberately resetting cumulative budgets at top-level frame completion: `aggregate_pipeline_budget_rejects_individually_small_responses` fails at the expected rejection assertion (0 pass / 1 fail). Mutation restored before the package gate.
- `cargo test -p otto-dbviewer --lib`: 475 pass / 0 fail / 6 ignored, 13.81s. The net increase from 461 is 15 new tests minus one obsolete ConnectionManager-configuration test.
- `cargo clippy -p otto-dbviewer --all-targets -- -D warnings`: pass, 12.06s. Scoped rustfmt and whitespace checks pass.
- `cargo test -p otto-dbviewer --doc`: pass; the crate currently defines no doc-tests (0 executed), compilation 13.09s.
- Logs: `/tmp/otto-r04-redis-tests.log`, `/tmp/otto-r04-redis-mutation.log`, `/tmp/otto-r04-redis-package.log`, `/tmp/otto-r04-redis-clippy.log`.

## Remaining verification limits

Redis failure/reconnect was tested with isolated protocol/TLS peers, not an external Redis deployment. No live PostgreSQL/SSH failure/reconnect tests, native desktop, screen reader, keyboard-only journey, phone/tablet walkthrough or remote WAN benchmark was run by R04 in this pass. MySQL/ClickHouse dev services were inspected as available but not queried for a new wide-data workload. Existing test names and source caps are not substituted for those measurements. Contract and shared-type edits are restricted to the database QueryResult region; other agents own their concurrent changes. No commit, push, deployment, external publication, user-session operation or production DB mutation was performed.
