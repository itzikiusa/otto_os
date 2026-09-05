---
description: Use when generating or refining a DESIGN artifact for an Otto Product story from an agent session — the Design tab's in-place "Create with AI" / "Refine" agent — or when a swarm agent publishes a mockup with the `otto-mockup` helper. Covers the FILE-BACKED model with FOUR formats — HTML screen (`design.html`), Mermaid diagram (`design.mmd`), Excalidraw board (`design.excalidraw`) and 3D scene (`scene.json`, `scene3d`) — which file to edit for which ask, the always-valid/whole-file rule, how the file is committed as a `kind:design` attachment and previewed live, the `otto-mockup --format` flags, and where the per-format authoring rules live (`otto-design-2d`, `otto-design-3d`).
category: development
version: 2
---

# Otto Product Design artifacts — in-place agent

Otto's **Product → Design** tab is one arena for every design artifact on a story:
an **Assets** list on the left (Screens · Boards · Diagrams · 3D · Images), the
**viewport/board** in the middle, an **Inspector** and the **Assistant** on the
right. The assistant is you — a specialized agent running **in place** (a live
shell embedded on the Product page, never in the Agents list). Artifacts are
**file-backed**: you EDIT one file the daemon owns, and the committed file becomes
a `kind:"design"` attachment on the story (legacy `kind:"mockup"` rows from the
old Mockups tab keep working and are listed alongside). You refine the SAME file
across the conversation, so follow-ups *change* the artifact instead of
regenerating it.

## The four formats

| format | file you edit | mime | best for | rules |
|---|---|---|---|---|
| `html` | **`design.html`** | `text/html` | high-fidelity UI screens — dashboards, settings, forms, emails; shown in a device frame (iPhone / iPad / desktop), light/dark | `otto-design-2d` § B |
| `mermaid` | **`design.mmd`** | `text/vnd.mermaid` | strict auto-laid-out diagrams — flows, sequences, ER, state machines | below |
| `excalidraw` | **`design.excalidraw`** | `application/vnd.excalidraw+json` | freeform boards — artboards/wireframes, user flows, component sheets, moodboards; the user keeps editing by hand | `otto-design-2d` § A |
| `scene3d` | **`scene.json`** | `application/vnd.otto.scene3d+json` | 3D blockouts — game levels, kiosks/booths/rooms, product shots; three.js viewport + hierarchy + inspector | `otto-design-3d` |

Uploaded files (`.glb`/`.gltf` models, `.png`/`.svg` images, `.excalidraw`) are
attachments too; a `.glb` becomes a `gltf` object inside a `scene3d` document via
its `attachment_id`. **No Python / Blender scripts** are ever authored or uploaded
— Otto generates the Blender export server-side from a validated scene.

The assist prompt tells you **which file to edit** and includes its current
contents. Read it, apply the requested change **in place**, and save. Reply with
ONE short sentence describing what you changed (the file is the artifact; your
prose is just a note). Unknown formats are rejected by the daemon (400) — there is
no silent fallback, so never invent a fifth format.

### Picking a format when the user didn't

- "screen / page / dashboard / what it looks like" → `html`
- "wireframes / flow between screens / board / sketch / artboards / moodboard" → `excalidraw`
- "sequence / ER / state / decision flow" (a diagram, not a UI) → `mermaid`
- "3D / level / layout of a space / kiosk / booth / product shot / blockout" → `scene3d`

## Universal rules (every format)

- **Write the WHOLE file each time.** Never a fragment, never a diff. Every save
  is validated and broadcast (`mockup_updated`) and the arena previews it live —
  keep the file **always valid** so every intermediate save renders.
- **Edit in place, preserve intent.** Keep ids/names the user may have touched in
  the Inspector or board; change only what the request covers; honour per-object
  `notes` (3D) and annotation pins (2D) as instructions.
- **No network, no scripts, no secrets.** HTML runs with scripts disabled in a
  sandboxed iframe; boards embed no base64 images; scenes reference models only by
  `attachment_id`.
- **Realistic content** — real-looking names, numbers, states. No lorem ipsum.

## Mermaid mode — `design.mmd`

The file holds ONE COMPLETE, valid Mermaid diagram (no ``` fences inside the file).
Pick the BEST type: `flowchart TD`/`LR`, `sequenceDiagram`, `classDiagram`,
`erDiagram`, `stateDiagram-v2`. Use short emoji-prefixed labels, rhombus decisions
`B{"❓ Valid?"}` with labelled edges `B -->|yes| C`, `subgraph` lanes, and colour via
`classDef`/`class` at the END:

```mermaid
flowchart TD
  A(["🚀 Start"]) --> B{"❓ Valid?"}
  B -->|yes| C["⚙️ Process"]
  B -->|no| D["❌ Reject"]
  classDef start fill:#dcfce7,stroke:#16a34a,color:#064e3b;
  class A start;
```

Quote labels that contain punctuation, keep node ids ASCII, and prefer ≤ 25 nodes
per diagram (split large flows into several artifacts).

## HTML / Excalidraw / scene3d

The authoring rules are dense enough to have their own skills — load them:

- **`otto-design-2d`** — `design.html` (self-contained, script-free screen;
  light+dark tokens; fits the device frame) and `design.excalidraw` (document
  shape, element fields, frames as artboards, bound labels, bound arrows, the
  8-pt grid, palette, components).
- **`otto-design-3d`** — the exact `otto-scene3d` v1 schema, unit sizes, blockout
  conventions (metres, y-up, floor at 0), lighting recipes, limits, GLB
  references, and when to use the Blender MCP / Blender export instead.

## From a swarm agent — the `otto-mockup` helper

Inside a swarm project cwd, `otto-mockup` publishes a design file into the
project's Product epic (or the discovery run's story) without the in-place shell:

```bash
otto-mockup --title "Checkout flow" --format excalidraw --folder Design < design.excalidraw
otto-mockup --title "Kiosk blockout" --format scene3d < scene.json
```

`--format html|mermaid|excalidraw|scene3d` (default `html`); `--folder` groups it
under the epic like `otto-product --folder`. The helper fails loudly (`curl -f`,
prints the HTTP status, exits non-zero) — if nothing landed, read the error
rather than retrying blindly; a 400 means the format or the file body was
rejected by validation.

## How it's committed

When the turn ends, Otto reads your file back, writes it to the attachment's
storage (`PUT …/attachments/{aid}/content` is the same path the UI's own edits
take), and records the resumable session id + `format` in `meta_json`. The Design
tab lists the artifact under its group; the Assistant panel previews it live as
you write; the Inspector/board edits by the user land in the same file, so
**re-read the current contents at the start of every turn** — they may have
changed since you last saw them.
