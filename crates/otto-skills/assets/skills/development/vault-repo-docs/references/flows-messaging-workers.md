# Flows, messaging, workers, and lifecycle

Inventory triggers before writing prose: HTTP/RPC calls, messages consumed,
messages produced, schedules/timers, startup hooks, shutdown hooks, retries,
reconciliation loops, and externally invoked commands.

## Required flow-note skeleton

Every flow note (frontmatter `type: flow`, stored under `flows/`) fills this
skeleton — the bundle audit enforces the starred sections:

```markdown
# Flow: <name>

**Trigger**: <method + path | job + schedule | topic + consumer> (citation) *

## Steps *
1. <verb + concrete object> — name each store as engine + object
   (`MySQL pr_bo.MdlGm_tblPlayers`, `MongoDB personal_details.db_properties`,
   `ClickHouse players_tmx_data`, `Redis session:{player_id}`), the exact
   external service, and the citation. "the DB" / "a service" is a stub.

## Request example *   <!-- HTTP-triggered flows: body + material headers and
```json … ```               params, or the explicit line "No request body." -->

## Response example *  <!-- HTTP-triggered flows: realistic success body -->
```json … ```

## Failure and retry
<error statuses, retries/backoff, idempotency boundary, poison handling>

<diagram fence — mermaid or d2> *
```

Diagram contract: the trigger node carries the method+path (or job/topic);
every data-store node is labeled engine + table/collection/key-pattern (a
cylinder named `players_tmx_data` without its engine fails the audit whenever
the prose names that engine); every external service appears under its real
name. One diagram that shows where data actually lives beats three generic
ones.

Each flow also documents preconditions, data read and written at every step,
external side effects, transaction boundaries, and observability, and links to
its API, message, worker, and data concepts.

Steps that call into an internal/platform library (a resolved local
dependency) must say what happens INSIDE the call as it matters to this flow —
discovery lookup, pooling, retries, transactionality — cited as
`<dep-repo>:path:line`, with the mention linked to that repo's vault bundle
(forward link if not yet scanned). See
[cross-repo-dependencies.md](cross-repo-dependencies.md).

## Messaging

For every message direction document broker/topic/queue, key/partition,
headers, complete payload schema and example, producer/consumer trigger,
delivery guarantees, retry/backoff, poison/dead-letter behavior, ordering,
deduplication, and downstream effects.

## Workers

For every worker document registration, schedule and timezone, scan scope,
pagination/batching, concurrency/locking, checkpointing, idempotency, failure
recovery, shutdown behavior, and citations. A function named `run` is not proof
that a worker is scheduled; trace the registration site.
