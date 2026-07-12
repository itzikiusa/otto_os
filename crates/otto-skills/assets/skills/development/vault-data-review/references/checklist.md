# Data reviewer checklist and output

Check ownership/tenant/namespace/grain; every known field and type/default;
primary/unique/secondary indexes, partitioning, TTL/retention; concrete reads
and writes with predicates/order/limits/mutations and callers; transactions,
atomicity, consistency, locking, retries, and merge semantics; joins and
derivations; field-level reader/writer/change impact; realistic examples; and
schema/query/caller citations. For Redis include exact key pattern/type/encoding,
expiry and lifecycle. A DAO/table name alone is shallow.

Return only `[]` or objects with `severity`, `category`, `summary`, `evidence`,
`missed_item`, `required_fix`. Evidence is an array of repository locations
(`repo_path`, positive `line`) and documentation locations (`doc_path`,
`section`). Category is `data` or `coverage`; severity is
`blocking`, `major`, or `minor`. Do not flag an evidence-backed explicit
no-write/no-TTL statement.
