# Flows, messaging, workers, and lifecycle

Inventory triggers before writing prose: HTTP/RPC calls, messages consumed,
messages produced, schedules/timers, startup hooks, shutdown hooks, retries,
reconciliation loops, and externally invoked commands.

Each flow includes trigger and preconditions, numbered code path, data read and
written at every step, external side effects, transaction boundaries, failure
and retry paths, idempotency/deduplication, observability, and a verified
diagram. Link the flow to its API, message, worker, and data concepts.

For every message direction document broker/topic/queue, key/partition,
headers, complete payload schema and example, producer/consumer trigger,
delivery guarantees, retry/backoff, poison/dead-letter behavior, ordering,
deduplication, and downstream effects.

For every worker document registration, schedule and timezone, scan scope,
pagination/batching, concurrency/locking, checkpointing, idempotency, failure
recovery, shutdown behavior, and citations. A function named `run` is not proof
that a worker is scheduled; trace the registration site.
