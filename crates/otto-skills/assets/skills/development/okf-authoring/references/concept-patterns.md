# Concept patterns

Choose the narrowest concept that a reader can name and link. Start with `# Overview`; add the applicable contract below; finish factual concepts with `# Citations`.

## Pattern routing

| Concept | Typical `type` | Required content |
|---|---|---|
| Service | `Service` | Responsibility, boundaries, entry points, dependencies, auth, configuration, failure behavior, operations |
| Endpoint | `API Endpoint` | Authentication, parameters, request, success response, errors, validation, side effects, flow, citations |
| Runtime flow | `Flow` | Trigger, ordered stages, dependencies, state changes, retries/idempotency, failures, external effects |
| Datastore | `Data Asset`, `Database Table`, `Collection`, `Redis Key` | Grain/purpose, fields, access, indexes/TTL, transactions, relationships, impact, examples, citations |
| Runbook | `Runbook` | Trigger/symptoms, prerequisites, diagnosis, safe actions, verification, rollback/escalation |
| ADR | `Decision` | Context, options, decision, consequences, status/date |
| Metric | `Metric` | Definition, formula, grain/window, source, dimensions, exclusions, interpretation |
| Shared fact | `Reference` | One cited reusable topic that at least two concepts need |

## API endpoint contract

Document one operation per concept. Include:

1. Method/path or RPC operation and purpose.
2. Authentication and authorization.
3. Path, query, and material headers, including validation/defaults.
4. Request schema/body plus a realistic body example; state explicitly when absent.
5. Success status, response schema/body, and realistic example.
6. Material error statuses with response bodies and trigger conditions.
7. Validation, side effects, runtime-flow link, and source citations.

Naming a request/response DTO is not a substitute for showing its known fields and an example. Keep OpenAPI and prose contracts consistent when OpenAPI exists.

## Datastore contract

State the grain first: for example, “One row per accepted order.” Then include:

- Full known fields/types/descriptions; mark unresolved fields unknown.
- Every evidenced read and write, with actual query/key/access patterns.
- Indexes and TTL/retention, including explicit absence when verified.
- Transaction, isolation, atomicity, merge, or consistency behavior.
- Joins, relationships, partitioning, and ownership boundaries.
- Field-level impact paths: which operation reads/writes which fields and downstream effects.
- Realistic query/payload examples, useful diagrams, and source citations.

## Runtime flow contract

Name the trigger and trace the ordered path across handlers, services, stores, messages, workers, and external clients. Record state transitions, retries, timeouts, idempotency, error recovery, schedules, and side effects. Link the endpoint and datastore concepts instead of duplicating their contracts.

## Reference minting gate

Create a `Reference` only when all four conditions hold:

1. The topic has a stable, nameable identity.
2. It is not bundle metadata such as overview, FAQ, or changelog.
3. Prose can naturally cite it: “See the X reference for …”.
4. At least two concepts need it.

Keep metric formulas in `references/metrics/<slug>.md`. Keep reusable joins in `references/joins/<a>__<b>.md`, alphabetizing names and showing the exact `ON` clause once.
