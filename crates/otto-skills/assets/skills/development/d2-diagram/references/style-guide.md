# D2 Style Guide

## Readability first

- A diagram should answer one question.
- Prefer fewer nodes with better labels.
- Use containers to communicate boundaries.
- Use consistent naming: `api`, `auth_service`, `redis`, `clickhouse`, `kafka`.
- Labels can be friendly; IDs should be stable and code-like.

## Edges

Good labels:

- `POST /login`
- `validates token`
- `publishes deposit_approved`
- `consumes tournament_finished`
- `writes login_history`
- `reads session route`
- `returns SSE event`

Weak labels:

- `uses`
- `data`
- `call`
- `thing`

## Styling rules

Use styling sparingly.

Suggested conventions:

- External systems: dashed border.
- Data stores: cylinder or SQL table shapes.
- Queues/topics: queue-like label/container.
- Critical path: slightly stronger stroke.
- Risk/unknown: note/callout with clear label.
- Async edges: dashed edge or label starts with `async:`.

## Naming

Use kebab-case for files and snake_case for node IDs.

```d2
payment_api: "Payment API"
clickhouse_raw: "raw_hourly_transactions"
payment_api -> clickhouse_raw: writes accepted transaction
```

## Human factors

- Put the most important actor on the left/top.
- Put external dependencies outside internal containers.
- Put data stores near services that own/write them.
- Make trust boundaries explicit.
- For onboarding diagrams, include a legend.
