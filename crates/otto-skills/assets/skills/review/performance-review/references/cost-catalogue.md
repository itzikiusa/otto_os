# Cost catalogue — what to hunt in each pass

Run one pass per lens. Keep only that lens active and walk the change line by line. For
every hit, write down `n` (where the size comes from) before deciding severity — see
`cost-model-and-severity.md`. This list is a prompt, not a cage; a real cost outside it
still counts.

Data-access cost dominates on this platform (MySQL/`pr_bo`, ClickHouse, Redis, Mongo), so
passes 1–2 are the heaviest and run first.

---

## 1. Data-access shape (N+1, queries-in-loops) — heaviest

- **Query inside a loop.** A `SELECT`/`db.GetContext`/`ExecuteSingleResultQuery` per element
  of a collection. Signature: 1 outer query returns N rows, then N more queries (one per
  row). → collapse to a `JOIN` or `WHERE id IN (…)` / batched fetch.
- **Redis call per item.** `GET`/`HGET` in a loop instead of `MGET`/pipeline/`HMGET`.
- **HTTP/service call per item.** A service-client call (wallet, bonus, currency) per row
  instead of the batch endpoint, if one exists. Check `go_casino_kit/clients` for a bulk form.
- **Lazy-load fan-out.** Accessing a related entity per parent that triggers a hidden fetch
  each time (ORM lazy load, accessor that queries).
- **Re-query per row of what the first query already returned.** Fetching the parent again
  inside the child loop; looking up by key data you already hold.
- **Multi-tenant trap.** `GetBrandConnector` / per-brand or per-player connection acquisition
  inside a loop — connection + query churn multiplied by `n`.
- **Count math:** always state `1 + N` and what `N` is. N≈orders/player, N≈rows/page, etc.

## 2. Indexes & scans

- **New predicate, no index.** A `WHERE` / `JOIN ON` / `ORDER BY` / `GROUP BY` on a column
  with no supporting index → full table scan. **Open the migration** and confirm the index
  ships with the change; "the DBA will add it" is not shipped.
- **Composite-index order.** Filtering on `(player_id, created_at)` but the index is on
  `(created_at, player_id)`, or only `player_id` — won't serve a range/sort efficiently.
- **Leading wildcard / non-sargable.** `LIKE '%x'`, `WHERE DATE(created_at) = …`,
  `WHERE fn(col) = …`, `col + 0 = …` — defeats the index, forces a scan.
- **Large fact tables.** Any scan/sort over `GSS_activities`, `MdlCsh_tblTransactions`,
  `MdlGm_tblPlayers`, or ClickHouse fact tables — these grow without bound; a scan is a
  blocker, not a minor.
- **`SELECT *` on fat rows.** Pulling every column (incl. blobs/JSON) when a few suffice —
  more bytes over the wire and into memory per row.
- **ClickHouse specifics.** Query that ignores the table's `ORDER BY` / partition key →
  reads far more granules than needed; `SELECT *` over a wide columnar table; missing
  `WHERE` on the partition column for time-range queries.
- **Mongo specifics.** A `find`/`sort` with no matching index → COLLSCAN; an unbounded
  `find` with no `limit`; `$regex` without anchor; querying a non-indexed field on a large
  collection.
- **OFFSET pagination at depth.** `LIMIT n OFFSET 100000` re-scans everything skipped —
  slows as users page deeper; prefer keyset/seek pagination.

## 3. Algorithmic cost (accidental O(n²) and worse)

- **Nested scan.** A loop over `a` containing a loop/`.contains`/`.find`/linear search over
  `b` → O(n·m). → index `b` into a set/map for O(1) lookup.
- **`list.contains` / `indexOf` in a loop** over a growing slice → O(n²). Use a `HashSet`.
- **Repeated string concat** in a loop building a big string → O(n²) copies. Use a builder.
- **Re-sort / re-allocate inside a loop** that could be hoisted out.
- **Quadratic dedup/group** done by hand where a single grouping pass would do.
- **Recompute of an invariant** each iteration that the compiler can't hoist.

## 4. Redundant work

- **Re-fetch already in hand.** Querying or calling a service for a value the current
  request already loaded; the same query run twice in one request path.
- **Serialize ↔ deserialize round-trips.** Marshalling to JSON and back to move data between
  layers in-process; converting a struct to a map to a struct.
- **Needless clone/copy.** Cloning a large slice/map/struct to pass it read-only; `to_vec()`
  / `.clone()` / deep-copy where a reference/borrow/slice would do. (Only a finding when
  the data is large *and* the path is hot.)
- **Eager work never used.** Computing/fetching a field, formatting a string, building a
  collection that some branches discard.
- **Whole-collection load to answer a scalar.** Pulling all rows to `len()`/sum/exists when
  the DB can `COUNT`/`SUM`/`EXISTS` it.

## 5. Memory & growth

- **Unbounded accumulation.** A slice/map/buffer/cache that grows with traffic and never
  evicts or resets → memory creep / OOM over uptime. State: grows per what, bounded by what.
- **Load-all instead of stream/page.** Reading an entire large result set / file / response
  into memory when it could be streamed or paged.
- **Per-request large allocation on a hot path.** Allocating a big buffer/map every request
  that could be sized, pooled, or reused.
- **Goroutine/task accumulation.** Spawning per item without bound or join → memory + scheduler
  pressure (this is also a `grill` resource finding — flag the *cost* angle here).

## 6. Blocking & concurrency cost

- **Blocking I/O on a hot path.** A synchronous DB/HTTP/disk call where latency multiplies
  per request; a blocking call inside a request handler that could be batched/deferred.
- **I/O while holding a lock.** A network/DB call inside a mutex-guarded section → serializes
  all callers behind the slowest round-trip.
- **Serial round-trips that could be parallel/batched.** N independent service calls done
  one-after-another; could be one batch call or concurrent fan-out with a bounded pool.
- **Chatty protocol.** Many tiny network calls where the API offers a coarser/bulk form.
- **Lock contention on a hot shared structure.** A global mutex/`SyncedMap` on the request
  path that every request fights over.

## 7. Pagination & batching

- **No `LIMIT` / pagination** on a list endpoint or query whose source table grows → response
  size and cost grow with the data; eventually times out.
- **Per-item writes** where a bulk insert / multi-row `INSERT` / pipeline exists → N writes
  → 1.
- **Fan-out where a batch API exists.** N single-key calls when the service/Redis/DB supports
  multi-key in one round-trip.
- **Unbounded user-controlled page size.** Accepting `pageSize` without a cap → a single
  request can pull the whole table (perf *and* DoS — say so).

## 8. Cache use & misuse

- **Hot cacheable read with no cache.** A frequently-repeated, rarely-changing lookup
  (sysparams, config, currency rates, player tier) hitting the DB every time.
- **Cache with no TTL / no bound.** An in-memory cache that never expires/evicts → memory
  leak and staleness.
- **Stampede risk.** On cache miss, many concurrent requests recompute the same expensive
  value simultaneously (no single-flight / lock).
- **Over-specific key → never hits.** A cache key including a timestamp/request-id/volatile
  field so the hit rate is ~0 — pure overhead.
- **Caching something cheap.** Wrapping a trivial computation in cache machinery costs more
  than it saves.
- **Invalidation that over-fetches.** A write that busts and re-warms far more than it
  changed.

---

## Sizing prompts (use on every hit)

- Where does `n` come from — constant, config, bounded query, unbounded table, user input?
- Per unit: round-trips, complexity class, bytes, or CPU? Show the math.
- Hot, warm, or cold path? (Cold → probably not a finding.)
- Plot it at 10× and 100× — where's the knee?
- What's the worst-case input on *this* platform — the whale player, the biggest brand, the
  10k-row report, `pageSize=max`?
- Does the fix actually lower the cost, and keep results correct?
