# Otto School — asset contract

The runtime (`ui/src/modules/home/school/`) loads these files from
`ui/public/school/` (served at `/school/…`). Everything here is a contract:
node names, clip names, origins and facings. `build_school.py --verify` and
`ui/unit/school-assets.test.ts` check it.

Units: metres. Y up. glTF 2.0 binary (`.glb`). At rest every object faces
**+Z** (its "front" — a kid's face, a monitor's display, a poster's print, a
wall's inside surface all point +Z). Origins sit on the floor (y = 0) unless
stated. Animations are **in place** (no root translation; the runtime moves
characters along paths).

## Characters

### `kid-{claude,codex,grok,agy,shell,custom}.glb`

One skinned kid (Quaternius Ultimate Animated Character base, CC0 — kid proportions, 62-joint rig), restyled per provider:

| file | outfit | decal on chest |
|---|---|---|
| kid-claude | warm terracotta/orange hoodie | Claude starburst |
| kid-codex | green hoodie | Codex `>_` cloud |
| kid-grok | black hoodie | "G" / Grok-style slash mark |
| kid-agy | blue hoodie | Antigravity "A" arch |
| kid-shell | grey hoodie | `$_` prompt |
| kid-custom | neutral teal hoodie | none (runtime may tint) |

Each file has every kid a different base face/hair where possible (variety),
standing height ≈ 1.0–1.1 m.

Clips (exact names):

| clip | loop | pose |
|---|---|---|
| `Sit_Type` | yes | seated, both forearms forward on the keyboard, small typing motion, slight head bob |
| `Sit_Idle` | yes | seated, relaxed, occasional lean back |
| `Sit_RaiseHand` | yes | seated, right arm straight up, small wave of the hand |
| `Sit_Slump` | yes | seated, head down, shoulders dropped (detention) |
| `Stand_Up` | once | seated → standing (ends in the `Idle_Stand` pose) |
| `Sit_Down` | once | standing → seated (ends in the `Sit_Idle` pose) |
| `Walk` | yes | walk cycle in place, ≈ 1.0 m/s stride |
| `Idle_Stand` | yes | standing idle |
| `Talk` | yes | standing, gesturing while talking |
| `Look_Around` | yes | standing, turning head left/right |
| `Wave` | yes | standing, waving |

**Seated geometry:** in every `Sit_*` clip, with the kid's origin placed on
the `Chair` node's origin and both facing the same way, the kid's hips rest on
the seat and the hands reach the keyboard of a `Workstation` placed at
`(0, 0, +0.55)` from the chair.

### `headmaster-otto.glb`

The robot headmaster (three.js `RobotExpressive`, CC0, Tomás Laulhé) —
standing height ≈ 1.6 m. Clips: `Idle`, `Walk`, `Nod`, `Scold`, `Wave`,
`ThumbsUp`, `Inspect_Screen` (leans forward ~25°, head down, as if reading a
monitor), `Point_Board` (right arm extended forward-up as if pointing at a
board).

## Kit — `school-kit.glb`

One file, each item a **top-level node with this exact name** (the runtime
clones them). Origins on the floor at the item's footprint centre unless noted.

| node | size (w×h×d) | notes |
|---|---|---|
| `Workstation` | ≈1.2×1.15×0.6 | student desk + PC monitor + keyboard + mouse. Monitor faces **−Z** (toward the kid sitting at −Z). Child mesh **`Screen`**: a flat display surface, UV (0,0)=bottom-left … (1,1)=top-right **as seen by the kid**, aspect 16:10, material named `ScreenMat` |
| `Chair` | ≈0.45×0.85×0.45 | student chair, seat top at y≈0.42, backrest at −Z |
| `TeacherDesk` | ≈1.6×0.78×0.8 | headmaster's desk with a monitor (no live screen) |
| `Board` | ≈4.0×1.4 | green chalkboard + frame + chalk tray, wall-mounted: origin at the wall surface, bottom edge 0.9 m above floor, facing +Z. Chalk art on its left half; child plane **`BoardTitle`** (≈1.8×0.35, UV like `Screen`) top-right for the workspace name |
| `Bookshelf` | ≈1.2×1.8×0.4 | filled with books |
| `Plant` | ≈0.5×1.0×0.5 | potted plant |
| `Clock` | ⌀0.4 | wall clock, origin at wall surface centre |
| `Bench` | ≈1.6×0.45×0.4 | detention bench |
| `Poster_1` … `Poster_6` | ≈0.7×1.0 | framed posters, origin at the wall surface centre |
| `Floor_Tile` | 2×0.02×2 | classroom floor (light blue/white tiles), top at y=0 |
| `Wall` | 2×3×0.15 | plain wall segment; origin bottom-centre of the INSIDE face; inside faces +Z |
| `Wall_Window` | 2×3×0.15 | wall with a window (frame + glass + sky/outdoor view) |
| `Wall_Door` | 2×3×0.15 | wall with a classroom door (door leaf is child **`DoorLeaf`**, hinged on its −X edge, so the runtime can rotate it about Y) |
| `Corridor_Floor` | 2×0.02×2 | corridor floor |
| `Corridor_Wall` | 2×3×0.15 | corridor wall with lockers |
| `Corridor_Door` | 2×3×0.15 | corridor wall with a classroom door (child `DoorLeaf` as above) + child plane **`NamePlate`** (≈1.2×0.3 above the door) and child plane **`DoorSign`** (≈0.5×0.35 beside the door) — both UV like `Screen` |
| `Ceiling_Light` | ≈1.2×0.1×0.3 | panel light (emissive) |
| `Backpack`, `Trashcan`, `Plant_Desk`, `Book`, `Book_Open`, `Water_Bottle` | small | optional decor, floor / desk-top origin, front +Z |
| `Corkboard` | ≈1.2×0.9 | optional, origin at the wall-surface centre |

Optional lighting: `env.hdr` (equirectangular, Poly Haven CC0) — the
runtime uses it as the scene environment when present.

Planes the runtime paints (`Screen`, `BoardTitle`, `NamePlate`, `DoorSign`)
each carry their **own** material instance so replacing `material.map` never
leaks to other meshes.

## Budgets

kid ≤ 8 k tris · headmaster ≤ 12 k · school-kit ≤ 40 k total · textures
≤ 1024² · shipped total (`ui/public/school/`) ≤ 10 MB.

## Manifest — `ui/public/school/manifest.json`

```json
{
  "version": 1,
  "files": {
    "kid-claude.glb": { "bytes": 0, "sha256": "…", "tris": 0, "clips": ["Sit_Type", "…"], "nodes": [] },
    "school-kit.glb": { "bytes": 0, "sha256": "…", "tris": 0, "clips": [], "nodes": ["Workstation", "…"] }
  }
}
```
