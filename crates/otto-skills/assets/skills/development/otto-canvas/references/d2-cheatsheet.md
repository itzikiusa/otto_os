# D2 cheat sheet (for canvas.d2 diagrams)

D2 is a declarative diagram language — you describe shapes and connections,
D2's own layout engine (dagre) places them. A `canvas.d2` file is ONE complete
diagram (no ``` fences inside the file). Quick reference for the shapes and
syntax the D2 canvas mode uses most.

---

## Direction

```d2
direction: right   # pipelines / architecture, left→right
# direction: down  # hierarchies / org charts (the default)
```

## Shapes + containers

A bare id declares a shape; giving it children makes it a **container**:

```d2
users: "User"
server: {
  api: "API Gateway"
  db: "Postgres"
}
users -> server.api          # dotted path reaches into a container
server.api -> server.db: query
```

Shape keyword: `shape: <kind>`. Useful kinds beyond the default rectangle:

| kind | use for |
|---|---|
| `sql_table` | schemas — pairs with typed rows (below) |
| `sequence_diagram` | message/call flows between actors |
| `cylinder` | data stores (DB, cache, queue backing store) |
| `queue` | message queues / topics |
| `person` | actors / users |
| `cloud` | external/third-party services |
| `circle`, `diamond`, `hexagon`, `page`, `step` | general shapes |

## Edges + labels

```d2
a -> b: request        # labelled edge
a -> b -> c             # chained
a <-> b                 # bidirectional
a --> b: "async"        # -- / --> for non-arrow / dashed variants also exist
```

## SQL table (schemas)

```d2
users: {
  shape: sql_table
  id: int {constraint: primary_key}
  email: string
  org_id: int {constraint: foreign_key}
}
orgs: {
  shape: sql_table
  id: int {constraint: primary_key}
  name: string
}
users.org_id -> orgs.id
```

## Sequence diagram

```d2
flow: {
  shape: sequence_diagram
  client: "Client"
  api: "API"
  db: "DB"
  client -> api: POST /charge
  api -> db: load account
  db -> api: account
  api -> client: 200 { id }
}
```

## Classes (reusable styles)

Define once, apply everywhere — the cleanest way to colour-code by role:

```d2
classes: {
  start: { style: { fill: "#dcfce7"; stroke: "#16a34a" } }
  error: { style: { fill: "#fee2e2"; stroke: "#dc2626" } }
  data:  { style: { fill: "#ecfeff"; stroke: "#0891b2" } }
}
begin: "🚀 Start" { class: start }
db: "Postgres" { shape: cylinder; class: data }
fail: "❌ Reject" { class: error }
```

## Inline styling (one-off)

```d2
node: "Special case" {
  style.fill: "#eef2ff"
  style.stroke: "#6366f1"
  style.font-color: "#1e1b4b"
}
```

## Icons (optional)

```d2
service: "Auth Service" {
  icon: https://icons.terrastruct.com/essentials/lock.svg
}
```

## Layout hints

```d2
legend: "Legend" { near: top-right }   # pin outside normal layout flow
grid: {
  grid-rows: 2
  grid-columns: 3
  a; b; c; d; e; f                     # children lay out in the grid
}
```

---

## Tips

- Be accurate but keep labels short — long labels blow up node width.
- Prefer `classes` over repeating `style.*` on every node when 3+ shapes share
  a colour (start/process/decision/error, same convention as the Mermaid mode).
- Containers are the natural fit for "lanes" (Client / API / Data) — nest
  related shapes instead of reaching for subgraph-style workarounds.
- `sql_table` + `->` between typed columns is the fastest way to draw a schema
  with visible foreign-key relationships.
- Sketch mode (a UI toggle, not D2 syntax) renders the same diagram hand-drawn
  — no syntax change needed, just a render-time flag.
