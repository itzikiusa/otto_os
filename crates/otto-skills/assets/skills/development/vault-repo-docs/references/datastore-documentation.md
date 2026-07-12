# Datastore documentation completion contract

Start from migrations/schema definitions, then trace every query/access helper
and its callers. Search raw SQL, builders/ORMs, stored procedures, key builders,
collection names, broker-backed projections, and tests.

For each relational table, collection, stream, or key pattern document:

- store/tenant/namespace, purpose, ownership, and grain;
- every known field: source type, nullability/default, semantic meaning;
- primary/unique/secondary indexes, partitioning, TTL/retention;
- concrete reads and writes with operation, predicates, ordering, limits,
  mutations, caller flow, and `file:line` evidence;
- transaction, atomicity, consistency, locking, merge, and retry semantics;
- joins/relationships and upstream/downstream derivations;
- field-level impact: which callers read/write each field and what changing it
  would affect;
- realistic query/document/key examples with secrets removed.

Never call a DB section complete after listing table names or DAO methods.
Dynamic SQL and `SELECT *` require tracing destination structs plus schema.
For Redis, include exact key pattern, type, fields/value encoding, expiry,
creation/read/delete paths, and scan behavior. For analytical stores, include
engine/merge semantics and whether queries require a final/dedup step.
