# Iteration 1 implementation — data tools (role 2)

Scope: C2-01–04, P2-01–03 and UX2-01–03. All work is on the shared review worktree/branch. No live databases/brokers, user repositories or external services were mutated. Builds/tests are executed centrally by the parent; this agent performed source inspection, edits and parent-authorized rustfmt only. UI paths were reserved in the coordination note before edits; existing visual styling was preserved.

## Evidence and repairs

| Finding | Confirmed cause and repair | Regression |
|---|---|---|
| C2-01 | `edit-sql.ts` accepted aliased/computed key values and consumed only the first FROM token. Replaced it with a conservative single-table/direct-column grammar: reject aliases, expressions, duplicate names, joins/table aliases/modifiers and extra FROM sources. Result names must be unique. MySQL double-quoted projection strings are also rejected; double-quoted column identifiers require PostgreSQL engine context. | `dbEditScope.test.ts`: aliased key/nonkey/expression, duplicate projection/result names, comma source, table alias, normal direct SELECT, PostgreSQL identifier vs MySQL string-literal provenance. |
| C2-02 | ClickHouse sorting/primary-key metadata does not prove uniqueness. `EditFlow.resolveTarget` refuses row mutations; UPDATE/DELETE/replace builders independently return null for ClickHouse. Copy-as-INSERT keeps its separate target and builder. | `dbEditScope.test.ts`: all row mutation builders reject even a one-row page with a purported key; INSERT remains available. No page-local uniqueness heuristic. |
| C2-03 | Both successful checkout and rollback auto-stash paths omitted `--index`. Both now restore the exact stash with `--index`; failures retain the stash and successful-switch recovery explicitly says `git stash pop --index`. Ordinary manual stash-pop was not changed. | Disposable Git fixtures compare cached/uncached diffs byte-for-byte on successful switch and failed checkout; staged conflict retains the stash's index snapshot. |
| C2-04 | Pre/post workers returned snapshots which replaced current workspace variables. Runtime now records explicit set/unset operations in `ScriptRun.writes`; store merges only those operations into the current map, while the request continues using its original execution snapshot. Last completed successful script wins only for keys it actually writes. | Controlled concurrent sends: read-only slow response, disjoint writes, unset while another key is added, manual edits while pending; explicit same-value writes remain intentional operations. |
| P2-01 | Sparse BSON source budgets missed rectangular null padding. Shaping now charges exact escaped JSON bytes (counting writer, no serialized-row copy), column metadata and every cell before retaining it; enforces 1,000,000 resident cells. A byte/cell truncation sets `truncated_reason: bytes`, which `collect_docs` preserves. Oversized metadata yields no oversized result columns. Conversion offload accounts for potential expanded width. | 100k sparse documents, 500 columns; escaped control-character cells; a late document widening earlier rows; final JSON <=32 MiB, bounded cells, correct retained values/reason. |
| P2-02 | Normal and governed SQL batch paths bypassed conservative limit injection. MySQL/Postgres now inject `max_rows+1` for each eligible SELECT, without offset or batch paging, on the existing shared connection. | Added ignored MySQL/Postgres integration fixtures whose SELECT increments a server counter: later statement sees 3 evaluations for a 2-row cap and the same backend ID. Requires isolated SQL fixtures. Existing injector guards continue to apply. |
| P2-03 | Schema history fetched every body serially. History now returns sorted/deduplicated `{version}` identifiers only. The UI lazily fetches its two selected details, aborts superseded list/detail requests and caches numeric versions in an 8 MiB/64-entry cache. Failed A/B loads have independent state and Retry. | Fake registry with 100 versions sees exactly one list request. Component-function test sees two body requests, cached revisits and cancellation on changed selection; cache byte/entry eviction test. |
| UX2-01 | Scratch requests bypassed the existing dirty detector on close. Dirty scratch and saved requests now both ask before discard; scratch consequence does not promise a saved copy. Stable tab identity remains authoritative after confirmation. | Function regression: Cancel keeps draft, Discard closes stable tab. Persistence E2E now explicitly cancels once and discards before asserting removal survives reload. |
| UX2-02 | Keyed automation editor was destroyed by unguarded view transitions. Editor exposes a coalesced Save/Discard/Cancel leave decision and registers it with the router. API request-tab, automation selection, environments and close transitions await it; canceled selection does not switch request tabs. Failed save stays open; edits arriving during a save retain dirty state. | Actual component-function tests for Cancel, failed Save, successful Save, Discard, deferred-save edits and canceled request-tab navigation. |
| UX2-03 | Tail fetch errors were swallowed while the active claim remained. Tail records error/last-success, clears the old new-count, preserves buffer/cursors and offers Retry while polling continues. Successful empty responses restore freshness. Responses/errors from a previous topic cannot modify a newly selected topic. | Actual incremental consume function: failure, preserved cursor/buffer, empty recovery and stale-topic response rejection. |

## Contracts and compatibility

- `docs/contracts/api.md`, Rust `SchemaVersion` and central TypeScript `BrokerSchemaVersion`/`BrokerSchemaVersionDetail` change together. Module-local broker types re-export these contracts. The version-list response intentionally no longer includes body, schema type or global schema ID; callers use the existing detail endpoint.
- The SQL batch contract documents the conservative exception: **non-rewritable statements keep their SQL and session semantics; a subsequent statement may still drain an unread result.** No connection switch, implicit skipped write or transaction restart was introduced. This is the known remaining bound for explicit LIMIT/UNION/locking/other ineligible statements.
- Mongo output may retain zero rows if even the first expanded row exceeds the budget. Its contract now states this exception to the older SQL first-row behavior.
- Database feature docs now state direct unique projection and enforced-key requirements, and ClickHouse's read-only row mutation restriction.

## Central verification evidence received

1. `node --test unit/dbEditScope.test.ts unit/apiClientOwnership.test.ts`: RED, 25 passed / 5 failed. Aliased projection accepted; ClickHouse builder generated DELETE; three variable races reverted `new` to `old`. Log: `/tmp/otto-review-data-tools-red.log`.
2. After initial UI fixes, same tests plus `unit/scriptRuntime.test.ts`: GREEN 35/35. Log: `/tmp/otto-review-data-tools-green.log`.
3. `cargo test -p otto-git --lib checkout_autostash_preserves_index_on_success_and_rollback`: RED, cached diff became empty instead of the staged hunk. Log: `/tmp/otto-review-git-red.log`.
4. `cargo test -p otto-brokers --lib version_history_lists_ids_without_fetching_schema_bodies`: RED, history returned zero entries instead of 100 because it attempted nonexistent body endpoints. Log: `/tmp/otto-review-data-rust-red.log`.
5. `cargo test -p otto-dbviewer --lib sparse_grid_expansion_obeys_cell_and_byte_budgets`: RED, retained cells exceeded 1,000,000. Log: `/tmp/otto-review-mongo-red.log`.
6. Expanded UI run initially exposed a **test timing error**, not a production defect: one `Promise.resolve()` did not wait for the VM's cross-realm Save call to begin. The test now waits on an explicit `saveAutomation`-entered barrier before editing. No behavior was changed to satisfy that timing error. Recovery/cache tests subsequently GREEN 8/8 per parent.
7. First typecheck found six unreachable ClickHouse comparisons after the early guard; removed dead mutation branches. No other Svelte errors were reported. Final rerun remains with parent.
8. Owned Rust files formatted with `rustfmt --edition 2021`; no builds/tests were run by this agent.

## Handoff commands / remaining verification

```sh
cd ui
node --test unit/dbEditScope.test.ts unit/apiClientOwnership.test.ts unit/scriptRuntime.test.ts unit/schemaVersionCache.test.ts unit/dataToolsRecovery.test.ts
npm run check
# repo root
cargo test -p otto-git --lib checkout_autostash
cargo test -p otto-dbviewer --lib grid_expansion
cargo test -p otto-brokers --lib version_history_lists_ids_without_fetching_schema_bodies
```

The parent should additionally execute the changed scratch-persistence E2E (`ui/e2e/desktop-api-tabs-persist.spec.ts`) and capture changed UI states. Isolated SQL integration commands, only with an explicitly disposable fixture at the configured local ports:

```sh
OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mysql_e2e mysql_batch_bounds_select_on_the_same_session -- --ignored
OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test postgres_e2e postgres_batch_bounds_select_on_the_same_session -- --ignored
```

Rust GREEN, final full UI gate, browser screenshots and SQL wire/backend integration have not been claimed from this handoff. Parent owns final verification and runtime CPU/RAM measurements. Normal router navigation participates in the automation guard; forced reload/window termination is not durable automation draft storage. No findings were rejected as false positives.
