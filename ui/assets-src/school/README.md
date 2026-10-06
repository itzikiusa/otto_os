# Otto School — 3D asset sources

Sources for `ui/public/school/` (see `CONTRACT.md`). Rebuild + verify:

```bash
B=/Applications/Blender.app/Contents/MacOS/Blender      # Blender 5.2
$B -b -P ui/assets-src/school/blender/build_school.py            # all → ui/public/school/ + manifest.json
$B -b -P ui/assets-src/school/blender/build_school.py -- --verify # exit ≠ 0 on any contract/budget miss
$B -b -P ui/assets-src/school/blender/render_previews.py          # previews/{classroom,corridor,lineup}.jpg
```

Partial builds: `-- --only kit|kids|headmaster` and `-- --kids claude,codex`.
The build is byte-reproducible (it re-execs Blender with `PYTHONHASHSEED=0`
and canonicalises index buffers).

| path | what |
|---|---|
| `blender/build_school.py` | entry point, manifest writer, `--verify` |
| `blender/lib/kit.py` | every `school-kit.glb` node, modelled in bpy |
| `blender/lib/kids.py` | Quaternius kid: head swap, kid proportions, hoodie recolour, chest decal, the 11 clips (IK-authored) |
| `blender/lib/posekit.py` | glTF-space pose authoring: FK, aim/IK edits, CPU skinning, clip writer |
| `blender/lib/headmaster.py` | RobotExpressive → Headmaster Otto, edited at glTF level (`lib/gltfpatch.py`) |
| `blender/lib/dims.py` | shared seat/desk dimensions (seated geometry holds by construction) |
| `blender/render_previews.py` | preview scenes, rendered from the built .glb files |
| `prep_textures.py` | raw generated images + decal SVGs → `textures/` |
| `prep_base.py` | extracts the borrowed kid heads from the Quaternius downloads → `base/quaternius/` |
| `env/` | the HDRI copied to `ui/public/school/env.hdr` |
| `base/` | CC0 inputs (see `LICENSES.md`) |
| `DEVIATIONS.md` | where the build interprets / departs from the contract |

The characters and their clips are edited at the glTF level, not round-tripped
through Blender. The source clips key only the joints they move, and a Blender
import → export re-bases the other joints on Blender's rest pose. The previews
pose the kids with the same math three.js uses (`lib/posekit.py`).
