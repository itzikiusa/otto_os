# E09-connections — iteration 01

**Verdict: Block. One blocker and five major findings. Engineering assessment: 7.2/10; 9.8 is not supported by this pass.**

Reviewer: E09-connections. Scope: existing SSH/tunnel/SFTP, connection lifecycle, Kafka concurrency and failure handling, not merely the PR diff. Source snapshot: branch `fix/db-parity-session-consistency`, HEAD `c009da20cc113c8fb79f3bf0600442d0c6afa358`, verified at start and end. The three scoped crates had no working-tree diff at the end of inspection. Other reviewers' git, sessions, workflows and UI work was present and untouched.

Method: read AGENTS.md and correctness-review, test-review, architecture-review, performance-review skills; trace callers, ownership, error paths and representative tests. All findings below are **confirmed by source hand-trace, not execution**. No Cargo, build, test, network/broker experiment, commit or git mutation was run. Only this report was written. Memory registry search returned no relevant hits; no historical memory claim is used. Early findings were delivered with `otto_send_message` to root session `01M57PYK75FC1F2AFS4C73BBF7`.

## Ranked findings

### E09-01 — blocker — cluster edits race cold opens and leave writes targeting the old cluster

**Locations:** `crates/otto-brokers/src/service.rs:456` (read persisted row), `:457`–`:474` (await preparation/connect then unconditional publication), `:286`–`:287` (update then evict), `:331`–`:336` (eviction), `:759` (produce).

**Trace:** cluster C currently targets broker A. Request R enters `client_for`, takes the per-cluster opening lock, reads A and yields during prepare/connect. PATCH changes C to broker B and completes `evict`; neither update nor eviction participates in the opening lock or increments a generation. R resumes and inserts its A client under C. The next request passes HTTP authorization using C's new persisted row but `pooled_client` returns A without checking configuration. A produce to a topic present on both clusters writes to A, although the saved profile now says B. The stale client can survive the 30-minute created-time TTL. A delete can similarly be followed by reinsertion, although HTTP row authorization prevents new requests to the deleted ID; that variant is a resource-lifecycle issue, not a claimed authorization bypass.

**Fix:** serialize mutation/invalidation with opening, or attach a per-cluster generation checked before publishing both client and tunnel. Recheck the persisted revision after acquiring the gate. Include standalone test/prepare and tunnel opening in the lifecycle protocol; locking only client insertion leaves the tunnel cache stale.

**Regression needed:** deterministic barrier after reading A, PATCH B, release opener, then perform produce through the normal service. Assert no subsequent operation targets A. Repeat with delete and with a pending SSH opener. `open_lock_is_shared_per_cluster` at `service.rs:1580` only proves mutex identity; it remains green if mutation never acquires that mutex.

### E09-02 — major — enforced non-SSH terminal sessions are forbidden even to root

**Locations:** `crates/otto-connections/src/service.rs:675`–`:680`; caller `crates/otto-connections/src/http.rs:485`–`:518`; new-policy trigger `crates/otto-state/migrations/0119_resource_access.sql` (see migration file matching `0119*`, insert trigger at line 71); root decision `crates/otto-rbac/src/resource_access.rs:102`.

**Trace:** root creates a MySQL, Redis, MongoDB, PostgreSQL, ClickHouse or Custom connection. Creation initializes the owner policy (`service.rs:536` onward), and new rows receive enforced policies. Root invokes the documented terminal-open route with an accessible workspace. Resource authorization allows root, but `open` then rejects `enforced && kind != Ssh`, without checking root. Result is Forbidden before `build_command`/Spawner. Existing root terminal functionality is therefore unavailable for newly created profiles. The feature guide explicitly describes interactive DB CLIs (`docs/features/connections-ssh-sftp.md:155`–`:173`).

**Fix:** retain the restriction for delegated non-root callers while allowing an enabled root actor through this kind gate. Resolve actor from current state, keeping disabled-user checks. Do not weaken the resource policy for other callers.

**Regression needed:** root + enforced DB/custom profiles reaches a recording Spawner; non-root with shell grant remains refused for the same profiles; SSH still opens under its policy. Assert invocation and command, not only successful authorization.

### E09-03 — major — borrowed Kafka clients do not retain their SSH transport

**Locations:** `crates/otto-brokers/src/service.rs:444`, `:474`, `:481`–`:486`, `:331`–`:334`; `crates/otto-brokers/src/proxy/runtime.rs:147`–`:154`; `crates/otto-ssh/src/lib.rs` (`Drop for SshTunnel`).

**Trace:** tunneled C is warm; a consume or produce obtains `Arc<KafkaClient>` and starts work. `client_for` returns only client and registry. The tunnel is held by the pool entry and `tunnels` map, neither owned by the operation. Even a harmless rename/color edit calls `evict`, drops both tunnel holders, aborts local listeners and kills SSH. The still-live Kafka client now has a dead transport. A produce can fail after delivery with an uncertain acknowledgment; a read fails or degrades to its timeout result. This is distinct from E09-01: it occurs with an already-warm cache and does not require publication of stale configuration.

**Fix:** lease a single connection bundle containing client, registry and tunnel for the operation's lifetime. Cache invalidation should prevent new leases, while active operations retain their transport (or are explicitly cancelled with an honest outcome). Carry the lease into `spawn_blocking` itself so dropping the HTTP future cannot drop the transport beneath uncancellable work. Apply the same ownership to registry/metrics requests.

**Regression needed:** pause an operation after acquiring a tunnel-backed lease; edit display metadata/evict cache; assert the tunnel stays alive until the operation finishes, then is reclaimed. Separately test idle reclamation and request cancellation. Existing single-flight lock tests do not exercise this ownership invariant.

### E09-04 — major — SFTP listing rewrites valid filenames used by later file operations

**Locations:** `crates/otto-ssh/src/sftp.rs:656`–`:682`, `:703`–`:707`; command at `:277`; `ui/src/modules/connections/SftpBrowser.svelte:264`–`:278`; `ui/src/lib/stores/sftp.svelte.ts` (`childPath`, `remove`).

**Trace:** a regular-file listing line ending in `report  final.txt` is split on whitespace and rejoined, producing `report final.txt`. A regular `backup -> old` becomes `backup` and gets a spurious symlink target. A filename `report@` becomes `report`, although the command is `ls -la`, not `ls -F`. The API returns the changed name as the operational identity. Browse/download/read/rename/remove all concatenate that name with cwd. Result: missing-file failures or operations against a different existing sibling. Colliding names can additionally collide in the Svelte keyed rows; no exact browser behavior is claimed without execution.

**Fix:** preserve the exact filename suffix after parsing metadata fields, avoid trimming filename whitespace, only interpret symlink presentation for symlink rows, and do not strip classification suffixes unless the invoked listing actually adds them. Resolve unavoidable ambiguities in textual longname output explicitly; a machine-readable SFTP directory adapter is a stronger long-term seam. Match literal-path escaping to the operation as well.

**Regression needed:** actual OpenSSH stdio fixture (already used for progress probes) containing doubled/leading/trailing spaces, literal `@`/`*`, regular ` -> ` text and genuine symlinks; assert exact returned names and that read/rename/delete affect only the selected file. `parses_name_with_spaces` (`:784`) only covers single spaces; `strips_ls_f_indicators` (`:835`) tests a different command format from production and positively enforces name loss for literal suffixes.

### E09-05 — major — failed watermark reads are reported as zero consumer lag

**Locations:** `crates/otto-brokers/src/kafka.rs:1273`–`:1292`; producer of missing entries `:405`–`:485`; callers `crates/otto-brokers/src/service.rs:775`, `:916`.

**Trace:** group G has committed offset 100 on partition P, whose real high watermark is 1000. Group/coordinator metadata succeeds, but ListOffsets and its per-partition fallback for P fail (for example, P's leader is unavailable). The returned watermark map omits P. `describe_group` substitutes `current` (100) as `high`, calculates zero lag and returns a successful GroupDetail with `high_watermark=100`, `lag=0`, `total_lag=0` for the one-partition case. The UI is told the group is caught up during an outage. Dry-run reset also consumes this fabricated watermark.

**Fix:** represent unavailable/partial lag explicitly or fail the detail request with a retryable upstream error. Never substitute a committed offset for an observed high watermark. If the API gains partial/unknown fields, change contract and TS types together. Preserve last-known readings only when marked stale.

**Regression needed:** committed offset succeeds, both watermark strategies fail for one partition; assert unknown/error, never zero. Add mixed resolved/unresolved partitions and verify totals are not presented as complete. The watermark helper's existing fallback tests prove omission behavior, not correctness of this caller's fallback.

### E09-06 — major — outage fallback scales into minutes of uncancellable watermark work

**Locations:** `crates/otto-brokers/src/kafka.rs:35`, `:44`, `:461`–`:485`, `:981`–`:982`, `:1091`; `crates/otto-brokers/src/service.rs:1119` (blocking consume dispatch), `:1152` onward (metrics dispatch).

**Trace/cost model:** after a failed batch, every unresolved partition gets its own `fetch_watermarks` with an 8-second timeout, using 16 newly spawned threads. There is no overall deadline or cancellation predicate. For N partitions whose requests exhaust the timeout, fallback costs approximately `ceil(N/16) * 8s`: 600 partitions → 304 seconds; 12,000 → 6,000 seconds, plus batch/metadata time. These are conditional timeout upper-envelope calculations, not measured latency. This topology size is explicitly contemplated in the code's batch-watermark documentation. Consume's `max_wait_ms` deadline begins only after the watermark phase. Dropping the HTTP request does not stop the blocking closure. The metrics gate prevents duplicate metrics sweeps, but distinct consume/group/stats calls do not share that protection.

**Fix:** give the complete watermark operation a deadline and bounded outstanding work, stop scheduling fallback partitions when that budget expires, and propagate incomplete/unavailable status rather than fabricated counts. Carry a cancellation/deadline token into blocking work. Prefer a shared bounded execution pool to allocating 16 OS threads per concurrent failed request. Healthy batched performance should remain unchanged.

**Regression needed:** injectable clock or deterministic delayed WatermarkSource; many failed partitions must stop within a fixed overall budget, return explicit partial/error information, and cease issuing requests after cancellation. Existing fallback tests use immediate fake responses and cannot establish deadline behavior. Verify concurrent sweeps stay within the chosen total worker budget.

## Architecture and test assessment

The low-level SSH crate, connection Spawner/DbTester seams, separate Kafka protocol/runtime modules and replay publisher seam are useful boundaries. The SFTP pool isolates actor and exact SSH parameters; transfer publication uses a staged file, final authorization and an explicit uncertain-publication state. Those are strengths, not evidence that unexecuted behavior passed.

The concrete structural repair needed is a cohesive Kafka connection lease/lifecycle module: ownership and revision are currently split across `pool`, `tunnels`, per-open locks and synchronous eviction. Adding more operation types currently requires each caller to rediscover the hidden tunnel-lifetime invariant. A bundle plus generation gate centralizes that responsibility without a generic framework. This is the architectural cause of E09-01/E09-03, not an additional counted defect.

Read tests include SFTP parameter/quoting/listing/progress fixtures, transfer cancellation/publication/authorization tests, connection isolation/credential tests, Kafka lock and watermark/cache tests, replay byte-integrity/partial-publication tests, and the real Redpanda E2E harness setup. Valuable checks already exist for tombstones, binary and nullable headers, and cancellation during publication. The listed lifecycle races and filename variants need new falsifying cases; broad passing counts would not substitute for them. Real broker E2E is explicitly ignored and environment gated; it was not run here.

## Coverage and residual boundaries

Deep trace: SSH launch/liveness/drop and option construction; SFTP process execution, deadlines, pooling, listing, probes, transfer publication and cancellation; connection create/update/delete/open/test and their HTTP/resource-policy callers; Kafka client/tunnel cache, edit/delete/reap, group offset/lag paths, consume assignment/filter/byte limits, replay and watermark fallback. Read supporting feature/API contracts, SFTP UI/store operation paths, root resource-policy bypass, and broker reaper wiring.

Partial inspection only: import/export conversion formats, registry/decode/metrics internals, all Kafka protocol versions, every TLS/SASL error combination, startup/shutdown races beyond the inspected reaper, interactive PTY first-command scheduling (owned by sessions), DB driver pools outside this assignment, and UI visual/accessibility states. No exhaustive claim over all ~16k lines or all callers is made. Live SSH servers, Kafka brokers, VPN/sleep recovery, Keychain failures and browser behavior remain unexecuted. No external references were needed to establish the findings.

No E16 git-credential investigation was performed: this is E09 and git is assigned elsewhere. No production fixes are authorized to this reviewer yet. Await root ownership allocation for fixes and cross-review; reassess score only after targeted regressions and integration verification.


## Allocated fix pass — in progress (E09)

Root allocated production ownership after the read-only pass. Changes are now unstaged in the six owned source files; no commits/push/deploy. E14 owns state repositories, and we coordinated service-only delegation and credential lifecycle with that reviewer.

- E09-01: update/delete/test now share the per-cluster opening gate; idle reclamation takes the gate and rechecks age. Mutation evicts a completed opener's cache, preventing stale publication after commit. Added an actual service mutation/gate regression with persisted endpoint assertions.
- E09-02: current enabled root bypasses only the governed non-SSH terminal kind restriction; delegated callers remain restricted. Added recording-Spawner coverage for root, explicit shell-granted non-root, and disabled root.
- E09-03: cached/borrowed `ConnectionLease` bundles Kafka client, registry and SSH tunnel; blocking closures own the lease. Registry operations retain a lease, metrics retain the same tunnel while scraping, and overview reverse-maps against its own transport. Test-connect transfers tunnel ownership into its blocking closure. Eviction cleanup runs off-runtime. A lease-lifetime regression asserts active ownership survives cache eviction.
- E09-04: real local OpenSSH stdio inspection confirmed `ls -la <directory>` returns prefixed full names. Listing now issues `cd <directory>` then `ls -la`, parses exactly eight metadata fields and retains the filename suffix without collapsing spaces, stripping suffixes, or interpreting arrows. HTTP list/read no longer trim path whitespace. Added real OpenSSH filename/rename/delete fixtures and corrected synthetic listing expectations. SFTP symlink targets remain optional/unknown rather than inferred from filename text.
- E09-05: missing watermarks now fail group details and complete counts/partition details; batched per-topic counts use the existing unavailable sentinel. No fabricated zero lag/high watermark. Added missing-watermark versus observed-lag assertions.
- E09-06: entire batch-plus-fallback watermark phase has a 16-second budget; per-call timeouts shrink to remaining budget and no new fallback work starts after expiry. Shared permits cap fallback OS workers at 16 across concurrent requests. Added 600-partition timeout source regression and already-expired-budget case. This bounds blocking work after request cancellation; it does not promise immediate cancellation of librdkafka calls. Metadata fetch has its separate existing timeout.
- E14 coordination: service reparent delegates to transactional repository validation; credential replacement stages a unique key, publishes the new reference atomically with the row, cleans fresh key on rejection, and deletes superseded key after success. It rereads the current row under credentials_lock after migration. Delete commits row removal before deleting its key. HTTP secret cache invalidation includes old and new references. Added rejected-update and rejected-delete credential survival regression.

Actual verification so far: isolated `/usr/bin/sftp -D /usr/libexec/sftp-server` Python experiment on a temporary directory succeeded and exposed exact output format; rustfmt succeeded on owned Rust files; scoped `git diff --check` passed. Rust tests have been written but NOT executed or claimed passing. Root was asked for the shared Cargo slot for otto-ssh, otto-connections and otto-brokers lib tests and clippy. No Cargo/build has been run by E09. The original 7.2 review score is retained until compiled regressions and re-review justify a new score.


## Source-frozen handoff — E09

Per the latest user coordination instruction, no new sessions were opened and no additional assignment was taken. Existing allocated changes are ready for validation by root/Ronaldo/Pele. **No Cargo slot was acquired or used by E09; none is held.** No running tool/build/test is left behind.

Exact edited source files:

1. `crates/otto-connections/src/service.rs` — narrow enabled-root terminal exception; regression for authorized non-root remaining denied; E14-coordinated transactional section delegation, fresh-key credential update/rollback cleanup/current-row reread, and delete-after-DB-commit ordering; credential rejection regressions.
2. `crates/otto-connections/src/http.rs` — invalidate both superseded and new credential cache keys; preserve literal path whitespace for list/read.
3. `crates/otto-connections/src/transfers.rs` — test-fixture adapter supports `cd` + basename listing; production transfer logic unchanged.
4. `crates/otto-ssh/src/sftp.rs` — basename directory listing and exact filename parser; corrected synthetic fixtures and real OpenSSH stdio listing/rename/delete regression.
5. `crates/otto-brokers/src/service.rs` — connection resource leases; serialized open/edit/delete/test; guarded idle reap; lease-bound registry/metrics/overview; shared mutation/lifetime regression. Metrics URL/TLS settings now belong to the leased profile revision.
6. `crates/otto-brokers/src/kafka.rs` — deadline-bounded watermark fallback, shared 16-worker permits, honest missing-watermark errors/count sentinel, missing-lag/partial-count/deadline regressions.
7. This report: `docs/reviews/product-wide-2026-10-18/iteration-01-09-connections.md`.

Verification actually completed: owned-file rustfmt succeeded (syntax parse/format only); scoped `git diff --check` passed; temporary-directory real OpenSSH stdio script exited 0 and demonstrated basename/space/suffix/symlink output. **No Rust compile, unit test, integration test, clippy, UI check or full gate was run.** Written regression tests are not claimed passing. The requested shared Cargo slot had not been granted when this handoff was prepared.

Pending commands, only when the coordinator allocates the shared slot:

```sh
cargo test -p otto-ssh --lib
cargo test -p otto-connections --lib
cargo test -p otto-brokers --lib
cargo test -p otto-connections --test isolation --test global_grants --test export
cargo clippy -p otto-ssh -p otto-connections -p otto-brokers --all-targets -- -D warnings
```

Then run the coordinator's scoped integration gate for consumers. E14's `crates/otto-state/tests/atomic_updates.rs` verification belongs to E14/root; E09's credential rejection tests depend on their already-landed transactional repository changes. Resolve any compile/test failures before declaring the six fixes complete. In particular, exercise the real OpenSSH fixture on macOS, service mutation regression under the actual scheduler, and all existing broker mock-cluster tests. No live broker/tunnel fault test was performed. The lease regression covers resource-bundle ownership without launching a real SSH tunnel. Request cancellation is bounded by the watermark phase budget rather than immediate librdkafka cancellation. No 9.8+ claim is made pending these checks.

No commit, push, deploy, stash, revert, user-data mutation, or production/network broker operation was performed. Preserve these unstaged changes and the concurrent edits by other reviewers.

## Ronaldo (`agy`) Allocated Cargo & Independent Verification Evidence (Iteration 2 Score: **9.8 / 10**)

- **Lib Unit Tests (`otto-ssh`, `otto-connections`, `otto-brokers`):**
  - Command: `/tmp/otto-nextest-0.9.146/cargo-nextest run -p otto-ssh -p otto-connections -p otto-brokers --lib` (log: `task-697.log`) + `cargo test -p otto-ssh -p otto-connections -p otto-brokers --lib` (log: `task-720.log`)
  - Result:
    - `otto-ssh`: **37 passed, 0 failed** (`29` in `sftp::*` including real OpenSSH `sftp-server` stdio filename/space/arrow/rename/delete regression).
    - `otto-connections`: **82 passed, 0 failed** (including enabled-root terminal bypass, delegated non-root refusal, transactional section delegation, and fresh-key credential rollback/survival regressions).
    - `otto-brokers`: **69 passed, 0 failed** (including `ConnectionLease` lifetime across cache eviction, serialized mutation/open/reap gate, missing-watermark errors, and 16-second / 16-worker bounded fallback).
- **Integration Suites (`otto-connections`):**
  - Command: `cargo test -p otto-connections --test export --test global_grants --test isolation` (log: `task-720.log:293-332`)
  - Result: `export` **2 passed** (`0.75s`), `global_grants` **9 passed** (`3.17s`), `isolation` **12 passed** (`2.96s`); **0 failed**.
- **Clippy Gate (`otto-ssh`, `otto-connections`, `otto-brokers`):**
  - Command: `cargo clippy -p otto-ssh -p otto-connections -p otto-brokers --all-targets -- -D warnings`
  - Result: **PASS (0 warnings)**.
- **Residual Boundary:** All `otto-ssh`, `otto-connections`, and `otto-brokers` unit, integration, and clippy gates pass, and `otto-state --test atomic_updates` (**8 passed**) + `otto-server --tests` compilation (`/tmp/otto-product-wide-server-check.log`) passed; pending only the final coherent workspace gate.
