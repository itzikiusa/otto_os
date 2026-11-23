# Forge engine: AssetFurnace v0.6.0 on this Mac (Phase 0)

Proved on 2026-11-19 on a MacBook Pro M5 Pro, 48 GB, macOS 27.2, Xcode 27.2, Blender 5.2.2,
while other agents shared the machine (swap ~5.5 of 7 GB used throughout, never critical).
All evidence (logs, GLBs, stats, renders, manifests) is under `out/p0/`.

## What AssetFurnace is, versus what the research said

Repo exists: Bingeljell/image-to-3dlab, tag v0.6.0 = commit 5f9040b (2026-11-18), Apache-2.0 + NOTICE.
The chain is real, but several details differ from the research notes:

| Research said | v0.6.0 actually |
|---|---|
| FLUX.2 klein 4B via **mflux** | via **stable-diffusion.cpp** (`sd-cli`, a prebuilt Metal binary, sha256-pinned), GGUF Q8_0 + Qwen3-4B text encoder. Qwen-Image 2.1 (Qwen Research License) is the alternative |
| BiRefNet | BiRefNet-lite, AssetFurnace's own ONNX re-export (181 MB, MIT) run through rembg; u2net fallback |
| Pixal3D via pixal3d.cpp on Metal | yes: raven38/pixal3d.cpp @ c89126d compiled from source (needs full Xcode, cmake, ninja) + Q8_0 weights raven38/pixal3d-sv-q8_0-v1 (8.4 GB) that **bundle a converted DINOv3 encoder** (no gated download) |
| TRELLIS.2 on Mac | needs the gated `facebook/dinov3-vitl16-pretrain-lvd1689m` + `hf auth login`: **blocked** under our no-login rule; not installed |
| "Finish" (decimate, UV, texture) | `scripts/retopo_repaint.py`: Pixal3D's own clean-up replayed at a face budget (post-replay), Pixel Match (projects the source picture), Blender normal + metal-roughness bake, texture compress. Optional repaint uses Hunyuan 2.1 paint (Tencent licence) and is off by default |
| SkinTokens rig, pure PyTorch, Unity names | yes: SkinTokens @ 273b691 + AssetFurnace's portable patch (SDPA instead of flash-attn, MPS), own Python 3.11 venv with the `bpy` wheel; Unity Humanoid names are a post-step (`name_bones.py`), the studio default is `bone_0..N` |
| 21 preset animations | **30** preset clips, all **pre-generated with NVIDIA Kimodo** (NVIDIA Open Model License), retargeted by `scripts/kimodo_retarget.py`; not CC0 |
| "a CLI and an MCP server" | no single CLI for the chain. `pipeline.py` covers SF3D / TRELLIS.2 / Hunyuan-ComfyUI only; Pixal3D, Finish, rig and animate are separate scripts; text-to-image, rig and animate orchestration lives in the studio web server (`viewer/*_api.py`). The MCP server is a thin client of that server and starts it in the background |
| — | Telemetry: `install.sh` POSTs install events, the studio sends daily anonymous usage counts and fetches a news file. We never ran `install.sh` or the studio; forge sets `I3D_NO_USAGE=1 DO_NOT_TRACK=1 I3D_NO_NOTICES=1 HF_HUB_DISABLE_TELEMETRY=1` anyway |
| — | Models default to `~/.cache/huggingface` and `~/.u2net`; we set `HF_HOME`, `ASSETFURNACE_MODELS`, `U2NET_HOME` under `models/` |

## Install steps that worked

Everything under this folder; `uv` 0.8.17; no sudo; never `curl | bash`.

```sh
export HF_HOME=$PWD/models/hf ASSETFURNACE_MODELS=$PWD/models/af U2NET_HOME=$PWD/models/af/u2net \
       UV_CACHE_DIR=$PWD/models/uv-cache UV_PYTHON_INSTALL_DIR=$PWD/models/uv-python \
       I3D_NO_USAGE=1 DO_NOT_TRACK=1 I3D_NO_NOTICES=1 HF_HUB_DISABLE_TELEMETRY=1
git clone --depth 1 --branch v0.6.0 https://github.com/Bingeljell/image-to-3dlab engines/assetfurnace
cd engines/assetfurnace
uv sync --frozen --no-dev --inexact --python 3.11       # install.sh's own step 4, run by hand
uv pip install --python .venv/bin/python ninja          # pixal3d.cpp's Metal build wants ninja
PATH=$PWD/.venv/bin:$PATH
../../tools/heavy.sh .venv/bin/python scripts/bootstrap_pixal3d.py --build-only --yes    # compile trellis-cli (Metal)
../../tools/heavy.sh .venv/bin/python scripts/bootstrap_pixal3d.py --weights-only --yes  # 8.4 GB + BiRefNet-lite
../../tools/heavy.sh .venv/bin/python scripts/bootstrap_autorig.py --yes                 # SkinTokens venv + 1.6 GB
../../tools/heavy.sh .venv/bin/python scripts/bootstrap_flux_image.py --yes              # sd-cli + 8.9 GB
```

Then `bin/forge` runs on `engines/assetfurnace/.venv` (see `forge/cli.py`).

## Timings and memory (M5 Pro 48 GB)

"real" and "max RSS" are `/usr/bin/time -l` (max RSS of the largest process in the tree). Metal
buffers on unified memory are not all counted in RSS, so "sampled footprint" adds the peak of
`footprint -p` summed over every process of the stage, sampled every 3 s (forge's own manifests
record `ru_maxrss` from `wait4`, the same number `time -l` prints).

| Stage | Exact command (from `engines/assetfurnace` unless noted) | real | max RSS | sampled footprint |
|---|---|---|---|---|
| install: AF venv | `uv sync --frozen --no-dev --inexact --python 3.11` | 15.7 s | not captured | - |
| install: pixal3d.cpp Metal build | `scripts/bootstrap_pixal3d.py --build-only --yes` | 36 s (+294 s waiting for the lock) | 0.3 GB | - |
| install: Pixal3D weights + BiRefNet-lite | `scripts/bootstrap_pixal3d.py --weights-only --yes` | 5067 s (84 min, network) | 0.8 GB | - |
| install: SkinTokens venv + checkpoints | `scripts/bootstrap_autorig.py --yes` | 706 s | 0.8 GB | - |
| install: FLUX.2 klein | `scripts/bootstrap_flux_image.py --yes` | sd-cli installed; weights stalled (see below) | - | - |
| character: stand-in picture | `Blender -b --python tools/make_standin.py -- out/p0/character/standin.png` (forge root) | 5.0 s | 1.9 GB | - |
| character: background removal | `python -c "...cut_out(Image.open(standin.png))..."` (BiRefNet-lite) | 2.9 s | 4.1 GB | - |
| character: image to 3D | `scripts/pixal3d_generate.py matted.png raw.glb --seed 42` (1024, 8 steps) | **154 s** | 3.4 GB | not sampled |
| character: Finish (20k) | `scripts/retopo_repaint.py raw.glb matted.png finished.glb --skip-paint --faces 20000 --views raw.svviews --native-dump raw.post.bin` | 23 s | 2.9 GB | 4.3 GB |
| character: rig | `bin/forge rig --mesh finished.glb ... --output rigged.glb` (SkinTokens 32.7 s, weight clean 3.3 s, Unity names 0.2 s) | 36 s | 2.9 GB | 6.5 GB |
| character: animate idle,walk,run | `bin/forge animate --mesh rigged.glb ... --output animated.glb` (6.8 + 2.2 + 2.2 s) | 11 s | 0.4 GB | 0.2 GB |
| prop: stand-in picture | `Blender -b --python tools/make_standin_chest.py -- out/p0/prop/standin.png` | 5.1 s | 1.9 GB | - |
| prop: forge generate (whole) | `bin/forge generate --mesh "" --image standin.png --prompt "" --output chest.glb --poly-budget 5000 --seed 42` | 1969 s | 10.0 GB | 3.3 GB at the sampled moments |
| ↳ matte | (inside forge) | 4.5 s | 4.1 GB | |
| ↳ Pixal3D | (inside forge) | **1013 s** (GPU shared with other agents' Android emulators / iOS simulators / Playwright) | 9.6 GB | |
| ↳ Finish 5000 | (inside forge) | 50 s | 3.7 GB | |
| ↳ gameready (bin/forge-gameready, other agent) | (inside forge) `forge-gameready finished.glb --output ... --tris 5000 --lods 3 --collision` | 902 s incl. ~8 min waiting for a lock slot | 7.3 GB | |
| prop: Finish with `--upright` | `scripts/retopo_repaint.py ... finished_upright.glb --skip-paint --faces 5000 --upright ...` | 29 s | 3.9 GB | |
| renders | `Blender -b --python tools/render_views.py -- X.glb X_turnaround.png [--clip walk --at 0.5]` | ~2.5 s each | 0.7-1.6 GB | |
| cancellation | SIGTERM during Pixal3D, SIGINT during SkinTokens | children gone within 6 s, exit 143, last line `{"error": "cancelled"}` | | |


Downloads (size on disk): Pixal3D weights 8.4 GB + BiRefNet-lite 0.18 GB (`models/af`, 8.5 GB total);
SkinTokens checkpoints 1.6 GB + Qwen3-0.6B tokenizer (`models/hf`) and its venv 2.0 GB (torch, bpy, open3d, gradio);
AssetFurnace venv ~1.1 GB; FLUX.2 klein Q8_0 + Qwen3-4B + VAE not downloaded (see below). Model weights total ≈10.1 GB (8.5 GB models/af + 1.6 GB SkinTokens in models/hf); plus venvs 1.1 GB + 2.1 GB and builds 0.3 GB (cap 35 GB).
The line was ~1-3 MB/s (VPN); Pixal3D's 8.4 GB took 84 min with two resumed connection drops.

## What failed or needed a fix

- `image_to_3dlab.matte.cut_out` returns `(image, model)`, not an image (my first matte call failed; fixed in forge).
- `clean_skin_weights.py` must run in the SkinTokens venv (needs `bpy`); the AF venv fails with `No module named 'bpy'`.
- On macOS `killpg` returns EPERM when only zombies remain in the group; forge's cancel path treats any OSError as "gone".
- The glTF importer in Blender 5.2 adds an "Icosphere" bone-shape object to rigged files; the render script hides it.
- TRELLIS.2: not attempted (gated DINOv3 needs a Hugging Face login; blocker: `facebook/dinov3-vitl16-pretrain-lvd1689m`).
- Hunyuan3D engines, SF3D and RMBG-2.0: deliberately not installed.
- **During the initial stand-in tests, FLUX.2 klein had only its binary installed**: `sd-cli` (36 MB, sha256-checked) is in
  `vendor/sdcpp/`, but the 8.9 GB of weights never arrived. The first attempt reached 0.39 GB in 15 min, a restart did
  not resume (huggingface_hub began a new partial) and stalled at 10 MB; a probe measured ~2 MB/s (~75 min for 8.9 GB).
  Per the coordinator the character and prop were proved from **stand-in pictures I rendered in Blender from primitives**
  (`tools/make_standin.py`, `tools/make_standin_chest.py`; no third-party image, so no licence question). `forge generate
  --prompt` builds its sd-cli command from AssetFurnace's own FLUX defaults (1024², 4 steps, cfg 1, euler) and subsequently ran on the text-generated knight and chest concepts. See the follow-up below.
- Pixal3D is GPU-bound: the same model took 154 s alone and 1013 s while other agents ran GPU emulators.
- Props come out tilted to the picture's camera; Finish `--upright` fixes it (forge passes it for `--kind prop`).
- Characters come out ~0.9 m tall (the model's unit box); Finish has `--height-m`, forge does not set it yet.
- The game-ready stage (other agent) on the chest: asked 5000 tris, LOD0 has 9530, LOD1-3 9121/9061/9051 (barely reduced),
  1198 non-manifold edges, and its re-bake left torn texture patches on the lid that the Finish output does not have.

## Outputs (out/p0)

- `character/`: `standin.png` → `matted.png` → `raw.glb` (Pixal3D) → `finished.glb` (Finish 20k) → `rigged.glb` → `animated.glb`
  (idle, walk, run); `*.stats.json`, `*.manifest.json`, `*_turnaround.png` (front, left, back, right), `walk_mid_turnaround.png`,
  `run_mid_turnaround.png`, `idle_mid_turnaround.png`.
- `prop/`: `standin.png`, `raw.glb`, `finished.glb` (tilted), `finished_upright.glb`, `chest.glb` (forge generate incl. game-ready
  LODs + 8 UCX hulls) with `chest.glb.manifest.json`. Its `triangles` field was corrected by hand after the run, when
  glbstats was changed to count LOD0 only (it had summed LODs and hulls: 37,745).
- `logs/`: every stage's full output with the `/usr/bin/time -l` block.

## Open licence questions

- **DINOv3**: Pixal3D's Q8_0 bundle ships `dinov3.gguf`, a format conversion of Meta's gated DINOv3 ViT-L/16, under the
  DINOv3 License (copy in `models/af/pixal3d-cpp/pixal3d-sv/DINOV3_LICENSE.md`). It allows commercial use and
  redistribution under the same Agreement with a copy, but bans military/nuclear/espionage/weapons end uses, requires
  Trade-Control compliance and forbids reverse engineering. We downloaded it without ever accepting Meta's gate:
  whether a third-party re-host binds us the same way is a question for counsel. AssetFurnace classifies Pixal3D
  output as "commercial-clear".
- **Pixal3D weights**: raven38's bundle labels the four flows MIT, converted from TencentARC/Pixal3D. The upstream
  issue asking whether the weights are really MIT (a ComfyUI wrapper calls them academic-only/EU-excluded,
  TencentARC/Pixal3D#33) was unanswered in the research notes.
- **SkinTokens**: code and checkpoints MIT; the training data (articulation datasets) is not named with a licence.
- **Animation clips**: AssetFurnace's 30 presets are Kimodo (NVIDIA Open Model License) outputs. NVIDIA claims no
  ownership of outputs, but the design's "CC0 animate stage" is not what this engine ships; Kimodo's own training
  data was not checked. The installed animation library offers 85 CC0 KayKit actions and 810 CC BY 4.0 100STYLE takes.
- **FLUX.2 klein 4B, Qwen3-4B, VAE**: Apache-2.0 (as stated by AssetFurnace's pins; model cards not re-read here).
- Blender/bpy (GPL-3.0) is only run as a tool; outputs are not affected.

## First prompt-driven verification

The pinned FLUX diffusion model, Qwen3-4B encoder and VAE downloaded and passed the engine bootstrap's SHA-256 checks on 20 November. The payload was 8,917,246,524 bytes. The knight's 1024-square concept took 47.4 s on the M5 Pro. Its complete generation took 289 s at a 15,000-triangle budget. SkinTokens fitted 52 joints to the 14,080-triangle display mesh, and both animation routes embedded idle, walk and run. Forge excludes packaged LOD and collision geometry from the input to SkinTokens. The working install keeps the concept, turnarounds, close-ups, clips and receipts under `out/text_to_asset/`. Close camera shots expose faceted fingers and black patches on the cheek and boots. The generation and Finish results above remain the timings for the original stand-ins.

The same seed produced a real 1024-square chest concept in 40.2 s. Its dense Pixal3D pass took 1,434.6 s and the whole CLI took 1,534.7 s. The prop had 27,455 occupied high-resolution shape tokens versus 5,431 for the knight. Set a longer local-stage timeout in Otto for a prop of this density. Both assets used three 2K maps and eight collision hulls. The knight's base mesh still had 827 non-manifold edges. The chest had zero at all four levels (4,988 / 2,494 / 1,246 / 624 triangles). The grain, lock and bands read clearly from the concept-facing side. On the back, the bands terminate part-way across the lid and stray brass-coloured patches interrupt the wood. The lid shares the base mesh, so an opening animation needs a separated lid and pivot. Detailed views, both animations, measurements and logs are in `out/text_to_asset/`.

## Rig and animate any body

`bin/forge rig` keeps the skeleton SkinTokens fits to any mesh. A body never fails the rig stage because of its shape. Rigging fails only for reasons unrelated to body shape. These include a missing model, a cancellation or a broken file. In those cases the stage ends with the usual `{"error": ...}` line and leaves the input mesh untouched.

The first check is the one AssetFurnace's preset moves already used: the humanoid mapper (two legs and a spine under the hips, and a chest that forks into a neck and two arms). Forge adds two conditions. First, the head must stand above the hips, rising at least 1.5 times its horizontal offset from them. Second, at rest each arm must reach sideways for at least a quarter of its length, as in the T or A pose the presets assume. On Forge's generated humans this share is 0.36 to 0.69. A round bird's folded wings measured 0.06 and 0.2. A bird or a dinosaur can fit the mapper with wings or arms, but human moves would stand it up like a person or fling its wings out. A humanoid gets exactly the former behaviour: Unity Humanoid names, the Kimodo preset retarget, and `body_plan: "biped_humanoid"` in the manifest. On the text-generated knight, the old and new code produced byte-identical rigged and animated GLBs from the same seed (see `out/rig-any-evidence/knight/`).

Every other body gets a body plan (`forge/bodyplan.py`), computed from the skeleton and its skinned mesh in the rest pose:

- **Legs.** A leg is a branch whose skin reaches the floor (within 6% of the height). From each foot, the limb climbs toward the body. It stops at a joint that holds another foot, the root, or a large branch that is not this foot, such as a head or a wing. A leg must hang from a joint at least 12% of the height above the floor. It must also drop at least half its horizontal reach. Its lower half must come down onto the foot at a slope of at least 1 in 2. A snake's head and tail lying on the ground do not count, and neither does a tail resting on the floor. Two bones from one joint whose feet stand together, pointing the same way, count as one leg split in two. They move together.
- **Spine, head and tail.** Wings, arms and ears come in mirrored pairs, so the spine is the longest chain of joints on the body's mirror plane. Without a clean mirror plane, for example with a tail curled to one side, it is the longest chain overall. On a long body the head is the end the toes point toward. If the toes give no clear direction, it is the higher end, or the thicker end on a legless body. On an upright body the head is the higher end. The other end is the tail. Forward runs from the body to the head.
- **Other limbs.** A branch of the head or neck is an ear. A side branch of the body that does not reach the floor is an arm (or wing). Anything else is an extra.

The manifest records `body_plan` (`quadruped`, `legs:N` or `legless`), `legs`, and under `body` the forward axis, floor, height and every chain by joint name. Joints get readable names such as `root`, `spine_1`, `neck_1`, `head`, `tail_1`, `leg_front_left_1`…, `leg_hind_right_3`, `leg_left_2_1` (the second leg from the front), `arm_left_1` and `ear_right_1`. Each vertex keeps at most four weights. Detection and naming take under 0.1 s.

`bin/forge animate` sends a humanoid to the presets as before. Every other body gets procedural `idle`, `walk` and `run` from `forge/gait.py`, written straight into the GLB's animation channels without Blender. Clips play in place, like the preset loops. The manifest's `motion.clips.<name>.speed_m_s` is the speed at which a game controller should move the body: the speed the planted feet travel backward.

| Body | idle | walk | run |
|---|---|---|---|
| one leg | breathing, weight shift, look | hop: crouch, push off, flight, land | longer, higher hops |
| two legs (not human) | the same | alternating steps, waddle over the stance foot | alternating, longer strides |
| three legs | the same | one leg after another (tripod roll) | the same, faster |
| four legs | the same, tail and ears sway | lateral-sequence walk (hind left, front left, hind right, front right) | trot (diagonal pairs) |
| six or eight legs | the same | metachronal wave, each side half a cycle apart | alternating tripods or tetrapods |
| legless, long (a lying spine, the body at least 1.5 times as long as it is tall: snake, fish) | slow small wave | travelling wave of bending, growing toward the tail | faster, larger wave |
| legless, upright (slime, a mushroom on its stem) | breathing squash and sway | hops: squash on the floor, stretch in the air, sway | higher, faster hops |
| legless, rigid (two joints or fewer: a block, a robot on wheels) | breathing squash and sway | glide: small bob, roll and yaw | the same, faster |

Each leg has a phase and a duty factor (the share of the cycle its foot is down). A planted foot moves straight back along the floor at the body's speed. A swinging foot lifts on an arc whose velocity matches the stance at lift-off and touch-down. Two-segment analytic IK reaches every foot target, and the knee keeps the side it bends to at rest. The foot keeps its rest orientation, so the sole stays flat. As it lifts, the toes dip by at most half the lift. The body bobs with the stance, pitches and rolls slightly, and the spine and tail swing with a lag. Arms and ears sway, and the head holds its orientation in world space. A leg without a knee (a single bone) can only rock its sole. Its stride is the widest swing that keeps the sole within 4 mm below and 1.5% of the height above the floor. In that case the body stays level. All legs share one stride, and when a stiff leg shortens it, the cadence stays at the legs' pendulum rate and the body slows down. Every clip is periodic, and its last key equals its first.

Each clip is measured by `forge/fit/plan_check.py`. These are forge-fit's foot, ground and loop checks with the same limits, per detected leg, on the deformed skin read back from the written GLB: `foot_penetration_m` 1 cm, `foot_slide_m` 5 cm, `loop_seam_m` 1 cm and `hips_acceleration_ms2` 150. `ground_penetration_m` (1 cm) covers every part of the skin that is not a leg, so a tail, a belly or a leg the skeleton missed cannot sink through the floor. On a legless body it covers the whole skin. A walk or run also needs `leg_swing_m` of at least 2 cm: every foot must travel under the body, because legs that stand still while the body moves are a slide. A failing walk or run is remade at 70%, then at 45% of the motion. If it still fails, it is left out and `motion.note` says which measure failed and which leg held the stride back. Idle always ships with its measurements. The manifest's `motion` stays at most three levels deep, because Otto nests it inside provenance that an installed app caps at eight levels. For each clip it gives pass or fail, the worst `sink_m`, `slide_m`, `foot_travel_m`, `ground_sink_m` and `loop_seam_m`, the failed checks and, under `left_out`, the reason. Every per-leg measurement is in `<output>.fit.json` beside the GLB, and `motion.fit_report` names that file. `bin/forge-fit check` on a non-humanoid GLB runs the same measures without Blender. Requested clip names other than idle, walk and run are listed under `motion.skipped`.

The animation library has no animal or creature takes: KayKit, 100STYLE and the Quaternius UAL are human skeletons, and Kimodo is human-only. Four-legged bodies therefore use the procedural gaits too.

## Job scratch and rig seeds

Each `bin/forge` stage creates a private directory under `out/work/`. By default Forge deletes it when the stage succeeds, fails, or handles SIGTERM or SIGINT. On error, it flushes the final JSON error line before deleting the directory. The exported GLB and adjacent manifest are the durable results. Manifest fields such as `input_image` for a generated concept and the commands under `timings` record the paths used during execution. Set `FORGE_KEEP_WORK=1` on the Forge process to inspect those files afterwards.

```sh
FORGE_KEEP_WORK=1 bin/forge gameready --mesh input.glb --poly-budget 5000 --output out/prepared.glb
bin/forge rig --mesh out/prepared.glb --seed 7 --output out/rigged.glb
```

On success, `work_retained` and `work_dir` in the manifest identify retained files. On a failed or cancelled retained run, the error gives the directory. An abrupt machine shutdown or SIGKILL can leave scratch behind because the process cannot run its cleanup. The two previously cited scratch files from the text-to-asset and in-app evaluations now live at `out/text_to_asset/prop/generate.log` and `out/text_to_asset/character/finished/finished.glb`.

Forge previously accepted the rig seed and discarded it. SkinTokens' demo has no seed argument. Forge now seeds Python, NumPy, the point-conditioning encoder and PyTorch, and uses the engine's single-process Blender loader for mesh sampling and weight transfer. The rig manifest records the effective seed (default 42). On the M5 Pro, two fresh seed-7 runs on the text-generated knight each produced 49 joints and byte-identical rigged GLBs (SHA-256 `2a7375d76f6cc6c482e5b3e3806a3f6d6dd0352357c6d17e3226222441056dc4`). The regression compares the skeleton nodes, skin data and complete binary buffer on the installed stack. Other devices and engine versions were outside this comparison.

### Verify finger names on a character rig

`bin/forge rig --mesh rigged.glb --hand-intake landmarks.json --output handfit.glb` checks an existing Unity-named SkinTokens rig. Supply an unrigged mesh to run the rigger first. The stage needs a front-facing bind pose with five separate fingertips on each side. Its input is the single skinned triangle primitive with four float weights produced by SkinTokens.

In the projected front view the whole character is 1000 units tall, the horizontal bounding-box centre is at x=550, and the sole is at y=1020. The origin is upper left. For each anatomical side (`Left`, `Right`), record a `wrist` and `palm` point, a palm-cut radius `cut`, maximum fingertip radius `reach`, and five named 2D points under `tips` (`Thumb`, `Index`, `Middle`, `Ring`, `Little`). These are artist-labelled anatomical landmarks. Reuse them across exports of the same aligned character.

The intake measures displacement on the actual skinned surface during a 20° proximal rotation of each chain. Five separate distal patches and a clear one-to-one motion assignment are required. When one control also moves a neighbour's distal patch substantially, only the foreign finger weights on that branch are reassigned. The output manifest records before/after matrices, renames, weight-row counts and maximum change to the rest surface. The character still needs a visual curl test for joint placement and deformation.

### Character detail and colour

Pass `--kind character` to select the character pipeline. Standard uses the native Pixal3D default at 1024 (eight steps on the installed build, twelve on an unpatched build). High uses twelve. Both pass the raw mesh and its calibrated views to game-ready, which allocates detail to head and hands at strength six and exports a separate LOD0. Forge sharpens the calibrated RGB images with Real-ESRGAN x4plus while preserving their alpha and camera transforms. The exported LOD0 pack includes collision hulls. Forge makes a display-only copy with the same UVs and camera frame for Pixel Match. Forge then copies its painted base colour into the full package so the display LODs share the result. A single wider feather softens the transition on oblique cheeks. On the test knight its median six-pixel luminance change along the visible cheek band fell from 31.6 to 11.9 on a 0–255 scale. A faint colour boundary remains. Rigging that package uses its display mesh.

`--quality draft` uses six sampling steps and the same detail allocation, and skips the upscaler and paint. `--source finish` keeps the engine Finish path available for comparisons. Prop and untyped generation keep their existing options. The receipt records the selected source, game-ready options, upscaled view sizes, projection outcome and timings. Use `./install.sh --stage engine --with-models` to provision the 67 MB upscaler along with the generation models.

### Hero Face at game resolution

On the text knight, the reconstructed head improved eye relief before reduction but left a visible collar seam. At a 15k budget the fused Hero route produced sharp holes across the hair and face. The RAW route with an upscaled 2K base-colour projection gave the better front and profile renders. Keep `--hero-face` experimental.

## Physical size and repeated preparation

`forge generate --height-m 1.8` sets a character's vertical extent in metres after baking and projection. `forge gameready --longest-side-m 0.15` can size a small prop in any orientation. Choose one option and supply a finite positive length. Omitting both preserves the original scale. Pixal3D typically produces a model about 0.9 m across. Model manifests from `bin/forge` include `size_m` as [width, height, depth] in glTF metres, measured on the visible mesh in its default pose.

A second `bin/forge gameready` on a prepared Forge GLB preserves its geometry, materials, LODs and collision and reports `reused_gameready: true`. The original triangle budget carries through. The GLB itself identifies the package, including older Otto copies with the original named LOD and collision nodes. Supplying a size converts the vertex coordinates and scales collision along with the display mesh.

## Faster first looks (opt-in)

These options shorten the wait before a person sees a piece. Defaults are unchanged. Measured numbers and renders are in `out/speed-evidence/`.

- `forge concept-server [--port 7831]` keeps FLUX warm. It starts `sd-server` under the heavy lock with the same weights, sampler, steps and CFG as the concept step, on 127.0.0.1. When the server answers, the command prints one line on stdout: `{"ready": true, "url": "http://127.0.0.1:7831"}`. It serves until SIGTERM or SIGINT, then stops the server's whole process group and exits 0. A server that exits on its own ends the command with `{"error": ...}` on stderr and exit 1. The server holds about 8.5 GB and a lock slot, so start it for a batch of pictures and stop it afterwards.
- `forge generate --concept-server URL` draws the picture on that server. The pixels match sd-cli's for the same seed. The time per 1024² picture drops from about 40 s to 27 s because FLUX and Qwen3 are not reloaded.
- `forge generate --concept-only` draws only the concept picture and writes the PNG to `--output`. A later `forge generate --image that.png --seed S` builds the same model as a one-go run with seed S.
- `forge generate --prompt TEXT [--kind character|prop] [--body B] [--hero-face] --print-prompt` draws nothing, needs no `--output`, takes no lock and starts no model. It prints one JSON line on stdout and exits 0: `{"prompt": ..., "negative_prompt": "", "width": 1024, "height": 1024, "kind": "character"|"prop"|null, "body": B|null, "cloud_suffix": ...}`. `prompt` is the exact text the concept step would send to FLUX. `cloud_suffix` is one more sentence for a cloud picture model (Codex, Gemini): one square picture of the subject alone, all of it in view, with no text, labels, border or second view. Send `prompt` followed by `cloud_suffix`, then pass the picture back with `forge generate --image`. `--image` or a missing `--prompt` is a clean error, exit 1.
- `--body humanoid|quadruped|bird|legless|other` picks the character's pose words for the concept picture (the drawn picture, `--concept-only` and `--print-prompt`) and implies `--kind character`. It is refused with `--kind prop`.
  - **humanoid:** the A-pose with open hands and spread fingers.
  - **quadruped:** standing on all four legs, side-on and turned three-quarters, legs apart, the tail held out clear of the legs. The cloud-pictures spike showed fur between the legs read as a fifth leg.
  - **bird:** standing, wings folded but distinct, both legs visible.
  - **legless** (snakes, fish, worms): the whole length in a loose curve that does not cross itself.
  - **other:** the whole body in view, limbs apart.

  With a `--body`, the face words (eyes, eyelids, expression, hair masses) are added only for a humanoid with `--hero-face`, so robots and animals do not grow human features. Without `--body`, `--kind character` keeps its wording from `prompt_recipes.json`, unchanged.
- `forge generate --skip-gameready` writes the generator's own mesh as a draft. The draft has about 1M triangles, a 4K atlas and takes about 40 MB. It has no LODs, collision or character projection.
  - To finish it, run `forge gameready --mesh DRAFT --poly-budget N --output FINAL` with the same budget the draft was asked for.
  - For a prop, the finish gives the same LODs a one-go run makes.
  - For a character (`--kind character`, raw source), the draft keeps a sidecar folder, `<output>.finish/`, beside the draft. The folder holds the generator's calibrated view (`raw.svviews`, about 1 MB). The draft manifest names it: `"finish": {"kind": "character", "quality": ..., "inputs": "<output>.finish"}`. It also records the `size_factor` the draft was scaled by.
  - `forge gameready` on such a draft first scales the mesh back to the generator's frame (within 3e-8 m). It then runs the one-go path: game-ready with head and hand detail, the 4× face projection, and the requested or recorded size. When the finish succeeds, it deletes the sidecar.
  - Keep the sidecar folder next to the draft when you move the draft. Without the folder the finish is refused.
- On the same generator mesh, the draft-then-finish route writes a byte-identical GLB to the one-go run. Two separate Pixal3D runs from the same picture and seed are not bit-identical: they produce the same token count but different HR and texture floats. So a draft and a later one-go run can differ as much as two one-go runs do.

```sh
bin/forge concept-server --port 7831 &          # wait for {"ready": true, ...} on stdout
bin/forge generate --prompt "a small red fox standing" --kind character --concept-only \
  --concept-server http://127.0.0.1:7831 --output out/fox.png
kill -TERM %1
bin/forge generate --image out/fox.png --prompt "a small red fox standing" --kind character \
  --poly-budget 8000 --height-m 0.45 --skip-gameready --output out/fox_draft.glb
bin/forge gameready --mesh out/fox_draft.glb --poly-budget 8000 --output out/fox.glb
```

Pixal3D sampling steps below 6 are not a draft setting. At 4 steps the fox and the telescope broke into floating fragments.

## Poses that fit the scene

`forge pose --mesh rigged.glb --pose sit --seat-height 0.35 --look-at 0,0.3,1.1 --output posed.glb` holds a rigged character in a pose and adds it as the default clip. The poses are sit (with a seat height), kneel, crouch, lie, curl, hold-out (palm up) and look-at. Every existing clip is kept. It runs on the CPU in under half a second, with no Blender and no GPU. The manifest reports where the body rests on the seat, whether the feet touch the floor, the palm point and normal, the head and gaze, the posed bounds and a self-check. A pose that would collapse the skin or push it through the floor or seat is refused. See [pose.md](pose.md). With `--swimming-fish`, the pose stage measures the head direction on a saved fish rig and reports `forward` in model coordinates for layout.


### Posing a musician or a supported body

`forge pose` accepts `lean` and the modifier `cross-arms` on a humanoid rig. A lean returns the sampled rear surface of the posed skin at three heights in `contact.back`. A head with an articulated upper body reports a surface landmark in `head.mouth`.

The pose stage can use a real axial wind instrument GLB passed as `--wind-instrument` and its scene scale relative to the performer as `--wind-scale`. The engine estimates the mouthpiece and bell, sets both hand targets and writes the instrument's rigid mounting transform to `wind_mount`. A pair of held objects uses the regular hand flags plus `--other-hold-style` and an optional `--other-hold-toward` for the opposite hand. Upright string players take `--fingerboard` and `--strings` as the two contact points, in metres relative to the performer's floor in the same frame as `--look-at`.

A planted shaft uses `--plant-at x,y,z` in the actor's own metres. An articulated radial drummer accepts two measured drumhead targets and alternates strikes in a loop. A character explicitly peeking below furniture can use `peek` to lie low while looking out. Measured contacts belong to the posed version used in that scene.

The local drawing check can reject a standing human outline on an animal, separate feet on a continuous snow base, or the wrong number of visible arms on an explicitly eight-armed octopus. A rejection receives one corrective drawing before the shape stage. The receipt records both attempts and their seeds. An explicitly counted wheeled chassis can be levelled when its lower surfaces resolve into matching pairs across axles. A dark creature with requested pale eyes can receive a material highlight on existing paired iris texture. The ordinary draft-and-bake path finishes the material on the delivered mesh.
