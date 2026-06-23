# Example — "service A calls B, and B does 10 things"

A worked, copy-pasteable walkthrough: the agent receives a request to visualize a
service call where the callee fans out into many sub-steps, and produces an
**exhaustive** sequence diagram on a new canvas scene — every one of B's 10 steps
as a distinct self-message, nothing hidden behind a single "B processes" box.

Run the commands from this skill's directory (so `scripts/canvas.mjs` resolves).
Assumes `OTTO_API_TOKEN` (and optionally `OTTO_BASE`) are exported and you have a
workspace id.

---

## 0. Setup — get a workspace id

```bash
export OTTO_API_TOKEN=…                  # from the otto-api skill
WID=$(otto ws-id)                        # first workspace (otto-api client)
node scripts/canvas.mjs whoami           # sanity-check the token
```

## 1. Create a scene

`create-scene` prints the full `CanvasScene`; capture its `id`.

```bash
SID=$(node scripts/canvas.mjs create-scene "$WID" "Checkout: A → B fan-out" \
      | sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' | head -1)
echo "scene = $SID"
```

## 2a. Let the agent draft the diagram (recommended)

Steer it to a sequence diagram with `mode = sequence`. `assist` **returns** the
mermaid (it does NOT save it):

```bash
node scripts/canvas.mjs assist "$SID" \
  "Service A calls service B once. B then does 10 things in order before replying. Show all 10 as distinct steps." \
  sequence
```

The result looks like `{ "mermaid": "sequenceDiagram\n …", "nodes": [], "edges": [], "note": "…" }`.
Take the `mermaid` string and persist it (step 3).

## 2b. …or write the mermaid yourself

The fan-out pattern: one numbered **self-message** (`B->>B:`) per sub-step, so all
ten are visible. (Cheat sheet: `references/mermaid-cheatsheet.md`.)

```
sequenceDiagram
  autonumber
  participant A as Service A
  participant B as Service B
  A->>B: charge(order)
  activate B
  B->>B: 1. authenticate caller
  B->>B: 2. validate order payload
  B->>B: 3. idempotency check
  B->>B: 4. load customer + account
  B->>B: 5. risk / fraud score
  B->>B: 6. reserve funds
  B->>B: 7. persist transaction
  B->>B: 8. emit "charged" event
  B->>B: 9. update cache
  B->>B: 10. write audit log
  B-->>A: 200 { txId }
  deactivate B
```

## 3. Add the diagram to the scene

`add-mermaid` does the read-modify-write (GET → parse `doc_json` → append a
`mermaid` node → PUT). It takes a raw multi-line string:

```bash
node scripts/canvas.mjs add-mermaid "$SID" "sequenceDiagram
  autonumber
  participant A as Service A
  participant B as Service B
  A->>B: charge(order)
  activate B
  B->>B: 1. authenticate caller
  B->>B: 2. validate order payload
  B->>B: 3. idempotency check
  B->>B: 4. load customer + account
  B->>B: 5. risk / fraud score
  B->>B: 6. reserve funds
  B->>B: 7. persist transaction
  B->>B: 8. emit \"charged\" event
  B->>B: 9. update cache
  B->>B: 10. write audit log
  B-->>A: 200 { txId }
  deactivate B"
```

Output confirms the appended node:
`{ "added_node": "m…", "kind": "sequence", "scene": { "id": "…", "title": "…", "nodes": 1 } }`.

The helper infers `mermaid.kind = "sequence"` from the first line, so the stored
node is:

```jsonc
{
  "id": "m…", "kind": "mermaid",
  "x": 80, "y": 80, "w": 560, "h": 400,
  "mermaid": { "src": "sequenceDiagram\n autonumber\n …", "kind": "sequence" }
}
```

## 4. Verify

```bash
node scripts/canvas.mjs get-scene "$SID"
# → CanvasScene; JSON.parse(doc_json).nodes[0].mermaid.src holds the diagram
```

---

### Why exhaustive matters

Collapsing "B does 10 things" into a single box hides exactly the detail the
reader asked for. The assist prompt is explicit ("if a step fans out into N
sub-steps, emit all N"), and a hand-written diagram should hold the same bar: ten
numbered self-messages, in order, with an `activate`/`deactivate` span so B's work
reads as one bounded operation. To present this step-by-step afterward, see
`examples/sequence-to-slides.md`.
