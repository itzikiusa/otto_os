# Platform Notes

Domain hints about common patterns in this org's platform, inferred from the codebase.
These are **places to check and common patterns** — not absolute rules. Verify against
the actual code; these may have evolved.

---

## Multi-tenant backoffice database (pr_bo)

The backoffice database uses a per-brand schema pattern. Each brand has its own logical
schema (`pr_bo_{brandId}`), accessed via a multi-tenant connection service.

**What to check in a story:**
- Every SQL query must include a brand scope. Missing `brand_id` filters are a data
  isolation bug — see `references/risk-catalog.md` §5.
- Table definitions live in `go_tests_utils/component/tables/` — use this as the
  single source of truth when verifying column names and types.
- Adding a column means a migration for all brand schemas, not just one.
- Common tables: `MdlGm_tblPlayers` (players), `MdlEnv_tblPlayerBalance` (wallet
  balances), `GSS_activities` (gaming transactions), `MdlCsh_tblTransactions`
  (financial transactions).

**Pattern to look for:**

```go
// Multi-tenant connector — brand_id is always the routing key
db, err := multiSQL.GetBrandConnector(ctx, brandId)
```

---

## Go microservices and service discovery

Internal service-to-service calls go through a service locator rather than hard-coded
URLs. When a story introduces a new cross-service call, check:

1. Does the target service have the endpoint already, or does this story imply the
   endpoint is also being added?
2. Is there an existing client in `go_casino_kit/clients/` for that service? (Check
   before writing a new HTTP client.)
3. Does the call carry brand context (brand ID in header or request body)?

**Pattern to look for:**

```go
serviceURL, err := serviceLocator.GetBrandService(ctx, brandId, "SERVICE_NAME")
```

If `SERVICE_NAME` is new (not already registered in the discovery config), that is a
dependency risk — surface it.

---

## ClickHouse analytics: materialized views and rollups

ClickHouse is used for analytics (game history, reporting, usage tracking). Common
patterns to check:

- **Materialized views** consume an insert stream and pre-aggregate into a summary
  table. If this story changes the source table's schema, the materialized view
  definition may need updating too.
- **Rollup / aggregate tables** (e.g. hourly game aggregates) may need a one-time
  backfill if historical data must reflect the new logic.
- **TTL settings**: new ClickHouse tables should declare a TTL for retention.
- **Primary key / partition key**: ClickHouse queries are fast only when the primary
  key is used in the `WHERE` clause. New queries should be verified against the table's
  `ORDER BY` / `PARTITION BY`.

**What to search for:**

```sh
rg 'MaterializedView\|ENGINE = AggregatingMergeTree\|ENGINE = SummingMergeTree' --type sql
```

---

## Wallet and bonus services

Any story that involves player balance, credits, refunds, or bonus awards touches the
wallet/bonus boundary. These are high-risk because:

- Transactions must be idempotent (deduplication keys are mandatory).
- Balance reads and writes must be consistent — race conditions produce real money bugs.
- Bonus eligibility checks are often separate from the wallet call; both may need updating.

**What to check:**
- Is there an existing wallet client call? Does it include a deduplication / idempotency key?
- Does the story change when/how bonus is awarded? If so, the bonus service contract
  may also need updating.
- Are rollback / reversal paths handled? (Bet placed → game error → refund path)

---

## Idempotent ingestion patterns

ETL jobs and event consumers in this platform are expected to be idempotent:
re-running the job with the same input must produce the same DB state. When scoping a
story that adds a new ingestion path, check:

- Is there a unique constraint or upsert (`ON DUPLICATE KEY` / `ON CONFLICT`) that
  prevents duplicate rows?
- Is there a "processed" flag or watermark table that tracks which records have been
  consumed?
- If the job fails halfway, does re-running it from the checkpoint produce correct output?

---

## Feature flags and system parameters

Configuration that varies by brand or environment is typically stored in a system
parameters service (not env vars). When a story introduces configurable behavior:

- Check whether the parameter already exists: search for the likely key name in the
  codebase before assuming it needs to be created.
- New parameters need to be seeded with a safe default for all existing brands.
- Flag-gated rollouts: if the story is rolling out gradually, verify the flag check
  is in the right layer (service, not just UI).

---

## Desktop (Tauri / Rust + Svelte)

Otto's desktop app is a Tauri application. Stories touching the app involve:

- **Rust backend**: `src-tauri/src/` — Tauri commands registered with `#[tauri::command]`,
  invoked from the frontend via `invoke()`
- **Svelte frontend**: `apps/desktop/src/` — Svelte components, stores, routes
- **IPC boundary**: any new command added to Rust must be declared in `invoke_handler`
  registration (`main.rs` or `lib.rs`) and typed on the frontend side
- **State**: Tauri `State<>` managed objects are singletons per app instance; concurrent
  access from multiple commands needs interior mutability (`Mutex`, `RwLock`)

When a story adds a new Tauri command, check:
1. Is it registered in the `invoke_handler`?
2. Is there a matching TypeScript type / wrapper on the frontend?
3. Does it need to be exposed in tests (unit or integration)?
