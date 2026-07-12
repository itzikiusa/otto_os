# Data reviewer checklist and output

Check ownership/tenant/namespace/grain; every known field and type/default;
primary/unique/secondary indexes, partitioning, TTL/retention; concrete reads
and writes with predicates/order/limits/mutations and callers; transactions,
atomicity, consistency, locking, retries, and merge semantics; joins and
derivations; field-level reader/writer/change impact; realistic examples; and
schema/query/caller citations. For Redis include exact key pattern/type/encoding,
expiry and lifecycle. A DAO/table name alone is shallow.

Return only `[]` or objects with `severity`, `category`, `summary`, `doc`,
`source`, `evidence`, `repair`. Category is `data` or `coverage`; source must be
`path:line`. Do not flag an evidence-backed explicit no-write/no-TTL statement.
