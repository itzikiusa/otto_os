---
description: Use when creating or editing a 3D SCENE for an Otto Product story from an agent session — the Design tab's `scene3d` format (a `scene.json` file the arena's three.js viewport renders and the inspector round-trips) — or when asked for a game-level blockout, a product shot, a kiosk/booth/room layout, a 3D concept for a story. Covers the exact `otto-scene3d` v1 schema with a full example, blockout conventions (metres, y-up, floor at y=0), lighting recipes, naming/grouping rules, the validation limits, how GLB models are referenced (attachment_id, never a URL), and when to prefer the Blender MCP tools / the server-side Blender render+script export instead of editing the file.
category: development
version: 1
---

# Otto Design — 3D scenes (`scene3d`) — in-place agent

Otto's **Product → Design** tab has a game-studio side: a three.js **viewport**
(orbit, grid, gizmo), a **Hierarchy** and an **Inspector**. Its document format is
**`scene3d`** — a small, declarative JSON file you can write by hand. You are the
agent that creates/refines it **in place**: the assist prompt names the file
(**`scene.json`**), includes its current contents, and you edit it. On every save
Otto validates the file, re-renders the viewport live and broadcasts
`mockup_updated`; when the turn ends it commits the file as a `kind:"design"`
attachment (`format:"scene3d"`, mime `application/vnd.otto.scene3d+json`).

Reply with ONE short sentence describing what you changed — the file is the
artifact. Keep the file **always valid JSON** and always a **complete document**
(rewrite the whole file, never a fragment), so every intermediate save renders.

## The schema — `otto-scene3d` version 1

```jsonc
{
  "type": "otto-scene3d", "version": 1,          // both REQUIRED, exactly these values
  "background": "#0f172a",                        // "#rrggbb"; optional (default dark slate)
  "grid": true,                                   // editor grid; hidden in Play; optional (default true)
  "camera": { "position": [6, 5, 8], "target": [0, 1, 0], "fov": 50 },   // fov in degrees; near/far optional
  "lights": [
    { "id": "sun", "name": "Sun", "type": "directional",
      "position": [5, 10, 5], "target": [0, 0, 0], "intensity": 1.2, "color": "#ffffff", "shadow": true },
    { "id": "amb", "name": "Ambient", "type": "ambient", "intensity": 0.4 }
  ],
  "objects": [
    { "id": "floor", "name": "Floor", "type": "plane",
      "position": [0, 0, 0], "rotation": [-90, 0, 0], "scale": [20, 20, 1],
      "material": { "color": "#334155", "roughness": 0.9 } },
    { "id": "crate", "name": "Crate", "type": "box",
      "position": [0, 0.5, 0], "rotation": [0, 30, 0], "scale": [1, 1, 1],
      "material": { "color": "#f59e0b", "metalness": 0.1, "roughness": 0.6 },
      "notes": "Hero prop — swap for the real model once the GLB is uploaded" },
    { "id": "hero", "name": "Hero", "type": "gltf", "attachment_id": "01J9…",   // an uploaded .glb attachment id
      "position": [2, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1] }
  ],
  "groups": [ { "id": "props", "name": "Props", "children": ["crate", "hero"] } ]
}
```

### Objects

| field | type | notes |
|---|---|---|
| `id` | string | **required**, unique across objects+lights+groups, `[A-Za-z0-9_-]{1,128}`. Stable — the user's selection and notes hang off it, so never rename ids when you can edit in place |
| `name` | string | shown in the Hierarchy; humans read this, keep it short and specific ("Checkout counter", not "Box 3") |
| `type` | enum | `box` \| `sphere` \| `cylinder` \| `cone` \| `torus` \| `plane` \| `text` \| `gltf` |
| `position` | `[x,y,z]` | **metres**, world space. Default `[0,0,0]` |
| `rotation` | `[x,y,z]` | **degrees** (the viewer converts). Default `[0,0,0]` |
| `scale` | `[x,y,z]` | multiplies a **unit** primitive (see sizes below). Default `[1,1,1]`, never `0` |
| `material` | object | `color` `#rrggbb`, `metalness` 0–1, `roughness` 0–1, `opacity` 0–1 (<1 ⇒ transparent), `emissive` `#rrggbb`, `wireframe` bool. All optional. Ignored for `gltf` (the model carries its own) |
| `attachment_id` | string | **`gltf` only, required for it.** The id of an uploaded `.glb`/`.gltf` attachment on the story. **Never a URL or a path** — the viewer resolves it through Otto's authed attachment route |
| `text` | string | `text` only — the string drawn on a 2 × 0.5 m quad (≤ 500 chars). Colour from `material.color` |
| `visible` | bool | default `true`. Prefer hiding over deleting when iterating on options |
| `notes` | string | free-form design intent / TODO shown in the Inspector (≤ 4 000 chars). Read the user's notes before editing an object — they are instructions to you |

**Unit primitive sizes** (so `scale` = real size in metres): `box` 1×1×1 centred on
its origin (so a box resting on the floor has `position.y = 0.5 × scale.y`);
`sphere` ⌀1 (radius 0.5); `cylinder` ⌀1 × 1 high, centred; `cone` ⌀1 base × 1 high,
centred; `torus` ring radius 0.4, tube 0.12; `plane` 1×1 facing **+Z** — a floor is
a plane with `rotation: [-90, 0, 0]`; `text` 2 × 0.5 quad facing +Z.

### Lights

`type` ∈ `directional` | `ambient` | `point` | `spot` | `hemisphere`. Fields:
`id` (required, same rules), `name`, `position` (not for ambient/hemisphere),
`target` `[x,y,z]` (directional/spot aim point), `intensity` (≥ 0; ambient 0.2–0.6,
sun 0.8–2.5, point/spot 5–50 because they fall off with distance),
`color`, `ground_color` (hemisphere only), `distance` (point/spot cutoff, 0 = ∞),
`angle` (spot cone, degrees, ≤ 90), `shadow` (bool — one shadow-caster is usually
enough; more cost frame rate), `visible`, `notes`. Max 64 lights.

### Camera, groups, limits

- `camera`: `position`, `target`, `fov` (1–179°, 35–60 is natural; 24–30 for a
  product shot), optional `near`/`far`. Play mode and the Blender export look
  through it — frame the thing that matters.
- `groups`: `{ id, name, children: [ids], visible?, notes? }`. Purely
  organisational (no transform of their own); a node belongs to at most one
  group; groups may nest by listing another group's id; lights cannot be grouped.
- **Hard limits** (validation rejects the save): ≤ 2 000 objects, ≤ 64 lights,
  ≤ 500 groups, every number finite, |coordinate| ≤ 100 000, unknown `type`s,
  duplicate ids, non-`#rrggbb` colours, non-safe `attachment_id`. Unknown extra
  keys are dropped silently — do not invent fields.

## Blockout conventions (follow these unless the user says otherwise)

1. **Metres, y-up, floor at y = 0.** Human scale: door 0.9 × 2.1, counter height
   0.9, table 0.75, chair seat 0.45, ceiling 2.7–3.2, corridor ≥ 1.2, car ≈ 4.5 × 1.8 × 1.5.
   Objects **rest on** the floor (`position.y = half height × scale.y`) — nothing
   floats or sinks unless intended.
2. **Start with the envelope**: floor plane, then walls/large masses, then
   furniture/props, then details. A blockout is 10–40 primitives, not 400 — use
   primitives to communicate volume and flow, not to model detail (that is Blender's job).
3. **Grey-box first, colour for meaning.** Neutral greys for structure
   (`#475569` … `#94a3b8`), one accent per functional zone (entrance, hero
   product, hazard), warm `#f59e0b`/`#fb923c` for interactive/hero, cool
   `#38bdf8`/`#818cf8` for screens/UI. Emissive + low roughness = a screen.
4. **Name for the Hierarchy**: `Floor`, `Wall N`, `Counter`, `Kiosk screen`,
   `Player spawn`, `Checkpoint 2`. Group by area (`Lobby`, `Props`, `Lighting rig`
   is NOT a group — lights are listed separately).
5. **Labels in-world**: a `text` object above a zone (`position.y ≈ 2.2`,
   `rotation: [0, <face the camera>, 0]`) reads better than a note nobody opens.
6. **Camera last**: place it where the reviewer should stand; eye height 1.6–1.8
   for walkthroughs, a ¾ high angle (`[8, 6, 10]` → `[0, 1, 0]`) for overviews.

## Lighting recipes

| goal | lights |
|---|---|
| Neutral studio (default) | `directional` `[5,10,5]`→`[0,0,0]` 1.2 shadow + `ambient` 0.4 |
| Outdoor day | `directional` (sun) `[20,30,10]` 2.0 `#fff4e0` shadow + `hemisphere` sky `#bfdbfe` / ground `#4b5563` 0.6 |
| Night / moody | `directional` 0.3 `#93c5fd` + `point` lamps 8–20 `#fbbf24` `distance` 6–10 + `ambient` 0.15 |
| Product shot | 3-point: key `spot` `[3,4,3]` 25 `angle` 35 shadow, fill `point` `[-3,2,2]` 8, rim `spot` `[0,3,-4]` 15; `ambient` 0.2; dark `background` `#0b1220`; camera fov 30 |
| Game level readability | `hemisphere` 0.8 + one `directional` shadow-caster; keep `roughness` ≥ 0.6 so surfaces don't sparkle |

Tone mapping is ACES, so intensities above ~3 on a sun start to clip whites.

## Workflow

1. **Read the current file and the notes** — `notes` on objects are the user's
   instructions; the Inspector shows them, the user expects you to honour them.
2. **Edit in place.** Keep ids; change transforms/materials; add new objects with
   fresh, descriptive ids; use `visible:false` to park alternatives. Preserve
   objects you were not asked to touch (including `gltf` references you cannot see).
3. **Sanity pass before saving**: valid JSON; `type`/`version` present; every id
   unique; no `scale` component `0`; every group child exists; planes used as
   floors are rotated `[-90,0,0]`; the camera actually looks at the content.
4. **Never emit URLs.** A model comes in as an upload (the user or Otto attaches
   a `.glb`, you get its `attachment_id`). If the user asks for a model you don't
   have, blockout it with primitives and leave a `notes` TODO naming the asset.

## Blender: when to reach for it

The arena renders `scene3d` without Blender. Use Blender only for things
primitives cannot do — real meshes, materials/UVs, a photoreal render:

- **Blender MCP tools present** in your session (an MCP server named *Blender*,
  tools like `execute_blender_code`, `get_scene_info`, `import/export`)? Prefer
  them for modelling/rendering work: build the scene there, **export a `.glb`**,
  have it uploaded as an attachment, and reference it from `scene.json` as a
  `gltf` object so the arena keeps a live, editable view. Keep `scene.json` the
  source of truth for layout; the GLB is an asset inside it.
- **No MCP** (the usual case): edit `scene.json`. Otto can then, on the user's
  request, **generate a Blender script from the validated file** (the server does
  this — `…/design/{aid}/blender-script` download, or `…/blender-render` when a
  local Blender is detected, producing `render.png` + `scene.glb` attachments).
  You never write Python for this; you never attach `.py` files — it is not an
  accepted format.

## Related

- 2D screens/boards/diagrams → skill **`otto-design-2d`** (`design.html`,
  `design.excalidraw`, `design.mmd`).
- The general Design-tab mechanics (formats, files, how the turn commits) →
  skill **`otto-mockup`**.
