# Runtime reviewer checklist and output

Check every trigger and registration; preconditions; numbered call path; data
read/written per step; calls/events/cache/files/balance side effects; transaction
boundaries; errors, retries/backoff, poison/dead-letter handling; idempotency and
deduplication; ordering/concurrency/locking; observability; and graceful stop.
For workers check schedule/timezone, scan scope, batching/pagination,
checkpointing, overlap protection, recovery, and shutdown. For messages check
topic/queue, key/partition, headers, complete payload, delivery semantics, and
producer/consumer links.

Return only `[]` or objects with `severity`, `category`, `summary`, `evidence`,
`missed_item`, `required_fix`. Evidence is an array of repository locations
(`repo_path`, positive `line`) and documentation locations (`doc_path`,
`section`). Category is `runtime` or `coverage`; severity is
`blocking`, `major`, or `minor`. Do not infer a schedule from an unregistered
worker function.
