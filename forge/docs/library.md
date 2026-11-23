# Forge Library

Search Quaternius assets installed under `models/library`. Twelve Standard editions supply 903 model and collection entries. The catalog exposes another 172 selectable animation takes. An entry groups the publisher’s alternate file formats, while hair with a different origin and root-motion editions keep separate IDs.

```sh
bin/forge-library find 'treasure chest' --kind prop
bin/forge-library find 'tree' --kind scenery
bin/forge-library find 'walk' --kind animation
bin/forge-library get fantasy-props-megakit/chest-wood --output chest.glb
bin/forge-library get universal-base-characters/superhero-female-fullbody --output woman.glb
bin/forge-library assemble universal-base-characters/superhero-female-fullbody \
    modular-character-outfits-fantasy/female-ranger-arms --output ranger.glb
bin/forge-animlib apply --mesh woman.glb --clips ual1-walk-loop --output walk.glb
```

`find` prints JSON. Each result contains the catalog ID, name, kind, pack, licence and a thumbnail path relative to `models/library`. Search uses complete words and common synonyms. Choosing an output path that already exists returns a JSON error. `get` and `assemble` emit progress on stderr and write `<output>.manifest.json` alongside the GLB.

## Models and licences

Eleven pack notices say “CC0 1.0 Universal (CC0 1.0)”. The supplied Pirate Kit archive contains no licence file. Its catalog entries carry the status **Unverified in archive** and are excluded from searches and exports. See [the per-pack inventory](../models/library/INVENTORY.md) for licence paths, triangle ranges, texture families, rigs and action names.

The Fantasy Props Standard chest has a skinned lid and four clips. Its pack includes a bronze sword, bronze axe and wooden shield. Bow, arrow, quiver and staff have no matching prop in this edition. The catalog stores visual attachment points in the original GLB scene’s Y-up coordinates in metres. Entries with a failed Fit intake retain that result. A grip point alone does not establish a good hand pose.

`get` embeds the publisher’s glTF buffers and material images in a self-contained GLB. For a clip ID it retains just the selected animation. FBX, OBJ or Blender-only entries use a cached Blender export. Meshes retain their original triangle budgets. The measured `size_m` values give X, Y and Z extents in the default glTF scene. Thumbnail paths identify 192 × 192 Blender Workbench studio previews. The shaded preview is useful for shape and framing. Texture tint, transparency and PBR lighting can look different in the game renderer.

## Bases, hands and outfits

The female and male full-body bases have 65 joints, including 15 articulating finger joints per hand and five terminal joints. The base export maps 52 anatomical joint names to Forge names, preserves original names in the node extras and adds six attachment sockets. The publisher includes eight hair and eyebrow models in two origin variants. The four complete fantasy outfits and twenty individual parts are catalogued separately.

For an outfit, choose the matching male or female base. The publisher advises using just the character head beneath a complete outfit because the clothes include skin. The female parts share the base’s inverse bind matrices. All 12 male files differ on 58 of 65 matrices, so the assembly must account for the difference. See the character verification files under `models/library/verification/characters/` for the source comparisons and pose tests.

## Animation

Each Universal Animation Library Standard pack has 43 named actions in both the ordinary and RM edition. This gives 172 IDs, including 113 distinct decoded motion signatures across the four files. Only 27 of the 86 RM takes move the root more than 1 mm or rotate it more than 0.1°. The catalog records measured root motion and uses the publisher’s `_Loop` suffix for loopability. The take’s metadata includes duration in seconds and category. The existing animlib CLI also lists these local packs.

## Rebuild the local data

The original ZIPs stay in Downloads. Forge copies each ZIP into its own directory and extracts it beneath `files/`. To regenerate the library with an existing Forge install, from the Forge directory run:

```sh
tools/heavy.sh python3 -m forge.library.import_packs ~/Downloads/cc0
python3 -m forge.library.prepare
tools/heavy.sh engines/assetfurnace/.venv/bin/python -m forge.library.inventory
tools/heavy.sh /Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup --disable-autoexec --python-exit-code 1 -P forge/library/anchors.py
tools/heavy.sh engines/assetfurnace/.venv/bin/python -m forge.library.inventory
engines/assetfurnace/.venv/bin/python -m forge.library.thumbnails
```

The conversion and thumbnail commands acquire the shared heavy lock once per batch. Keep the archives, extracted source files, conversions, evidence and thumbnails in the ignored model cache. `forge/library/catalog.json` is the searchable metadata shipped with the stage. The 55 GiB disk floor applies to local preparation and export.

## Fit results on the downloaded characters

Use `models/library/derived/characters/{female,male}/base.glb` for the socketed character and `left.glb` or `right.glb` alongside it for a wrist-cut donor. On the real bases and on the four extracted hands, all 40 isolated digit pulls moved the intended digit. Every finger joint owns vertices. Rebinding the cut hands reproduced joint centres within 0.00055 mm.

**All four donors fail Fit’s 1.8× web-stretch gate for maximum independent curls.** Both female hands reach 1.89× on ring and 2.48× on little. Both male hands reach 2.26× on index, 1.88× on middle, 1.86× on ring and 2.21× on little. Keep Fit’s passing fallback for this use case. The library catalog records the per-digit measurements with the donor paths.

A six-part Ranger outfit on the female and a four-part Peasant outfit on the male both assembled. The largest correction on the male garment was 34.09 mm. Bare base heights measured 1.775 m and 1.820 m respectively.

All 172 UAL variants played on each of the two bases. Over 3,329,820 joint-channel values, the largest bind-relative errors were 0.00000540° in rotation and 0.000000060 m in translation. Absolute walk landmarks differed from the mannequin’s by as much as 7.60 cm. Use `forge-animlib apply --no-grounding` for authored, bind-relative motion. The default applies grounding.

## Prop intake

`fantasy-props-megakit/sword-bronze` passed the Fit geometry check at publisher scale using the female base’s right-hand width. `get` includes its grip and tip landmarks in the manifest. `fantasy-props-megakit/shield-wooden` contains a rear strap. Its `strap_line` records the centres of the ends of that separate strip of faces. Pirate Kit’s Cutlass, Dagger and two Sword meshes failed the same Fit intake. Their entries carry the failed result and the archive’s unverified licence status. There are no library assets on which to run bow, arrow or quiver intake.

[Six representative Blender previews](../models/library/verification/fantasy-props.jpg)

Run the real-file integration suite with the installed packs and generated thumbnails:

```sh
tools/heavy.sh env FORGE_HEAVY_HELD=1 engines/assetfurnace/.venv/bin/python -m unittest discover -v -s tests/library
```

The environment variable tells the Blender subprocesses that this test runner already holds a heavy slot.

## Verification on this install

The library suite passed all 19 tests in 27.940 seconds, including the Smart-mode searches, licence filter and all four assembled character combinations. It checks the CLI, the two bind routes, template extraction, relative output paths, outfit assembly, retained geometry and texture bytes, and all 1,075 rendered thumbnails. The existing animation suite passed all seven tests in 31.693 seconds. [Library test output](../models/library/verification/smart/suites/library.log) · [Existing animation tests](../models/library/verification/animation/legacy-regression.log)

All 172 motion variants exported on both the generated knight and the corrected warrior with grounding. The four 43-clip batches consumed 54.77 seconds of Blender time on the knight and 76.42 seconds on the warrior. An independent GLB reader checked 33,640 poses per character at 120 Hz. Between bake frames the worst sole penetration was 0.406 mm on the knight and 0.199 mm on the warrior. The largest body-aligned limb-direction error over the eight measured limb segments was 0.00024°. [Playback measurements](../models/library/verification/animation/generated-results-120hz.json)

The assembled Ranger and Peasant both have a continuous visible collar in the frontal rest renders. The male shoulder tabs stand away from the sleeves. These outfit checks cover the rest pose. [Assembly renders](../models/library/verification/outfit-comparison.jpg) · [Sample clip previews](../models/library/verification/clip-previews.jpg)

## Smart mode

`bin/forge library --prompt "Treasure chest. A chest full of gold coins" --kind prop --output chest.glb` exports one game asset. Supply the item name first, optionally followed by a period and its description. The stage accepts Otto’s uniform argv, including empty or unresolved placeholder values. `--kind` accepts `any` (default), `prop`, `scenery` or `character`. `--min-score` accepts 0 through 1 and defaults to 0.65. The score is a word-match heuristic.

Read the output manifest for `source` (library), `id`, `pack`, `licence`, `plain_name`, `score`, `clips`, and any `anchors`. `related_pieces` lists separately obtainable objects mentioned in the description, such as the coin pile inside a chest. A nonzero exit whose final stderr object has `error="no library match"` is the signal to generate. Its `best` is an eligible candidate summary, or an empty object when there is none, and `score` is in [0,1]. Other errors identify invalid input, unavailable files or export failures.

`bin/forge library --list-kinds` prints a compact planning summary on stdout. The Pirate Kit remains in the inventory for licence review and is excluded from both searches and exports.

For a dressed character, use `bin/forge library --kind character --prompt "female ranger" --output ranger.glb`. Smart mode offers the Ranger and Peasant outfits on female and male bases. These are four combinations of two clothing designs. A generic character defaults to the female Ranger. The manifest lists the base, garment pieces and animation licences. Clothing, hands and sockets follow the assembled skeleton.

Default animation names are `idle`, `walk` and `run`, using UAL1 Idle Loop, Walk Loop and Jog Forward Loop respectively. Pass `--clips ual2-consume,walk,ual2-laytoidle` to select tracks and control their order. Unique publisher action names also work. The stage binds all joint channels natively and records the underlying UAL ID for each output track.

The export manifest also includes `size_m`, the visible default-pose dimensions [width, height, depth] in glTF metres. For an assembled character this includes the outfit.
