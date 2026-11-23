# forge gameready

Turns a raw AI-generated high-poly textured mesh (100k to 1M triangles, often open or non-manifold) into a
game-ready asset: one low-poly LOD0 at a triangle budget, baked PBR maps, LODs, convex collision hulls,
all in one GLB. It runs headless on this Mac in Blender 5.2.2 LTS. It covers Meshy's remesh, smart
topology (decimation class, see Limits), UV unwrap and convert, and adds LODs and collision, which Meshy
does not produce.

## Usage

```
bin/forge-gameready INPUT.glb --output OUT.glb [--tris 15000] [--quads] [--texture 2048] [--lods 3]
                    [--collision] [--pack DIR] [--detail head,hands] [--detail-strength 6]
```

| Option | Default | Meaning |
|---|---|---|
| `INPUT` | | GLB, glTF or OBJ. Every mesh in it is joined. A skinned (rigged) input is refused. |
| `--output` | required | The single output GLB (contract v1.1). |
| `--tris` | 15000 | LOD0 triangle budget. |
| `--quads` | off | Quad-dominant LOD0 (QuadriFlow) at `tris/2` quads. |
| `--texture` | 2048 | Size of every baked map, in pixels. |
| `--lods` | 3 | Extra levels at 50%, 25%, 12.5% ... of LOD0. |
| `--collision` | off | CoACD convex hulls, at most 8, at most 64 vertices each. |
| `--pack DIR` | off | Also writes per-level GLB and FBX files and the PNG maps (the engine export step). |
| `--detail` | off | Upright humanoids: `head`, `hands` or `head,hands` get more triangles and texels. Off for props. |
| `--detail-strength` | 6 | Detail regions get this many times the triangle and texel density of the rest (1 = off). |

Outputs:

- `OUT.glb`: node `<name>` (LOD0), nodes `<name>_LOD1` ... `<name>_LODn`, and with `--collision`
  nodes `UCX_<name>_00` ... (Unreal's convention). `<name>` is the output file's stem. All LODs share
  one material and one set of embedded textures. Godot reads collision from `-convcolonly` suffixes, not
  `UCX_`, so a Godot import shows the hulls as meshes until they are renamed or hidden.
- `OUT.glb.manifest.json`: `tools`, `licences`, `timings` (per step and per bake pass),
  `triangles` (per level), `lods` (node, triangles, faces, non-manifold and boundary edges),
  `texture_maps` (name and size), `collision` (hull nodes and vertex counts), `clean` (non-manifold
  and boundary edges before and after, merged vertices, removed loose parts, filled holes), `repair`
  (with `local_patch_rounds`), `uv`, `bake` (cage distance, roughness/metallic baked or constant,
  `missed_texels`), `detail` (regions found, strength, and per region and for the whole LOD0: triangles,
  area, triangles and texels per cm2), `bounding_box_m`.
- `--pack DIR`: `<name>_LOD0.glb`/`.fbx` ... `<name>_LODn.glb`/`.fbx` (LOD0's files carry the
  `UCX_` hulls; FBX keeps the quads) and `textures/<name>_basecolor.png`, `_normal.png`, `_orm.png`.
- stderr: one JSON object per line, `{"stage":"gameready","pct":45,"message":"Baking normal map"}`,
  ending at `pct` 100. On failure the last line is `{"error":"..."}` and the exit code is non-zero.
  Blender's own log goes to stdout.
- SIGTERM or SIGINT kills the Blender child at once (`{"error":"cancelled"}`, exit 130). Blender is the
  only child: CoACD and xatlas run inside it.

The first run installs `coacd==1.0.14` and `xatlas==0.0.11` (both MIT, about 4 MB) with `uv` into
`forge/gameready/.venv` (gitignored), a `--target` directory that Blender's bundled Python 3.13 reads.

## What each step does

1. **Import and join.** Imports GLB/glTF/OBJ into an empty scene, refuses armatures/skins, clears
   parents keeping transforms, applies transforms and joins every mesh into one high-poly.
2. **Clean** (on the high-poly, which stays the bake source). Merges vertices closer than 1e-5 of the
   bounding-box diagonal (GLB files split vertices along UV and normal seams), deletes wire edges and loose
   vertices, deletes disconnected parts smaller than 1% of the diagonal, fills boundary loops of at most
   32 edges, drops imported custom normals and shades smooth with no auto-sharp edges. The input's face
   winding is kept: bmesh's recalc-normals turned 1,611 of the text knight's Finish faces inside out
   (folds and patches cut off by non-manifold edges take their "outside" from a coin toss), and those faces
   rendered as black marks on the face and boots after the bake. Sharp edges at 30 degrees put hard creases
   across a 14k face and on rounded chest bands; without them the baked LOD0 shades 2 to 8 times closer to
   its source on matched close-ups. Non-manifold and boundary edge counts are reported before and after. Interior fins and
   large openings are reported, not fixed.
3. **Watertight repair** (only when cleaning leaves open or non-manifold edges). A closed shell with a small cluster of over-connected edges first gets a local repair on a copy. The stage excises the affected faces and caps a single simple boundary loop. It accepts the copy only if vertex and edge manifold checks pass. On the original 4,980-triangle Finish chest, two edges each joined four faces. Replacing 26 faces with a 12-triangle cap produced closed LODs of 4,946, 2,472, 1,236 and 670 triangles. The original textured mesh remains the bake source. For broader defects, a copy of the high-poly is
   voxel-remeshed (Blender's OpenVDB remesher) at 1/1000 of the bounding-box diagonal, edges shorter than
   0.1 mm are dissolved (QuadriFlow refuses them), the few non-manifold spots marching cubes leaves where two
   surfaces touch inside one voxel are cut out and capped (up to three rounds; 21 edges of 5.4M triangles on
   the text knight's raw mesh, where the whole repair used to be discarded and decimation stalled at 215k
   triangles before xatlas hung) and normals recalculated (reliable on the closed shell). The voxel repair
   is skipped when the cleaned mesh is already within the budget (a Finish mesh is then LOD0 as it is; a
   voxel copy would only re-approximate it and, on the text knight's Finish, brought dark marks back), and a
   shell with more area than its input (`area_ratio` 1.2 or more: 1.33 on the open test tube, a sheet turned
   into a slab whose LODs plateau) is discarded; a double-walled generator mesh shrinks (0.79 on the text
   knight's raw mesh). The copy is closed and
   manifold, and it is used only as the geometry source for the low-poly and `--quads`: every map is
   still baked from the original textured mesh, so no texture detail is lost. On the real knight it also
   removed an inner wall: Pixal3D's mesh is double-walled (half of its surface lies about 2 mm inside the
   outer one), and before the repair that inner wall took half the triangle budget, produced 2,199 UV
   charts and stalled the LODs.
4. **Low-poly.** Default: Blender's collapse decimation to `--tris`, re-aimed up to three times, so it
   lands within 2% (the tests got exactly 15000 and 8000). `--quads`: decimate to 4x the budget,
   Laplacian-smooth a copy (QuadriFlow collapses regions of a bumpy surface: asked for 4000 quads on the
   raw decimated sculpt it made 1300, with folds), QuadriFlow at `tris/2` faces aimed by secant steps
   (its count is non-linear in the target), then shrink-wrap the result onto the high-poly.
   **Detail regions** (`--detail`): found without a rig on an upright humanoid (Z up): the head is
   everything above the neck, the narrowest horizontal slice between 70% and 92% of the height; each hand is
   within 10% of the height of the outermost vertex on its side between 20% and 75% of the height. They are
   applied on the first reduction (feed the raw generator mesh, not an already reduced one). The geometry is
   decimated uniformly to 40x the budget, then twice with Blender's vertex-group weighting used as a freeze
   (its cost term dwarfs the quadric error at any factor): first with the regions frozen until the rest
   reaches its share, then with the rest frozen until the regions reach theirs, which is
   `tris x s x a / (s x a + 1 - a)` for strength `s` and region area share `a`.
5. **UV.** xatlas charts and packing at the texture resolution with 2 to 8 px padding. Blender's Smart UV
   Project was tried first and rejected: on a 15k decimated sculpt it made 2,319 islands covering 16% of
   the atlas, against xatlas's 648 charts and 37%. For quads, xatlas only sees triangles, so its chart
   borders become seams (a cut along a quad's diagonal is moved onto the quad's edges) and Blender
   unwraps with Minimum Stretch (SLIM) and packs. With `--detail` (triangle path) the charts that lie mostly
   in a detail region grow by sqrt(strength) and Blender's Pack Islands packs all charts again (xatlas's own
   UV-mesh repack lost the corner order and overlapped charts).
6. **Bake** with Cycles on the CPU, selected-to-active from the high-poly. Cage: extrusion is 2x the 99th
   percentile of the low-to-high surface gap (at least 0.2% of the diagonal), ray distance 2x that.
   Tangent-space normal (MikkTSpace); base colour, roughness and metallic are read exactly by routing each
   source material's input into an emission shader for an EMIT bake. Roughness and metallic are baked only
   when a source material drives them with a texture or the parts disagree; otherwise the constant goes
   into the material. They are packed as ORM (R = 1, no AO bake; G roughness; B metallic), which glTF and
   Unreal both read. 4 samples, margin `size/128` px. The CPU is used on purpose: on Metal each map took
   about 1 s instead of 0.5 s, the first run spent 190 s compiling kernels, and one run crashed inside
   Apple's shader compiler (heap corruption in `AGXMetalG17X`, SIGTRAP). `missed_texels` counts texels
   inside LOD0's islands whose cage ray found no source surface (two extra EMIT bakes at margin 0, about
   1 s at 2K); Blender's EXTEND margin fills them from neighbours, so they smear rather than go black.
   Text knight: 2 from its Finish mesh, about 18,600 (0.6% of the islands) from its raw mesh.
7. **LODs.** Each level is collapse-decimated from the previous one to half of LOD0's count per step, so
   it keeps LOD0's UVs and material.
8. **Collision.** CoACD on LOD0, threshold 0.08, at most 8 hulls, at most 64 vertices each (PhysX caps a
   hull at 255), seed 1.
9. **Export.** One GLB (Y-up, embedded PNG maps), the manifest, and with `--pack` the per-level GLB/FBX
   files (FBX: face smoothing, tangent space, relative texture paths).

## Measured (M5 Pro, 48 GB, 2026-11-19, full test runs; per-step times from the manifests)

The repair time is listed in the Output column; the low-poly column excludes it.

| Input | Output | import | clean | low-poly | UV | bake (all maps) | LODs | collision | total |
|---|---|---|---|---|---|---|---|---|---|
| sculpt, 510,720 tris, 1K colour + MR textures | 15,000 / 7,500 / 3,750 / 1,874 tris, 3 x 2048 maps, 8 hulls | 0.38 s | 1.11 s | 2.34 s | 0.75 s | 2.47 s | 0.05 s | 4.98 s | 12.6 s |
| sculpt, `--quads --tris 8000` | 4,075 quads (8,150 tris) / 4,074 tris, 3 x 1024 maps | 0.36 s | 1.14 s | 8.75 s | 4.25 s | 1.59 s | 0.03 s | | 16.3 s |
| hollow tube, 204,812 tris | repair tried and discarded (3.6 s), 4,999 / 2,499 / 1,249 tris, 2 x 1024 maps, 8 hulls | 0.24 s | 0.69 s | 0.78 s | 0.08 s | 0.35 s | 0.01 s | 18.5 s | 24.2 s |
| real Pixal3D knight, 935,156 tris (1,060 non-manifold edges after cleaning) | repaired (1,109,282-tri voxel copy, 1.4 s), 14,998 / 7,498 / 3,750 / 1,874 tris, closed manifold, 3 x 2048 maps, 7 hulls, 6.4 MB | 0.83 s | 2.58 s | 5.37 s | 0.49 s | 3.34 s | 0.05 s | 4.29 s | 18.4 s |
| knight, `--quads --tris 15000 --lods 1` | 7,546 quads (15,092 tris) / 7,546 tris, closed, 3 x 1024 maps | 0.82 s | 2.57 s | 10.27 s | 4.58 s | 2.02 s | 0.03 s | | 21.8 s |
| multipart, 6 parts, 39,360 tris | 8,000 / 4,000 / 2,000 tris, 3 x 1024 maps, 8 hulls | 0.10 s | 0.12 s | 0.11 s | 0.12 s | 0.65 s | 0.03 s | 17.1 s | 18.3 s |

Add about 1 s of Blender start-up per run. CoACD dominates; without `--collision` every case finishes in
under 17 s. The quad count varies a little between runs (7,826 to 8,150 triangles across the runs made on 2026-11-19) because
QuadriFlow is not deterministic.

## Tests

`tools/heavy.sh tests/gameready/run.sh` (15 tests, about 4 to 5 minutes with the real meshes present). It builds the inputs procedurally in Blender
(`tests/gameready/make_inputs.py`: a 510k-triangle displaced sculpt with a colour texture, a metal/rough
texture and wrinkles for the normal bake; an open tube with flipped patches, duplicate seam vertices,
small holes, a 3-faces-per-edge fin and a loose fragment; a 6-part parented car with three materials; a
skinned cube), runs the stage as Otto would, re-imports the real files and checks: the single GLB holds
exactly `<name>`, `<name>_LODn` and `UCX_<name>_NN`; LOD0 within 5% of the budget; LODs strictly
descending at about half each; one shared material; UVs inside 0-1 and no overlaps (rasterised at 2048);
maps at the requested size; the normal map is a real tangent-space map with detail (more than 25% of
texels deviate over 5 degrees); baked base colour, roughness and metallic match the source textures at
3,000 random surface points; hulls are convex and enclose 99% of LOD0's vertices; hollow input is
repaired as reported; multipart colours and per-part metallic survive the bake; `--quads` FBX is more than
95% quads; the progress lines are JSON and reach 100; a rigged input fails with `{"error"}`; SIGTERM
leaves no Blender process. When `out/p0/character/raw.glb` (the real Pixal3D knight, 33 MB, referenced in
place, not copied) exists, two more tests run on it and otherwise skip with the path in the message: the
repair is used and closed, LOD0 and every LOD re-import as closed manifold (after welding UV-seam
splits), LODs within 10% of half the previous level, baked colour matches the source at 3,000 points
(median error 0, 95th percentile 4/255 on its flat-coloured texture), and `--quads` succeeds with
more than 95% quads. When the text knight's Finish mesh
(`out/text_to_asset/character/finished/finished.glb`) exists, one test checks that LOD0's head and feet faces
face the same way as the source surface under them (under 0.5% inside out; the old clean() left 296 of 2,140)
and that no baked texel there is near-black where the source is not. When its raw mesh
(`out/closeup-gen/baseline/raw.glb`) exists, one test runs `--detail head,hands`: the repair is used, LOD0
is within 5% of the budget, the regions get their triangle share within 10%, each region has more than 3x
the asset's triangle and texel density, and the UVs do not overlap. Mutation checks were run: disabling the cage (bake misses), baking the wrong
channel into base colour, building LODs at a third instead of a half each, and switching the repair off
(the knight's `--quads` then fails and its repair assertions fail) each made the suite fail.

## Close-ups (text knight, 2026-11-20)

Matched cameras from `tools/closeup_views.py` (one views file per asset, the same for every variant; modes
lit, unlit base colour, grazing-light clay, normal-map shading and face normals with back faces red, plus a
near-black/back-facing pixel count per crop). Evidence sheets are in `out/closeup/`; the measured report is
`closeup-gameready-report.md` in the design research folder.

- **Black marks** (face, under one eye, boot toes, greaves): inside-out faces from cleaning, not the bake.
  Base colour was not darker (face 1,593 near-black px in Finish, 1,431 in game-ready); back-facing pixels
  went from 27 to 747 (face) and 8 to 1,568 (left boot); with the input winding kept they are back to the
  source's 27 and 8 and the marks are gone.
- **Raw vs Finish as the geometry source** (p0 knight, which has both): silhouette IoU against the raw mesh
  is 0.997 to 0.999 on all seven views for 15k from raw and 0.981 to 0.992 for 15k from Finish. Feed raw.
- **Detail regions** (text knight raw, 15k): head 323 triangles / 0.93 per cm2 / 79 texels per cm2
  without; 637 / 1.86 / 144 at strength 3; 1,065 / 3.15 / 249 at strength 6. The ear, brow ridge, eyelids
  and fingertips come back progressively; the rest of the body drops from 80 to 58 texels per cm2 (Blender's
  packer fills less of the atlas than xatlas). About 5 to 10 s more decimation.
- **Texture size**: raw's 4K atlas has about 307 texels per cm2 on the head and 317 overall; Finish's 2K
  texture 107. 2K with `--detail` at 6 already gives the head 249; 4K gives 997 on the head and 232 overall,
  a 31 MB GLB instead of 12 MB and 28 s of bake instead of 9 s, and the face and hand close-ups look the same
  as at 2K. 4K stays an option.
- **Projection of the source picture**: run AssetFurnace's own Pixel Match
  (`engines/assetfurnace/scripts/photo_paint.py LOD0.glb raw.svviews OUT.glb`) on the game-ready LOD0 with
  the generator's own `transforms.json` camera; no camera estimation. Its outline alignment went from 0.958
  to 0.973 IoU (scale 1.010, no shift) and it repainted the front-facing 5 to 6% of the atlas in 6 to 11 s.
  With the view picture upscaled 4x by Real-ESRGAN x4plus (18 s; the camera stays valid because the focal
  length comes from the FOV and the image width) eyes, lids, brows and mouth are clearly crisper (68 to 72 s).
  Remaining defect: a lighter band where the projection fades out on the left cheek. Raw without projection
  has clean geometry but the generator's own face paint (tiny pupils, heavy brows), so the projection step is
  needed when Finish is skipped.

## Quality, honestly

- **Real knight (Pixal3D, 935k triangles), before vs after the repair.** Before: LOD0 was open (1,060
  non-manifold edges), `--quads` refused it, the LODs plateaued at 7,496 / 7,131 / 7,105 triangles and
  LOD0 had 2,199 UV charts; the wireframe showed hundreds of long sliver spikes. After: every level is
  closed manifold, the LODs halve (14,998 / 7,498 / 3,750 / 1,874), charts drop to 110, `--quads` gives
  7,546 clean quads, and the run costs 1.4 s more for the repair plus about 1 s more decimation (18.4 s
  total). The repaired surface sits on the original: 99% of it within 0.06 mm, worst 0.45 mm (measured on
  20,000 samples). What it removed is the inner wall: the median original-surface sample lies 1.96 mm from
  the repaired copy at every voxel size tried (2.96, 1.69, 1.18 mm), which only an inner layer explains;
  1% lies 40 to 57 mm away, deep inside the body. In 3x close-ups of the front view (evidence
  `knight_closeups_source_before_after_quads.png`; rows source, before, after, after with `--quads`;
  the full sheets are `knight_tris15k_before_repair_turnaround.png`, `knight_tris15k_repaired_turnaround.png`,
  `knight_quads7.5k_repaired_glb_turnaround.png` and, for real quad wireframes, the FBX sheet
  `knight_quads7.5k_repaired_turnaround.png`, whose silver reads whiter because FBX does not carry the ORM map):
  the fingers, thumb, helmet crest and belt buckle of the repaired triangle LOD0 look the same as before
  the repair and as the source. The `--quads` LOD0 is softer: the fingers merge into a mitt with a thinner
  thumb, the crest is slightly thinner and the belt's lower corners round off, all from the smoothing that
  QuadriFlow needs. The knight's texture is flat-coloured, so these colour numbers flatter the bake.

- **Baked LOD0 vs source.** On the sculpt at 15k triangles (97% fewer) the renders are hard to tell
  apart from the source: the colour texture, wrinkles and grooves come back through the maps. The visible
  difference is the silhouette: wrinkles that stick out of the outline become smooth on LOD0, because no
  normal map changes an outline. Base colour matches the source at 3,000 sampled points with a median
  error under 0.05.
- **Topology (default).** This is decimation topology: irregular triangles, dense where curvature is
  high, long thin triangles on flat or cylindrical areas (very visible on the tube and on the car body).
  It is fine for static props and rendering. It is not artist retopology: no edge loops, no deformation
  flow.
- **Topology (`--quads`).** QuadriFlow gives evenly spaced, field-aligned quads (100% quads in the
  tests), the same class as Meshy's quad remesh. Still no edge loops placed for deformation, and the
  smoothing step softens fine silhouette detail more than the triangle path does.
- **Deforming characters** (rigged, animated) need edge loops around shoulders, elbows, knees, mouth and
  eyes, and enough density where joints bend. Neither decimation nor QuadriFlow provides that. What would
  be needed: manual or template-based retopology (fitting a rigged base mesh to the AI mesh and baking onto
  it), or a learned artist-topology model on a rented GPU (see the research notes); automatic quad
  remeshers are not deformation topology.
- **UVs.** xatlas charts are many (648 on the 15k sculpt) and use about 63% of the atlas with padding.
  Fine for baked textures, poor for hand painting.
- Roughness/metallic: a constant ORM red channel (no AO bake).

## Limits

- Skinned input is refused (v0).
- The input's face winding is kept. Patches that are really flipped in the source (they render inside out
  there too) stay flipped, and xatlas cuts a chart per patch: the open test tube gets 929 charts using 21%
  of the atlas instead of 4 charts using 70%. Generator meshes that go through the voxel repair are not
  affected: the closed shell is oriented reliably.
- `--detail` assumes an upright humanoid (Z up, A- or T-pose) and finds regions from the bounding shape; it
  does not read a skeleton. With `--quads` it only weights the pre-QuadriFlow decimation and does not grow
  the UV charts. Detail charts are packed by Blender, which uses the atlas less well than xatlas.
- The text knight's raw mesh decimates to an LOD0 with about 940 non-manifold edges (thin voxelised parts
  collapsing), and its LOD3 plateaus near 3,200 triangles.
- The repair voxel is 1/1000 of the diagonal (1.2 mm on the 0.9 m knight). Parts thinner than about a
  voxel lose their shape or vanish in the repaired copy (and so in LOD0), though their colour is still
  baked where LOD0 has a surface. An open, zero-thickness sheet (a cape, the hollow test tube) becomes a
  thin closed slab.
- Interior surfaces disappear from LOD0 when the repair runs (wanted for double-walled AI meshes, wrong
  for an asset whose inside is meant to be seen, such as an open box).
- Cleaning does not remove interior faces, fins or self-intersections, and does not close large holes;
  the counts are reported in the manifest. The hollow test mesh keeps 1,068 non-manifold edges (its two
  open ends and the fin).
- `--quads` UVs: about 0.01% of texels fold inside their own island (a few triangle pairs), measured in
  the tests; the triangle path has none.
- Textures are always square at `--texture`; no 4K default, no AO, no emission or alpha bake.
- The UCX hulls are in the single GLB as plain meshes; engines other than Unreal need them renamed or
  hidden.
- The CoACD hulls can be 30 s on a large open mesh; they are optional.
- The repair is discarded when the voxel copy is itself not closed manifold: the zero-thickness test tube
  voxelises (at 3.5 mm) into a 4-million-triangle slab with 61 non-manifold edges, which also stalls
  decimation. Such inputs keep the old behaviour (open LOD0, `--quads` fails) and the manifest says
  `"used": false`.
