# Example — build a sequence scene, then present it as slides

A worked walkthrough: create a scene with a sequence diagram, then add a slide so
the scene can be presented — first as a single "show everything" slide (the easy
path, what `canvas.mjs add-slide` does), then as a **step-by-step playback** of
the sequence's messages (the manual read-modify-write path).

Run from this skill's directory with `OTTO_API_TOKEN` exported.

---

## 1. Build the sequence scene

```bash
WID=$(otto ws-id)
SID=$(node scripts/canvas.mjs create-scene "$WID" "Login flow" \
      | sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' | head -1)

node scripts/canvas.mjs add-mermaid "$SID" "sequenceDiagram
  autonumber
  participant U as User
  participant S as Auth Service
  participant DB as Database
  U->>S: POST /login
  S->>S: 1. validate credentials
  S->>DB: 2. lookup user
  DB-->>S: user
  S->>S: 3. check lockout
  S->>S: 4. issue session token
  S-->>U: 200 { token }"
```

Note the appended node id — `add-mermaid` prints `added_node`. Call it `M1` below.

## 2. The easy slide — reveal the whole scene

`add-slide` appends a slide whose single `reveal` step exposes every node at once:

```bash
node scripts/canvas.mjs add-slide "$SID" "Login walkthrough"
# → { "added_slide": "s…", "reveals_nodes": 1, "scene": { "id": "…", "slides": 1 } }
```

Resulting slide in `doc.slides`:

```jsonc
{ "id": "s…", "title": "Login walkthrough", "reveal": [ { "nodeIds": ["M1"] } ] }
```

That's enough to present the scene. For message-by-message playback of the
sequence, build the slide manually (next).

## 3. Step-by-step playback of the sequence

A slide can step **through a mermaid node's messages** by setting `mermaidNodeId`
and giving one `RevealStep` per `mermaidMessageRange` (inclusive `[from, to]`, the
message index in the diagram). There's no `canvas.mjs` subcommand for this — do
the read-modify-write yourself.

The diagram above has 7 messages (0-indexed): `0 POST /login`, `1 validate`,
`2 lookup`, `3 user (reply)`, `4 check lockout`, `5 issue token`, `6 200 {token}`.
A slide that reveals them progressively (1 → 3 → 5 → 7 messages):

```bash
# Fetch, mutate, PUT back — using the otto-api client + python3 for the JSON edit.
otto GET /canvas/scenes/$SID | python3 - "$SID" <<'PY'
import json, sys, subprocess
scene = json.load(sys.stdin)
sid = sys.argv[1]
doc = json.loads(scene["doc_json"])
mnode = next(n["id"] for n in doc["nodes"] if n["kind"] == "mermaid")
doc["slides"].append({
    "id": "play1",
    "title": "Login — step by step",
    "mermaidNodeId": mnode,
    "reveal": [
        {"mermaidMessageRange": [0, 0]},   # the request
        {"mermaidMessageRange": [0, 2]},   # + validate, lookup
        {"mermaidMessageRange": [0, 4]},   # + reply, lockout
        {"mermaidMessageRange": [0, 6]},   # + token, response
    ],
})
body = json.dumps({"doc": doc})
# PUT it back via the otto client (reads body from stdin).
subprocess.run(["otto", "PUT", f"/canvas/scenes/{sid}", "-"], input=body, text=True, check=True)
print("added step-playback slide 'play1' with", len(doc["slides"]), "slide(s) total")
PY
```

The stored slide:

```jsonc
{
  "id": "play1",
  "title": "Login — step by step",
  "mermaidNodeId": "M1",
  "reveal": [
    { "mermaidMessageRange": [0, 0] },
    { "mermaidMessageRange": [0, 2] },
    { "mermaidMessageRange": [0, 4] },
    { "mermaidMessageRange": [0, 6] }
  ]
}
```

In the Canvas Studio presenter, advancing the slide widens the visible message
range, so the sequence "draws itself" one beat at a time.

## 4. Verify

```bash
node scripts/canvas.mjs get-scene "$SID"
# JSON.parse(doc_json).slides — you should see both the "reveal all" slide and "play1"
```

---

### Notes

- `add-slide` is the fast path (reveal everything). Use the manual
  `mermaidMessageRange` form only when you want incremental playback.
- `mermaidMessageRange` indexes the diagram's **messages** in source order, not
  participants or notes; keep ranges inclusive and start from `0`.
- A slide may also scope to a frame via `frameNodeId` (the slide viewport) and
  carry `notes` for the presenter — see `references/scene-schema.md`.
