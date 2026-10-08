# R06 — AWS, Kubernetes, brokers

**Verdict: repairs supplied; integration verification and material replay bounds work remain. Not a 9.8 claim.**

Snapshot: merged PR #94, `a0bd718b9fbc008d72c164ce643a24ed78e6368c`; review branch `review/quality-20261008`. Source findings below existed at that snapshot; this review does not attribute them to the merge algorithm. Concurrent R01–R05/root changes were preserved. Rust 1.99.0; Node 22.22.3 (different from CI Node 26.10.0), macOS. No real AWS/Kubernetes/Kafka endpoint, installed daemon, or user database was used.

## Scope and evidence level

Inventory covered the complete R06 roots in `ownership.json`, including the monitoring scheduler. Deep inspection concentrated on mutation target identity, replay bytes/selection/evidence, AWS account/region routing, Kafka pooling and list bounds, Kubernetes command lifecycle, monitor cache/supervision, and loading/recovery flows. This is broader than PR diff review, but is **not a claim that every line of the roughly 50,000-line domain was audited**.

Inspected units/seams:

- AWS: `accounts.rs` credential/region selection and invalidation; `access.rs`/`http.rs` authorization and router; `native.rs` native-vs-CLI selection/token distinction; `cli.rs` concurrency/cancellation; S3 pagination/preview/download limits; SQS list/receive/mutations; EC2 mutations; EKS import and RDS listing; Athena query classification; metrics/log pagination. UI page keying, EC2/RDS/EKS selection/details, SQS mutation handlers, replay-like confirmation boundaries, S3Preview lifecycle, refresh cadence.
- Kubernetes: `access.rs` namespace/operation mapping, `actions.rs`/UI actions and scale/sync dialogs, resource list gateway/cache/content validators, command/log/exec cancellation boundaries, pod HTTP validation and retry rules, monitor cache/fleet scope, collector/status persistence, backfill plan, scheduler reconciliation. UI resource selection, monitor coalescing/windowing, logs/recovery and responsive overview.
- Brokers: HTTP role/guard checks; service client/tunnel single-flight/pooling; Kafka consume bounds and offset reset; bytes/tombstones/transforms; schema registry caching; replay evidence; GroupsTab preview/reset; ReplayPanel and ClusterViewer. R04 owns SSH internals.

The installers, all native credential/SSO workflows, full monitoring classification/backfill mathematics, every certificate/proxy combination, and every wizard/state were not independently executed. No new end-to-end performance benchmark or real-broker chaos run was performed.

## Confirmed findings and repairs

### R06-01 — High correctness: replay escaped its approved offset range — fixed and regression passed

Location: `crates/otto-brokers/src/service.rs:1433` (`replay_selection`), `:1469` (range count). Before repair, `[from,to]` became a message count; consumed records were all published. On a compacted partition, selecting 10–12 can yield records 10,12,17, thus publishing 17 without approval. Retention clamping can likewise read newer offsets. The production replay now filters partition and inclusive offset bounds before publication; arithmetic saturates before the driver’s 5,000 cap.

Evidence: new regression failed with `[9,10,12,17]` plus another partition instead of `[10,12]`, then passed. Scope of the test is the production selection seam with synthetic raw Kafka records, not broker compaction machinery.

### R06-02 — High correctness: explicit consumer-group reset could not deserialize — fixed and regression passed

Location: `crates/otto-brokers/src/types.rs:598`, `crates/otto-brokers/src/kafka.rs:1331`. UI and documented `{mode:"offset",offset:42}` reached internally tagged `Offset(i64)`, which expects a scalar inside a map. It failed with `invalid type: map, expected i64` before resetting anything. Changed the variant to `Offset { offset: i64 }`; the driver uses that field. Wire shape matches the existing contract.

Evidence: `explicit_reset_offset_uses_the_documented_object_shape` executed red with that precise serde error, then green; it also verifies serialization matches the UI shape.

### R06-03 — High correctness/UX: replay discarded acknowledged progress after a later publish failed — repair supplied, Rust regression pending

Locations: `crates/otto-brokers/src/replay.rs:48`, `:68`, `crates/otto-brokers/src/http.rs:577`, `ui/src/modules/brokers/ReplayPanel.svelte:100` and `:216`. Previously the second failed `.produce_raw(...).await?` returned before persisting the first acknowledged write. A manual retry could duplicate the first message while the user saw only a generic failure.

The new publication module stops on failure, persists the acknowledged prefix through the real `BrokerOpsRepo`, and returns `error?` plus `evidence_saved` alongside count/evidence. Evidence-save failures retain response evidence and report the problem. The HTTP audit includes partial metadata. Canonical TypeScript wire types and `docs/contracts/api.md` were updated together. UI renders partial error/evidence, gives a warning rather than success, and names duplicate risk before a retry. The request is captured before approval and duplicate clicks are disabled during approval.

A new deterministic failure-on-second-send regression uses the production publication module and real isolated SQLite repository; it asserts only offsets 0 and 1 were attempted, only acknowledged offset 0 was persisted with destination offset 100, and the result warns about duplicates. **This new Rust test has not yet run at handoff.** The new UI partial-result handler regression ran red (success toast on partial failure) then green.

Limits: this is not exactly-once delivery; a failed send can have an unknown Kafka outcome. Cancellation/daemon termination during sends is not transactionally coupled to SQLite. A durable operation journal with resumable statuses is required to close that larger gap.

### R06-04 — Medium UX/correctness: production scale approval described zero replicas for every scale — fixed

Location: `ui/src/modules/kubernetes/actions.ts:134`. Scaling a production deployment to 3 prompted “to 0 replicas,” then submitted 3. Confirmation now uses the actual merged replica count. Test exercises both 0 and 3 and checks both the displayed approval and request; observed red then green.

### R06-05 — Medium correctness: AWS mutation scope changed while approval was pending — fixed for traced EC2 and SQS delete paths

Locations: `ui/src/modules/aws/Ec2View.svelte:109`, `ui/src/modules/aws/SqsView.svelte:188`. EC2 looked up the current region again after approval. SQS deletion reread the current selected queue and region. A route/selection change during the asynchronous confirmation changed the destination from the named target (SQS could also remove a matching message ID from a newly selected queue’s local list).

Both now capture account/region/target before approval, and the SQS completion updates only the same visible queue. The actual production handlers ran against a deferred approval, with state changed before resolution. Both failed before repair and pass after repair. These are handler-level tests, not simulated user clicks behind a modal.

### R06-06 — Medium correctness: equal RDS/EKS names in different regions collided in detail responses — fixed

Locations: `ui/src/modules/aws/RdsView.svelte:111`, `:101`, `:191`; `ui/src/modules/aws/EksView.svelte:87`. Open region A’s `orders`, then region B’s `orders`; a slow response for A passed the name-only guard and replaced B. RDS additionally derived its visible instance via name-only lookup across the all-region list. Both detail handlers now use request generations and capture explicit region on the selected object; RDS lookup/selection include region. Tests reproduce late A winning before repair, and B retaining its data afterward.

## Remaining findings / material limits

- **Major performance, source-confirmed, not repaired in this pass:** replay calls `consume_raw` (`service.rs:1007`, `kafka.rs:922`) without the viewer byte budget. It materializes up to 5,000 complete payloads before publishing serially. At 1 MiB/message that is approximately 4.9 GiB raw payload data before additional overhead; the 16 MiB viewer cap is deliberately absent. Cost is O(N × payload bytes) memory and O(N × publish latency) wall time. Use a bounded batch/stream with persistent progress or reject oversized batches before any sends, with a response that cannot imply full completion. No workload benchmark was run, so this is a cost-model finding, not measured latency.
- **Major correctness boundary:** selectors above the 5,000 driver cap, or a consumption deadline before the requested range is exhausted, are not explicitly represented as incomplete replay selection. `selector_to_consume_req`/driver clamp and `raw.truncated` do not provide a complete replay contract. Validate supported selection size and propagate incomplete selection truthfully before claiming completion.
- **Minor-to-major depending on uptime/cardinality, source-confirmed:** `schema_registry.rs:145` negative-cache entries expire only when the same ID is revisited. Distinct bogus Confluent IDs cause one retained error entry each; there is no global cap/prune. Cost O(distinct IDs) while the client remains active. Existing per-ID TTL tests do not prove bounded memory. Apply a cap/expired-entry sweep and exercise many distinct IDs.
- Remaining neighboring asynchronous approval paths deserve the same target audit: EKS import and SQS redrive/purge/send still contain reads after confirmation. These were inspected but not fully fault-injected; no claim of their complete coverage.

## Architecture / design / usability assessment

The domain already has useful deep seams: driver operations behind service/context interfaces, per-resource authorization before credentials, bounded list gateways, and shared page/modal/menu components. The replay extraction earns its seam through two real adapters (Kafka and deterministic failing publisher); it centralizes byte transforms, acknowledgement tracking and partial-result semantics rather than adding an interface-only layer.

The strongest future-cost concern is the split between request-scoped replay execution and durable evidence: lifecycle, retry policy and publication progress cannot be made reliable by adding more toast conditions. A persisted operation state machine is the concrete next refactor if crash/cancel resumability is required. No broad rewrite or line-count-only finding is proposed.

The inspected screenshots show a coherent shared header, readable content and contained phone layout. Browser tests exercise retry/cancel/selection states and keyboard range navigation. The fixture’s dated “last cycle” label is synthetic; screenshots do not demonstrate real collector freshness, native WKWebView or screen-reader behavior.

| Vertical | Assessment / 10 | Confidence and reason |
|---|---:|---|
| Correctness | 8.5 | High on reproduced repairs; medium overall. New partial-replay Rust regression and remaining incomplete-selection/crash boundaries prevent 9.8. |
| Performance | 8.0 | Medium static evidence: bounded UI/list/monitor mechanisms, but replay byte accumulation and negative-cache retention remain; no new representative benchmark. |
| Design | 9.0 | Medium: shared UI/driver seams inspected and replay responsibility improved; persistent operation boundary and full visual breadth remain. |
| UX/usability | 9.0 | Medium: 11 real mounted mocked browser journeys and red-capable handlers; partial replay browser/native journeys still pending. |

These are bounded engineering assessments, not scores inferred from test counts.

## Executed verification

1. `cargo test -p otto-brokers --lib explicit_reset_offset_uses_the_documented_object_shape`: failed as expected with serde map/scalar error before fix.
2. `cargo test -p otto-brokers --lib replay_range_excludes_compacted_gaps_and_other_partitions`: failed as expected before filter repair.
3. `cargo test -p otto-brokers --lib`: **61 passed, 0 failed, 0 ignored, 5.39 s**. This run was **before** the later replay publication extraction/partial-response change; it is not presented as verification of that change.
4. `node --test unit/infraMutationTargets.test.ts unit/infraViewers.test.ts unit/k8s*.test.ts unit/awsAthenaWriteGate.test.ts unit/pollBackoff.test.ts` from `ui`: **57 passed, no skips**. Six new tests cover production handler/selection behavior; each new behavior was observed failing before its repair. Existing helpers include 5k/12k/50k table bounds, log rings, poll/304 behavior and monitor coalescing. Unit timings are not a UI performance benchmark.
5. `npm run check`: **passed** (0 Svelte errors/warnings, guard/type configs passed), during this shared worktree wave. The final RDS/EKS/SQS edits landed after this command started; coordinator must include them in the final UI check.
6. Named browser command below: **11 passed, 0 skips, 29.2 s**. Mocked AWS/K8s routes, isolated temporary daemon, Chromium desktop project with phone viewport cases. This run predates the new partial replay and later RDS/EKS/SQS handler edits. It tests existing recovery/composition journeys, not the new Rust code.

```sh
OTTO_E2E_SLOT=r06 OTTO_E2E_PORT=7866 OTTO_E2E_PW_PORT=5266 \
OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod \
npx playwright test e2e/desktop-ux-r4-cloud.spec.ts --project=desktop-browser --workers=1 \
  --grep 'Athena phone|EKS successful|S3 pending|SQS |AWS setup|CloudWatch multi|Kubernetes log failure|Fleet Requests and monitor overview composition (native-light|native-dark|warm-light-phone)' \
  --output=e2e/.artifacts-r06
```

Artifacts: `../evidence/R06/cloud-functional-results.json`; `fleet-native-light.png`, `fleet-native-dark.png`, `fleet-phone.png` (all three visually inspected). Daemon fixture was `target/debug/ottod` built before these broker edits; no claim of fresh-binary backend validation. `git diff --check` passed at handoff.

## Exact queued gates / handoff

- Run `cargo test -p otto-brokers --lib` again for **new** `replay::tests::nth_publish_failure_persists_acknowledged_prefix_and_stops` and existing byte/tombstone/nullable-header tests after extraction. Mutation-check the new test by temporarily skipping evidence persistence on error, then restore; it must fail `evidence_saved`/DB assertions. Root owns Cargo coordination.
- `cargo clippy -p otto-brokers --all-targets -- -D warnings`; affected `otto-server` compile/selected broker HTTP contract tests, including partial audit fields. Check all `ReplayResp` constructors (current source has one in the new module).
- Final whole-wave `npm run check` and the six new handler tests. Mounted browser case for partial replay should assert error/evidence, retry duplicate warning and no misleading success toast.
- Existing AWS/K8s package tests and selected server monitoring/backfill tests were **not run by R06**; root may include them in its integration gate. No full Playwright suite requested.
- Remaining replay bounds and schema-cache findings above need repair before a near-9.8 claim. Native Cloud/SSO and representative performance evidence remain separate environmental coverage limits.

## Root follow-up: bounded replay and cache (same review branch)

The three remaining material findings above are now repaired, superseding their pending status; the original provisional scores are retained pending independent integration review. Replay validates count/range before cluster lookup, caps raw key/value/header accumulation at16MiB before copying the next record, and rejects an incomplete read before publishing anything. Deliberately selecting fewer messages than the topic contains is distinguished from deadline/budget failure. UI explains the limits and rejects ranges above5000 offsets. The negative schema cache serializes admission, prunes expired entries and caps distinct errors at512.

Four real regressions failed before repair:1024failed IDs retained; an oversized raw record returned despite a one-byte budget; zero-deadline read falsely complete; invalid selectors reached cluster lookup. After repair all66broker library tests passed through nextest (7.653s execution), including the partial-publication evidence regression, bytes/tombstones/nullheaders, and the new guards. Evidence:/tmp/postmerge-r06-bounds-{red,green}.log. Named mounted browser `Partial replay retains acknowledged evidence and warns before retry` passed1/1in5.5s; real UI with mocked upstream verifies partial error, retained acknowledged evidence, no success toast, and retry duplicate warning before another request. Evidence:/tmp/postmerge-r06-partial-browser.log. `cargo clippy -p otto-brokers --all-targets -- -D warnings` passed (34.06s).

Adjacent source-confirmed correction after the above gate: `consume_raw_from` now errors if any requested partition watermark is missing (batch watermark API documents missing entries as unreachable). Previously `(0,0)` could misreport an unreachable partition as empty. This final guard awaits the consolidated broker/consumer gate; the preceding66-test run does not claim to cover this later edit.

Final selected replay UI gate: three named cases passed9.1s (`Replay rejects…`, `Replay confirms…`, `Partial replay…`), including inclusive5000-offset boundary and rejection of5001offsets. Log:/tmp/postmerge-r06-replay-browser.log. No real Kafka publication in browser tests; the isolated daemon provides real workspace/cluster setup, with replay transport intercepted by the fixture.
