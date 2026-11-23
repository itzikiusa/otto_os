# Motion from words (NVIDIA Kimodo)

`bin/forge-motion` makes a rigged humanoid perform a motion described in a sentence. NVIDIA Kimodo
generates the motion locally on its own 77-joint SOMA skeleton, forge-animlib's retargeter bakes it onto
the character with rest-pose compensation and grounding, and `forge-fit check` measures the clip.

> **Status (2026-11-20): runs on real prompts.** With the Llama 3 encoder in place, all five stage tests pass, and
> the seven example prompts were generated and inspected (table below). Use it for whole-body motion: jumps,
> falls, walks, swings. Do not use it for actions where a hand must reach a prop. In those tests the hands stayed
> 30-50 cm from the target, where forge-fit's IK gets within 5-11 cm.

```sh
./install.sh --stage motion --with-models      # 17.5 GB; the Llama 3 part needs access (below)
bin/forge-motion --prompt 'jumps forward and lands' --character out/rigged.glb \
    --seconds 3 --seed 1 --output out/jump.glb
```

Options: `--seconds` 1-10 (default 5; Kimodo's range), `--seed` (default 0), `--name` (clip name; default from
the prompt), `--steps` (denoising steps, default 100), `--device mps|cpu` for the motion model (default mps),
`--height` in metres for the fit check (default 1.8). The output GLB holds the character and one clip with that name.
`<output>.manifest.json` records the prompt, the seed, the model and pinned revisions, the licences, the timings
and the fit-check summary (each clip passes, or lists the failing measures with value, limit and frame). Progress is
JSON Lines on stderr. A failure ends with `{"error": ...}` and a non-zero exit. SIGTERM or SIGINT kills the
generation or Blender process group. The work directory is removed unless `FORGE_KEEP_WORK=1` is set. Generation,
Blender and the fit check each take a shared heavy slot.

## Access to the text encoder

Kimodo encodes the prompt with LLM2Vec adapters (MIT) on top of `meta-llama/Meta-Llama-3-8B-Instruct`. To get it,
open https://huggingface.co/meta-llama/Meta-Llama-3-8B-Instruct while logged in, submit the licence form and wait
for approval. Then log in on this Mac with `hf auth login`. The fetch script reads the token from
`~/.cache/huggingface/token` and never copies it. The 16.07 GB download took 90 minutes here (2026-11-20). `bin/forge-doctor` shows the motion stage as `needs_models` until the
weights are present.

## How it runs on Apple Silicon

- **Install:** Kimodo `e1c219c` from `engines/kimodo`, installed without dependencies into `forge/motion/.venv` (Python 3.11,
  `forge/motion/requirements.txt`, which leaves out the web demo packages). Its `MotionCorrection` C++ extension
  compiles x86 SSE/AVX intrinsics, so the installer sets `SKIP_MOTION_CORRECTION_IN_SETUP=1`. Kimodo's foot-skate
  post-process is therefore off. Forge's grounding still plants the feet.
- **Devices:** Kimodo picks `cuda` or else the CPU. `forge/motion/generate.py` passes the device itself. On MPS, some
  SOMA skeleton assets load as float64, which MPS cannot hold. The worker casts them to float32 at load time,
  the change upstream PR #45 makes in `kimodo/skeleton/base.py`. Kimodo's own files are not patched. The text
  encoder runs on MPS in bf16.
- **Measured with the encoder** (3-6 s clips, 100 steps, M5 Pro under load): encoder load 15-24 s, generation
  8-11 s on MPS and 34-47 s on CPU, so MPS is about 4x faster. Peak RSS is 9-12 GB, MPS driver memory 14-15 GB,
  and the peak memory footprint is 22.5 GB. A run takes 26-36 s in the worker, plus about 6 s for the retarget and
  the fit check, plus the wait for a heavy slot. CPU and MPS give different motions for the same seed, because
  the noise comes from a different generator. Each device repeats itself exactly. The quality is the same: the
  CPU jump is a running leap of 1.0 m and the MPS jump a two-footed jump of 1.4 m.
- **Encoder load log:** transformers 5.1 prints a MISSING/UNEXPECTED table for the LoRA keys while it loads the
  adapters. The adapters do load. Checked by hand: the merged mntp delta matches 2·B·A of the adapter file to
  bf16 rounding, and the supervised adapter's weights equal the file's. Paraphrases score 0.87 cosine similarity
  and unrelated prompts 0.54-0.63.
- **Skeleton:** the BVH is written with Kimodo's standard T-pose rest. Eight SOMA joint names are renamed to Mixamo
  names so forge-animlib maps them correctly: SOMA `LeftLeg` is the thigh and `Chest` sits above `Spine2`.
  Fingers are not transferred.

## Tests

```sh
python3 tests/motion/test_motion.py                               # real models; skipped with a reason when absent
FORGE_MOTION_STUB_ENCODER=1 python3 tests/motion/test_motion.py   # pipeline only, prompt ignored
```

Do not wrap the suite in `tools/heavy.sh`. The stage takes its own slots, and a wrapped run holds both. The tests
run the stage on the text-to-asset knight and check five things: a clip of the requested length exists, the hips
travel and at least ten joints rotate, the fit check finds no foot penetration after grounding, the same seed gives
the same channels, and SIGTERM leaves no process, output or work directory. With the real encoder, all 5 pass
on MPS (2026-11-20, 750 s, mostly waiting for a slot). In stub mode the movement test fails as expected, because
unconditioned motion is a near-static idle.

`tests/motion/evidence.py` generates the owner's example prompts on the knight and the archer warrior into
`out/motion-evidence/`. `tests/motion/strip.py` renders front and side strips and measures how close each palm
comes to a forge-fit prop anchor, such as `sword.grip` from `pull_sword` or `quiver.opening` relative to the chest
from `shoot_bow`. `--cpu` writes `<id>_cpu.*` beside the MPS files.

## Real-prompt results (2026-11-20, seed 1, MPS)

| Prompt | What the body does | Verdict |
| --- | --- | --- |
| jumps forward and lands | crouches, swings the arms, jumps 1.3-1.4 m forward, lands in a crouch, stands | pass |
| stumbles backward and falls | slumps, sits back and falls onto its back, 0.9 m backward, lies flat; hardly any backstep | pass (fall); weak stumble |
| walks tired and slow | walks 2.9 m in 5 s (0.6 m/s) with the head lowered | pass |
| swings a sword in a wide arc | wide stance, raises the right arm overhead and cuts down across the body, back to a guard | partial: a strike, not a wide horizontal arc |
| stands idle, breathing and shifting weight | stands with 3 cm of hip travel and slight sway | pass, but it does not loop |
| pulls a sword from the ground with both hands | leans in, brings both hands together at waist height, lifts them to the chest; never reaches down | fail for the prop: palms 31/42 cm from `sword.grip` (IK: 11/6 cm) |
| draws an arrow from a back quiver, draws and shoots | hunches over the hands, crosses both arms in front of the face once, drops them | fail: right palm 49 cm from `quiver.opening` (IK: 5 cm); hands 0.47 m apart at most (IK full draw 0.95 m) |

forge-fit check fails every clip on `limb_body_penetration_m`, between 3.5 and 9.6 cm against a 3 cm limit.
The library clips on these characters fail the same way. The fall also slides a foot 7.4 cm. `loop_seam_m`
applies only because the clip names contain "walk" and "idle", and Kimodo clips do not loop. Fixing a hand-prop
clip needs an arm-IK pass over the Kimodo body, which forge-fit does not have yet.
