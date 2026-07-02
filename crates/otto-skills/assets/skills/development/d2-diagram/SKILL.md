---
description: Use whenever asked to work on a diagram — create, update, improve, review, validate, or render one — including architecture / C4 maps, sequence & API flows, ERDs / SQL schemas, data flows & ETL pipelines, infrastructure / Kubernetes / deployment topology, dependency & module graphs, state machines, incident/debug timelines, and repo-derived diagrams. Produces durable D2 (`.d2`) sources plus rendered SVG in `docs/diagrams/`, with quality linting (d2_doctor) and layout/style rules. For Otto Canvas scenes (a per-scene `canvas.d2`/`canvas.mermaid`/`canvas.json` edited over HTTP), use the `otto-canvas` skill instead.
category: development
version: 1
---

# D2 Diagram Master

> Scope note: this skill is for durable, repo-committed diagram artifacts. When the
> request is about an **Otto Canvas scene** (you are in a scene directory editing
> `canvas.d2` / `canvas.mermaid` / `canvas.json`, or driving the Canvas HTTP API),
> defer to the `otto-canvas` skill — it owns that file-backed model. The D2 syntax,
> style, and quality references below still apply to Canvas D2 scenes.
> Tooling: rendering needs the `d2` CLI (`brew install d2`); `scripts/d2_doctor.py`
> needs python3. Both degrade gracefully when absent — validation/rendering is
> skipped, never faked.

Use this skill whenever the user asks to create, update, review, document, render, or reason about diagrams using D2, including architecture diagrams, sequence diagrams, ERDs, data flows, service maps, deployment diagrams, infrastructure diagrams, dependency diagrams, incident/debug diagrams, state machines, API flows, and repo-derived diagrams.

This skill is optimized for agentic development environments. It should produce diagrams that are correct, readable, version-control friendly, renderable, and useful to engineers.

## Core promise

Always create a durable diagram artifact, not just a sketch. The default deliverables are:

1. `docs/diagrams/<slug>.d2` — source of truth.
2. `docs/diagrams/<slug>.svg` — default rendered output when D2 is available.
3. Optional `docs/diagrams/<slug>.png` or `.pdf` when requested.
4. A short explanation of scope, assumptions, and how to update the diagram.

Never claim that a diagram rendered successfully unless the D2 CLI or equivalent renderer actually produced the output.

## Required workflow

### 1. Understand the diagram intent

Classify the request into one primary diagram type:

- System architecture / C4-style context, container, or component map
- Sequence / request lifecycle / event choreography
- ERD / database schema / SQL model
- Data flow / ETL / analytics pipeline
- Infrastructure / cloud / Kubernetes / deployment topology
- Dependency / module graph / package map
- State machine / lifecycle / workflow
- Incident/debug/troubleshooting timeline
- Security/trust boundary / threat model
- Mixed overview, when the user asks for “everything”

If the user is vague, make a reasonable first diagram and state the assumed scope. Do not block on clarification unless the ambiguity would make the output misleading.

### 2. Discover facts before drawing

When working inside a repo or workspace, inspect relevant files first. Prefer source truth over guesses.

Recommended discovery order:

1. README, docs, architecture docs, ADRs.
2. Entrypoints: `main.go`, `cmd/**`, `server.go`, `Application.java`, `package.json`, `angular.json`.
3. Routing/API definitions: Go routers, Spring controllers, OpenAPI specs, GraphQL schemas.
4. Config: `docker-compose.yml`, Kubernetes manifests, Helm charts, Terraform, env examples.
5. Data contracts: SQL migrations, protobuf, Avro/JSON schemas, ClickHouse DDL, Mongo collection docs.
6. Integration points: Kafka topics, SQS queues, Redis keys, external HTTP clients, DB clients.
7. Tests that reveal flows.

Use `references/repo-discovery.md` for language-specific heuristics.

### 3. Choose the diagram structure

Use `references/diagram-selection.md` to pick the best diagram type and `templates/` as starting points.

Prefer one clear diagram over one giant diagram. If the system is complex, create a set:

- `system-context.d2`
- `service-container.d2`
- `request-sequence.d2`
- `data-model.d2`
- `deployment-topology.d2`

### 4. Write idiomatic D2

Use stable, readable IDs. Keep labels human-friendly. Use containers for bounded contexts, services, clusters, schemas, or trust zones.

Rules:

- Direction first: `direction: right` for system/data flow, `direction: down` for layered stacks.
- Group related nodes inside containers.
- Every important edge must have a label, unless the relationship is visually obvious.
- Prefer verbs on edges: `calls`, `publishes`, `consumes`, `reads`, `writes`, `caches`, `authenticates`, `redirects`.
- Add notes for assumptions, risks, or unknowns.
- Keep line lengths readable.
- Do not over-style. Use styling only to clarify boundaries, risk, async flow, external systems, storage, or critical paths.
- Avoid icons unless the user provided assets or explicitly asks; icons often make diagrams brittle.

Use `references/d2-patterns.md` and `references/style-guide.md`.

### 5. Render-validate and repair

If the D2 CLI is available, always validate by rendering:

```bash
scripts/d2_render.sh docs/diagrams/<slug>.d2
```

Or manually:

```bash
d2 fmt docs/diagrams/<slug>.d2
d2 --layout=dagre docs/diagrams/<slug>.d2 docs/diagrams/<slug>.svg
```

If render fails:

1. Read the error.
2. Fix the D2 source.
3. Render again.
4. Repeat until the diagram renders or report the exact blocker.

Do not stop after a syntax error. Self-heal first.

### 6. Quality review

Before presenting the diagram, check:

- Does it answer the user’s actual question?
- Are system boundaries visible?
- Are external dependencies separated?
- Are async flows distinct from sync calls?
- Are data stores shaped consistently?
- Are risk/unknown areas marked?
- Are edge labels meaningful?
- Would a new engineer understand this in under 60 seconds?
- Is the `.d2` file easy to edit in a PR?

Run the optional doctor:

```bash
python scripts/d2_doctor.py docs/diagrams/<slug>.d2
```

### 7. Output format

When creating files, respond with:

- What was created/changed.
- Render status.
- Key assumptions.
- Any missing facts that would improve the next revision.
- Links or paths to `.d2` and rendered output.

When only returning source in chat, wrap it in a fenced `d2` code block and include render instructions.

## Diagram type rules

### Architecture diagrams

Use for services, bounded contexts, platform topology, and C4-style views.

Best practices:

- Show users/actors, public edge, backend services, data stores, queues, third-party systems.
- Use containers to separate frontend, backend, data, infra, external vendors.
- Mark synchronous vs asynchronous interactions.
- Avoid implementation details unless the user asks for component-level view.

Start from `templates/architecture.d2`.

### Sequence diagrams

Use for login, deposit, payment, bonus grant, tournament win, SSE, Kafka choreography, retry logic, and failure flows.

Best practices:

- Set `shape: sequence_diagram`.
- Declare participants in desired order.
- Show success path first.
- Add grouped sections for retries, errors, idempotency, or async callbacks.
- Use notes for timeouts and consistency assumptions.

Start from `templates/sequence.d2`.

### ERD / SQL diagrams

Use for relational schema, ClickHouse tables, Mongo conceptual collections, and domain entities.

Best practices:

- Use SQL table shapes where useful.
- Show primary keys, important foreign keys, and cardinalities.
- Do not dump every column unless requested.
- For ClickHouse, include engine/order/partition notes when relevant.
- For Mongo, model collections and embedded arrays/documents conceptually.

Start from `templates/erd.d2`.

### Data flow diagrams

Use for ETL, CDC, outbox, Kafka, ClickHouse ingestion, dedup, analytics, observability, and ML pipelines.

Best practices:

- Separate producers, transport, processors, storage, consumers.
- Label batch/streaming edges.
- Show idempotency/dedup points.
- Show replay/retention windows when known.

Start from `templates/dataflow.d2`.

### Infrastructure/deployment diagrams

Use for Kubernetes, Cloudflare, Traefik, AWS, R2/S3, Redis, Kafka/MSK, and runtime topology.

Best practices:

- Separate Internet, edge, cluster, namespace, service, pod, storage, external providers.
- Show ingress path and network boundaries.
- Mark HA/multi-AZ assumptions.
- Include observability/logging paths when relevant.

Start from `templates/infra.d2`.

### Debug/incident diagrams

Use when diagnosing weird production behavior, retries, intermittent empty body, SSE stuck pending, duplicated inserts, etc.

Best practices:

- Show request path and failure points.
- Use numbered steps.
- Mark evidence vs hypothesis.
- Include probes/logs/metrics to verify.

Start from `templates/troubleshooting.d2`.

## Layout selection

Default to `dagre` because it is bundled and fast.

Use `ELK` for dense directed graphs, many ports, complex node-link layouts, or when dagre creates poor spacing.

Use `TALA` only when installed and useful for software architecture diagrams, especially when placement constraints matter. If TALA is not installed, do not fail; render with dagre or ELK and mention that TALA may improve layout.

## D2 generation constraints

- Keep source deterministic; avoid timestamps unless documenting an incident.
- Prefer explicit IDs over labels as IDs.
- Quote labels with spaces or punctuation.
- Keep unknowns visible: use nodes named `unknown_*` or notes labeled `Assumption`.
- Never invent private endpoints, secrets, credentials, or internal names.
- Do not include sensitive values from `.env` files. Redact secrets.
- For generated diagrams from repo analysis, cite or list the files inspected in the final note.

## File naming

Use kebab-case:

- `docs/diagrams/system-context.d2`
- `docs/diagrams/login-sequence.d2`
- `docs/diagrams/clickhouse-dedup-flow.d2`
- `docs/diagrams/cloudflare-request-path.d2`

## Done definition

A diagram task is complete only when at least one of these is true:

1. `.d2` source was created and render validation succeeded.
2. `.d2` source was created, render validation was impossible because D2 is unavailable, and manual render instructions were provided.
3. Existing `.d2` source was reviewed or improved with concrete changes and issues explained.

