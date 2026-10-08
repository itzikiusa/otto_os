# Rooms Arcade original art

A compact, cohesive hard-surface art kit for Arena Duel and Circuit Clash. The
fighters are original armored arena soldiers; the kart drivers are original
fox, panda, rabbit and robot characters. These are intentionally stylized small-web-game assets, not a
claim of AAA production scale.

Every shipped mesh and material is authored by `build_assets.py` and
`build_roster.py` in Blender.
There are no downloaded source meshes, texture dependencies, or paid assets.
The forms combine sculpted shells, beveled armor panels, recessed vents,
mechanical joints, wheel spokes, engine components, emissive accents, and
layered foliage. Rigid detail is merged per articulated part to limit object
count. Material slots remain separate for recoloring and emission control.

## Rebuild

Run from the repository root with Blender 5.2:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/build_assets.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/build_roster.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/validate_assets.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python ui/assets-src/room-games/render_heroes.py
```

The first command creates six GLBs, the hash manifest, and front, rear, and environment source previews.
The roster command adds two libraries and four portraits. The validation command re-imports each GLB, checks finite mesh bounds, asset roots, clip
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
`PinkLight` carry emission. Fighters are about 1.85 m high and karts 2.48 m long.
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
The fighter uses rigid articulated object animation rather than a deforming skin.
The karts expose wheel pivots and a driver socket for runtime animation, with no baked clip.

## Visual revision 2

The revised fighters use authored chamfered cross sections for a tapered chest,
angular command helmet, expressive optic eyes, cheek guards, shoulder armor,
thighs and calves. The vented reactor backpack reads from the chase camera.
Karts have swept monocoque bodies, tapered sidepods, nose canards, patterned
rubber tires, brake discs, twin-element wings, diffusers, tail lamps and helmet
rear vents. Tiny tread blocks deliberately use simple faces rather than
expensive bevel geometry.

Paint is less metallic and more saturated to hold its identity under image
based lighting. Ivory and foliage are darker to avoid bleaching under bright
hemisphere lighting. Palm foliage is feathered rather than stacked spheres;
racing barrier markings exist on both sides; the reactor uses a dark core and
individual energy channels instead of a solid glowing cylinder.

Before/after references: `previews/before/fighter-azure.png` versus
`previews/fighter-azure.png`, and `previews/before/kart-azure.png` versus
`previews/kart-azure.png`. Rear detail is visible in `previews/fighter-rear.png`
and `previews/kart-rear.png`. Renderer composition and camera work are separate
from the asset revision; white collision proxies must not cover the artwork.


## Character and adventure expansion

`build_roster.py` creates four original, expressive drivers with separate head
pivots and transparent chooser portraits, plus the country/underwater scenery
library. Karts export a `DriverSocket` and no baked driver. The roster is one
shared GLB, under 1.5 MiB. To render seated attachment checks after both scripts:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --python-expr "import sys;sys.path.insert(0,'ui/assets-src/room-games');import build_roster as R;R.kart_previews()"
```

The fighter revision adds a visible human face, tactical undersuit, utility
belt, ammunition pouches, pack side pockets, chest harness, articulated gloves,
and a low-ready gun pose. Idle, Run, Shoot and Death remain compatible.
