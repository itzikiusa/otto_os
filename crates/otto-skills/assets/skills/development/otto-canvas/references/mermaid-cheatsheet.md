# Mermaid cheat sheet (for canvas diagrams)

A `mermaid` node (`{ kind: "mermaid", mermaid: { src } }`) packs an entire
diagram into one node — the densest, most reliable way to draw on the canvas.
This is a quick syntax reference for the five diagram types Canvas Studio
renders, plus the **fan-out pattern** that keeps complex flows honest.

> Golden rule: **be exhaustive.** If a step fans out into N sub-steps, emit all N
> as distinct messages/nodes — never collapse "B does 10 things" into one box.
> The assist engine's prompt demands this; your hand-written diagrams should too.

---

## Sequence diagram (`sequenceDiagram`) — service-to-service flows

The default choice for "service A calls service B" stories.

```mermaid
sequenceDiagram
  autonumber
  participant A as Service A
  participant B as Service B
  participant DB as Database

  A->>B: POST /charge
  activate B
  B->>B: 1. validate request
  B->>B: 2. idempotency check
  B->>DB: 3. load account
  DB-->>B: account
  B->>B: 4. risk score
  B-->>A: 200 { id }
  deactivate B
```

Arrows: `->>` (solid call), `-->>` (dashed reply), `-x` (lost message), `-)`
(async). Self-message (`B->>B: …`) renders a step on B's own lifeline — **use one
self-message per sub-step** to show fan-out. Wrap a multi-step span in
`activate B` / `deactivate B` (or `B->>B:` lines). Extras: `autonumber`,
`Note over B: …`, `alt/else/end`, `opt/end`, `loop N times … end`, `par … and …
end`, `critical … end`.

### The fan-out pattern ("B does 10 things")

When asked "service A calls service B, and B does 10 things", emit **all 10** as
distinct self-messages on B (numbered), not a single "B processes" box:

```mermaid
sequenceDiagram
  autonumber
  participant A as Service A
  participant B as Service B
  A->>B: request
  activate B
  B->>B: 1. authenticate caller
  B->>B: 2. validate payload
  B->>B: 3. rate-limit check
  B->>B: 4. load config
  B->>B: 5. enrich context
  B->>B: 6. apply business rules
  B->>B: 7. persist record
  B->>B: 8. emit event
  B->>B: 9. update cache
  B->>B: 10. write audit log
  B-->>A: response
  deactivate B
```

(See `examples/service-fanout.md` for the full create-scene + add-mermaid run.)

---

## Flowchart (`flowchart TD` / `LR`) — decisions & branches

```mermaid
flowchart TD
  Start([Start]) --> Check{Authorized?}
  Check -- yes --> Do[Process request]
  Check -- no --> Deny[/Return 403/]
  Do --> Save[(Database)]
  Save --> End([Done])
```

Directions: `TD`/`TB` (top-down), `LR`, `RL`, `BT`. Node shapes: `[rect]`,
`(round)`, `([stadium])`, `{diamond}`, `{{hexagon}}`, `[(database)]`,
`[/parallelogram/]`. Links: `-->`, `---`, `-- label -->`, `-.->` (dashed),
`==>` (thick). Group with `subgraph name … end`.

---

## Class / UML (`classDiagram`)

```mermaid
classDiagram
  class Order {
    +String id
    +Money total
    +submit() bool
  }
  class LineItem {
    +int qty
  }
  Order "1" *-- "many" LineItem : contains
  Order ..|> Payable
```

Relations: `<|--` (inheritance), `*--` (composition), `o--` (aggregation), `-->`
(association), `..>` (dependency), `..|>` (realization). Member prefixes: `+`
public, `-` private, `#` protected.

---

## State diagram (`stateDiagram-v2`)

```mermaid
stateDiagram-v2
  [*] --> Draft
  Draft --> InReview : submit
  InReview --> Approved : approve
  InReview --> Draft : request changes
  Approved --> [*]
```

`[*]` is the start/end pseudo-state. Composite states: `state Name { … }`.
Annotate transitions with `: event`.

---

## Entity-relationship (`erDiagram`)

```mermaid
erDiagram
  CUSTOMER ||--o{ ORDER : places
  ORDER ||--|{ LINE_ITEM : contains
  ORDER {
    string id PK
    datetime created_at
  }
```

Cardinality (left/right): `||` exactly one, `o{`/`}o` zero-or-many, `|{`/`}|`
one-or-many, `o|`/`|o` zero-or-one. Attribute block: `ENTITY { type name PK/FK }`.

---

## Tips for canvas-ready diagrams

- **Prefer `assist`** to draft mermaid from a prompt, then `add-mermaid` the
  returned `src` (set `mode: "sequence"|"flow"|"uml"` to steer the type).
- Keep one diagram per `mermaid` node; size ~`w:520, h:360` and grow `h` for tall
  sequences.
- Set the payload `kind?` hint (`sequence`/`flowchart`/`class`/`state`/`er`) so
  the renderer and slide playback know how to step it.
- For **step-by-step presentation**, sequence diagrams are special: a slide's
  `mermaidMessageRange` reveals messages incrementally (see
  `examples/sequence-to-slides.md`).
- Escape newlines as `\n` when embedding `src` in JSON over HTTP; the `canvas.mjs`
  helper takes a raw multi-line string and encodes it for you.
