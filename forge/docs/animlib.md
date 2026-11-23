# Animation library

The installed catalog contains 85 KayKit game actions and 810 trimmed 100STYLE locomotion and idle takes. Of the 895 entries, 24 are marked loopable. List the catalog and bake selected clips into a skinned GLB. Run these commands from the Forge checkout. The apply command takes a shared heavy slot automatically.

```sh
bin/forge-animlib list --category locomotion | jq '.[] | {id, duration, root_motion, loopable}'
bin/forge-animlib apply --mesh character.glb --clips walk,run,idle --output character-animated.glb
```

`list` writes a JSON array to stdout. Categories include locomotion, combat, idle, interaction, emote and death. IDs are case-sensitive. Durations are seconds. The short IDs `walk`, `run` and `idle` select the KayKit actions `Walking_A`, `Running_A` and `Idle`. Open the resulting GLB in a glTF animation viewer and select a clip by its ID.

## Rig and motion

Supply a GLB with one humanoid armature and at least one mesh skinned to it. The head, hips and bilateral upper arm, forearm, hand, thigh, shin and foot must be identifiable. Unity Humanoid and Mixamo names are supported, including namespace prefixes. Optional shoulders, spine links, neck and toes are animated when both skeletons provide them. For an AssetFurnace auto-rig, use its Unity-named export if the raw rig still has numbered joints such as `bone_0`. An unsupported joint map prints the missing joints and input bone names.

The retargeter aligns each target bind segment to the corresponding source joint-to-joint direction, preserving world vertical on upright characters. It keys the bind transforms of optional target joints when the source omits them. Switching between captures and KayKit actions therefore resets the clavicles and neck. Authored joint translations and uniform scale changes transfer, and hip travel scales with leg length.

Grounding is on by default. It samples the deformed shoes against the target mesh’s lowest bind-pose vertex, raises the pelvis for floor clearance and smooths horizontal corrections across stance changes. For in-place locomotion it estimates a steady gait speed so a game controller can provide forward movement. Use `--no-grounding` for direct FK, authored foot slides or a runtime IK system. The manifest records the setting. The catalog’s `root_motion` field describes source travel, which is baked at the hips. Choose a fresh output filename. Contact is evaluated on a flat plane; terrain, facial animation and finger articulation remain runtime or rig responsibilities.

## Sources and shipping credits

The catalog is `forge/animlib/catalog.json`. Source files and their licence notices live under `models/animlib/<pack>`. Static reference poses and repeated KayKit action names are excluded. KayKit loop flags require a cycle-type action with matching end poses. The 100STYLE recordings are longer performances. The catalog marks them as non-looping and records the motion sample range, so you can select a take by style and movement direction.

| Source | Licence | Game distribution |
| --- | --- | --- |
| KayKit Adventurers and Skeletons | CC0 1.0 | The bundled publisher licences explicitly permit commercial games. Kay Lousberg welcomes optional credit. [Adventurers](https://github.com/KayKit-Game-Assets/KayKit-Character-Pack-Adventures-1.0) · [Skeletons](https://github.com/KayKit-Game-Assets/KayKit-Character-Pack-Skeletons-1.0) |
| 100STYLE | CC BY 4.0 | Credit “The 100STYLE Dataset - Ian Mason”, link the dataset and licence, and identify your changes, such as trimming and retargeting. The publisher notice and full licence are kept beside the archive. [Author and credit instructions](https://www.ianxmason.com/100style/) · [Dataset](https://zenodo.org/records/8127870) · [Licence](https://creativecommons.org/licenses/by/4.0/) |

Quaternius Universal Animation Libraries 1 and 2 list CC0, but the Standard downloads require the publisher's interactive itch.io flow. Kenney Mini Characters and KayKit’s standalone Character Animations also require a download dialog. The standalone KayKit actions overlap the two installed packs. Obtain those packs manually if wanted. The local catalog only includes installed sources. [Quaternius 1](https://quaternius.com/packs/universalanimationlibrary.html) · [Quaternius 2](https://quaternius.com/packs/universalanimationlibrary2.html) · [Kenney](https://kenney.nl/assets/mini-characters) · [KayKit Character Animations](https://kaylousberg.com/game-assets/character-animations)

## Stage contract and tests

The stage writes `<output>.manifest.json` with the exported clip durations, licences, publisher links and credits. The timings distinguish Blender processing from total elapsed time, which includes waiting for the shared lock. Progress is JSON Lines on stderr. Failures return a nonzero status with a final `{"error":"..."}` line. SIGTERM or SIGINT cancels the Blender process group. The stage checks for 55 GiB of free space before and during processing.

```sh
python3 tests/animlib/test_animlib.py
```

The tests generate a skinned Mixamo T-pose source and Unity A-pose target, decode the exported glTF animation channels, and compare both hands, both feet and the head in world space. They also exercise actual publisher clips, render frames from the output, and cancel a child process through the CLI.

## Rebuild the library on another checkout

```sh
tools/heavy.sh python3 forge/animlib/catalog_tools/fetch_kaykit.py
python3 forge/animlib/catalog_tools/fetch_100style.py --all
tools/heavy.sh python3 forge/animlib/catalog_tools/build.py
```

The 100STYLE downloader takes the shared lock per batch. It resumes a partial transfer and verifies the publisher's checksum. Keep the 1.47 GB archive compressed. Apply extracts only the selected BVHs.

A 100STYLE ID has the form `100style-<style>-<movement>`. The final codes are `fw`/`fr` for forward walking/running, `bw`/`br` for backward walking/running, `sw`/`sr` for sidestepping, `id` for idling and `tr1`–`tr3` for transitions. The stage treats the publisher's frame-cut indices as zero-based and inclusive. Catalog root motion includes horizontal excursion or a change in heading. KayKit thresholds are 1 cm or 20° on its root. The 100STYLE thresholds are 20 cm or 45° on its hips. An in-place cycle can still move a character through the game using the game controller.
