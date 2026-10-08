# Rooms Arcade original art

A compact, cohesive hard-surface art kit for Arena Duel and Circuit Clash. The
fighters are original autonomous arena robots; the kart drivers are original
helmeted pilots. These are intentionally stylized small-web-game assets, not a
claim of AAA production scale.

Every shipped mesh and material is authored by `build_assets.py` in Blender.
There are no downloaded source meshes, texture dependencies, or paid assets.
The forms combine sculpted shells, beveled armor panels, recessed vents,
mechanical joints, wheel spokes, engine components, emissive accents, and
layered foliage. Rigid detail is merged per articulated part to limit object
count. Material slots remain separate for recoloring and emission control.

## Rebuild

Run from the repository root with Blender 5.2:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/build_assets.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/validate_assets.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/render_heroes.py
```

The first command creates six GLBs, the hash manifest, and five source previews.
The second re-imports each GLB, checks finite mesh bounds, asset roots, clip
names, zero texture dependencies and the 20 MiB geometry budget, and writes
`validation.json`. The last renders the two chooser images **from shipped
GLBs**, so the hero artwork represents the actual geometry. The pipeline uses
a fixed random seed for rock variation. Blender version can affect export bytes;
`manifest.json` records the exact output hash and size of each delivered GLB.

`CONTRACT.md` documents public node names and usage. `LICENSES.md` documents
provenance. `validation.json` records measured model and per-item bounds. No
.blend or large source archive is necessary: the complete editable source is
the Python authoring script.

## Presentation

Use physically based lighting, a soft hemisphere fill, a directional key and
contact shadows. Azure/Ember are team colors; `CyanLight`, `AmberLight`, and
`PinkLight` carry emission. Fighters are about 1.83 m high and karts 2.51 m long.
Use shared geometry/material clones, and share the environment kit across all
six environments. No texture streaming is needed.

Environment composition remains in the game renderer: station uses bulkheads,
columns and reactor; foundry uses furnaces, pipes and copper machinery; dunes
uses rock formations and arches; coast uses palms and pale rocks; forest uses
pines and broadleaf trees; neon uses luminous towers/signs. Terrain and road
surfaces must agree with the simulation's collision and route definitions.

## Verification limits

Blender imports and hero/source renders were verified. GPU performance, native
pointer lock, gameplay collisions, and networking belong to runtime validation.
The robot uses rigid articulated object animation rather than a deforming skin.
The karts expose wheel and head pivots for runtime animation, with no baked clip.
