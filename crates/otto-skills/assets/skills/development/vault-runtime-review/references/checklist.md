# Runtime reviewer checklist and output

Check every trigger and registration; preconditions; numbered call path; data
read/written per step; calls/events/cache/files/balance side effects; transaction
boundaries; errors, retries/backoff, poison/dead-letter handling; idempotency and
deduplication; ordering/concurrency/locking; observability; and graceful stop.
For workers check schedule/timezone, scan scope, batching/pagination,
checkpointing, overlap protection, recovery, and shutdown. For messages check
topic/queue, key/partition, headers, complete payload, delivery semantics, and
producer/consumer links.

Flow-note depth is in scope: each step names the store it touches as engine +
table/collection/key-pattern (not "the DB" or a bare service name — trace which
engine actually backs the call), HTTP-triggered flows carry request and
response examples, and the diagram shows every store and external service under
its real name. A step that reads via an intermediary service still documents
the underlying store when the source reveals it.

Return only `[]` or objects with `severity`, `category`, `summary`, `evidence`,
`missed_item`, `required_fix`. Evidence is an array of repository locations
(`repo_path`, positive `line`) and documentation locations (`doc_path`,
`section`). Category is `runtime` or `coverage`; severity is
`blocking`, `major`, or `minor`. Do not infer a schedule from an unregistered
worker function.
