# Technical Risk Catalog

Risks to hunt for actively when scoping a story. Do not just list the heading —
for each risk category, describe what you found (or explicitly confirm you checked
and found nothing). A clean bill of health requires evidence.

---

## 1. Contract and backward-compatibility breaks

**What it is:** A change to a shared type, API response shape, event schema, or
DB column that other consumers already depend on.

**Where to look:**
- Any struct/type that is serialized to JSON or written to a DB column
- HTTP response bodies consumed by external callers or a different service
- Kafka/AMQP/event message schemas (check consumer definitions, not just producer)
- Shared packages / libraries imported by multiple services
- gRPC `.proto` files or OpenAPI specs in the repo

**Evidence to cite:** File path of the shared type + at least one other consumer path.

**Severity signal:** If you can find two or more distinct call sites that depend on a
field you are changing, it is a breaking change unless the modification is purely additive
(adding a new optional field with a safe zero value).

---

## 2. Concurrency and race conditions

**What it is:** Two concurrent operations on shared mutable state that can interleave
and produce incorrect results.

**Where to look:**
- Any field updated by multiple goroutines / threads without a lock or atomic
- Optimistic locking / version columns in DB update queries (missing `WHERE version = ?`)
- Cache invalidation: update DB then delete cache vs. delete cache then update DB
- Idempotency: what happens if the same request arrives twice (network retry, user
  double-click, at-least-once delivery)?

**Evidence to cite:** The specific update path and whether a lock/transaction/version
check exists.

---

## 3. Performance hot paths

**What it is:** A code path that will be called at high frequency or on large data sets,
where a suboptimal implementation causes latency spikes, CPU saturation, or OOM.

**Where to look:**
- N+1 query patterns: a loop that issues one DB query per iteration
- Missing indexes on columns used in `WHERE`, `JOIN ON`, or `ORDER BY` clauses
- Unbounded scans: `SELECT *` without `LIMIT` on large tables
- Synchronous operations on the request path that could be async (e.g., sending email
  inline, computing an expensive aggregate)
- ClickHouse queries that don't use the primary key / partition key

**Evidence to cite:** The query or loop path + estimated row count / call frequency.

---

## 4. Security and trust boundaries

**What it is:** A path where insufficiently validated input, missing authorization, or
insecure data handling could expose data or allow privilege escalation.

**Where to look:**
- Input validation: are all user-supplied values sanitized before use in queries?
- Authorization: does every handler verify the caller has access to the resource (not
  just that they are authenticated)?
- Tenant isolation: can a caller of brand A reach data of brand B? (critical in
  multi-tenant systems — see platform-notes.md)
- Secrets/PII in logs: does any new logging statement print passwords, tokens,
  full PAN numbers, or personal data?
- SQL injection: parameterized queries vs. string interpolation

**Evidence to cite:** The handler or query where the check does or does not exist.

---

## 5. Multi-tenancy isolation

**What it is:** In multi-tenant platforms, data from one tenant leaking to another
tenant — either via a missing `brand_id` / `tenant_id` filter or via a shared cache key
that is not namespaced.

**Where to look:**
- Every DB query: does it include a `brand_id` / `tenant_id` filter?
- Cache keys: are they prefixed with the tenant identifier?
- Background jobs: do they scope to one tenant per invocation, or do they process all
  tenants with correct isolation?
- Service discovery / routing: does every outbound call carry the tenant context?

**Evidence to cite:** The query or cache key pattern and whether the tenant scope is present.

---

## 6. Data migrations and backfills

**What it is:** Schema or data changes that require a migration to run — and whether
that migration is safe to apply without downtime.

**Where to look:**
- New NOT NULL columns without a default: will break existing rows until backfill runs
- Renamed columns: old code and new code must both work during rolling deploy
- Index creation on large tables: can lock writes depending on DB engine / mode
- Backfill volume: how many rows? At what write rate? Is there a runbook?
- Rollback: can the migration be reversed without data loss?

**Evidence to cite:** The migration file path and the table + estimated row count.

---

## 7. Idempotency

**What it is:** An operation that must produce the same result if called more than once
(retries, at-least-once delivery, user double-submit).

**Where to look:**
- Financial / wallet transactions: is there a deduplication key checked before applying?
- Event consumers: does processing the same event twice create a duplicate record?
- Job / ETL runs: if the job crashes halfway, does re-running it produce correct output?

**Evidence to cite:** The idempotency key and where it is checked (or the absence of such a check).

---

## 8. Retention, archival, and compliance

**What it is:** New data that is subject to a retention policy, GDPR deletion, or
audit requirement that the story does not mention.

**Where to look:**
- PII fields on new tables: is there a retention TTL or deletion hook?
- Audit logs: does the story create an action that should be logged for compliance?
- ClickHouse TTL: if writing to analytics, is a TTL configured?
- Right-to-erasure: if a player can be deleted, does the new data get cleaned up?

**Evidence to cite:** The column/table and any existing retention mechanism in similar tables.

---

## 9. Dependency and third-party availability

**What it is:** The story assumes an internal or external service, library version, or
infrastructure component exists or behaves in a specific way.

**Where to look:**
- New service-to-service calls: does the target service have the endpoint yet?
- Library upgrades: does the story require a newer API than the pinned version?
- External APIs: rate limits, SLA, authentication changes
- Feature flags or config values: are they already defined in the config system?

**Evidence to cite:** The import path / client call site and the current pinned version or endpoint status.

---

## 10. Observability gaps

**What it is:** A new code path that has no logging, metrics, or tracing — making it
impossible to debug in production.

**Where to look:**
- New endpoints: are they included in existing request-logging middleware?
- New background jobs: do they emit structured logs on start, completion, and error?
- New DB queries in hot paths: are slow-query logs sufficient, or is explicit timing needed?
- Error paths: are errors logged with enough context (brand_id, request_id, player_id)?

**Evidence to cite:** The new path and whether the surrounding code has observable instrumentation.
