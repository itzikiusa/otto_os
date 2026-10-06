#!/usr/bin/env python3
"""Extract the head meshes the kids borrow from other Quaternius characters
(keeps base/ small): each donor .glb is reduced to its head mesh + skin +
materials, animations dropped.

    python3 ui/assets-src/school/prep_base.py <dir with the downloaded .glb files>

Sources (all Quaternius, CC0, via poly.pizza — see LICENSES.md):
  casual.glb   https://poly.pizza/m/kZ3DmIoGip  (Casual Character)
  woman1.glb   https://poly.pizza/m/qJ2gsTUBHL  (Animated Woman)
  woman2.glb   https://poly.pizza/m/nIItLV9nxS  (Animated Woman)
  punk.glb     https://poly.pizza/m/djXoqejw6w  (Punk)
  adventurer.glb https://poly.pizza/m/ZwF0K7WBmu (Adventurer)
"""

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "blender"))
from lib.gltfpatch import Doc  # noqa: E402

HEADS = {"casual.glb": "Casual2_Head", "woman1.glb": "Casual_Head", "woman2.glb": "Formad_Head",
         "punk.glb": "Punk_Head", "adventurer.glb": "Adventurer_Head"}

if __name__ == "__main__":
    src_dir = sys.argv[1]
    out_dir = os.path.join(HERE, "base", "quaternius")
    os.makedirs(out_dir, exist_ok=True)
    for f, head in HEADS.items():
        d = Doc(os.path.join(src_dir, f))
        for i, n in enumerate(d.j["nodes"]):
            if "mesh" in n and d.j["meshes"][n["mesh"]]["name"] != head:
                d.drop_node(i)
        d.j["animations"] = []
        d.save(os.path.join(out_dir, f))
        print("wrote", f, os.path.getsize(os.path.join(out_dir, f)))
