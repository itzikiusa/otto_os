# 3D Studio, Voice Studio and Otto Forge

> **Status (2026-11-20): control plane shipped; local Forge pipeline partly
> verified.** `otto-media` (3D provider registry, stage jobs, TTS providers,
> clips, workspace export) and its UI (Design Hall 3D Studio **Generate** /
> **Pipeline** menus, Voice Studio at `#/design/tts`) are built. Otto Forge
> lives in the top-level `forge/` directory. Three Forge stages have stage
> guides with functional tests: **game-ready**, **animation library** and
> **voice**. The engine stages (`forge generate`, `forge rig`,
> `forge animate`) have no stage guide yet; only their Otto argv shape is
> unit-tested (`forge/tests/test_forge_argv.py`). Game-ready timings come from
> procedural Blender inputs, not an AI-generated mesh.

Otto does not ship a 3D or speech model. Otto is the UI, artifact graph, job
runner and export layer. A 3D job runs a **provider** (a local CLI or an HTTP
inference server) and commits its mesh into Design Hall as a new artifact or
version. A Voice Studio clip comes from a **TTS provider** (built-in macOS
`say`, or an HTTP server). **Otto Forge** is the open-source, Apple Silicon
pipeline that plugs into both:

```
prompt / image ──▶ forge generate ──▶ forge gameready ──▶ forge rig ──▶ forge animate   (3D Studio, custom_cli)
                                     bin/forge-gameready             bin/forge-animlib  (CLI only)
text ──▶ bin/forge-voice serve (Kokoro / Qwen3-TTS)                                     (Voice Studio, kokoro)
         bin/forge-voice speak --visemes  → WAV + mouth-cue sidecar                     (CLI only)
```

Every Forge stage writes its `--output` plus `<output>.manifest.json`, reports
progress as JSON Lines on stderr, ends a failure with a `{"error":"..."}` line
and takes the machine-wide 2-slot lock `forge/tools/heavy.sh` for heavy work.

Code: `crates/otto-media` (`http.rs` routes, `service.rs` jobs/synthesis and
concurrency, `store.rs` built-in seeds, `types.rs` limits and presets,
`gen3d/` drivers, `tts/` drivers, `net.rs` SSRF guard, `export.rs` workspace
export), `crates/otto-server/src/policy.rs` (RBAC), migration
`crates/otto-state/migrations/0221_design_3d_and_media.sql`;
UI `ui/src/modules/design-hall/studio3d/` (`Studio3D.svelte`,
`GenerateModal.svelte`, `Provider3DManagerModal.svelte`,
`WorkspaceExportModal.svelte`, `AnimationBar.svelte`, `providers.ts`) and
`ui/src/modules/design-hall/tts/` (`TtsStudioPage.svelte`,
`TtsProviderManagerModal.svelte`, `TtsFields.svelte`, `WaveformPlayer.svelte`,
`ttsPresets.ts`); Forge `forge/forge/cli.py` (engine stages),
`forge/forge/gameready/`, `forge/forge/animlib/`, `forge/forge/voice/`,
`forge/bin/`, `forge/install.sh`.
Contract: [`docs/contracts/api.md` § Native 3D Asset Generation & Built-in TTS](../contracts/api.md)
and `design_artifact_updated` in [`ws.md`](../contracts/ws.md). Forge guides:
[README](../../forge/README.md), [game-ready](../../forge/docs/gameready.md),
[animation library](../../forge/docs/animlib.md), [voice](../../forge/docs/voice.md),
[third-party notices](../../forge/NOTICE.md). 3D Studio itself:
[Design Hall §10](design-hall.md).

## 1. Setup

### 1.1 Install Forge

Requirements (from `forge/README.md`): native macOS arm64 and `uv`; Git, CMake,
full Xcode and the Metal toolchain for the engine build; Blender **5.2.2 LTS** in
Applications for the Blender stages; FFmpeg and ffprobe on `PATH` for dialogue
encoding and its tests; Python 3 on `PATH` for the animation and doctor
launchers. Keep at least **55 GiB** free on the volume: the installer and the
animation/voice tools stop below that floor. A full data install downloads
about **24 GB**.

From `forge/`:

```sh
./install.sh --plan --with-models      # print the selected payloads and sizes
./install.sh --with-models             # or: --stage engine|gameready|animlib|voice
bin/forge-doctor                       # readiness per stage; --json for automation
```

The engine stage pins AssetFurnace **v0.6.0** (commit `5f9040b`). Weights,
the engine checkout, `.venv/` directories and `out/` are per-install and stay
out of git. The stage launchers run Hugging Face offline once weights are
cached.

### 1.2 Connect 3D Studio

Open the provider list from the Design Hall lobby's **⋯ → 3D generation
providers…**, from 3D Studio's **Generate ▾ → Manage 3D providers…**, or from
the Generate dialog's **Advanced → Manage providers…**. The modal is **3D generation
providers**.

Otto Forge is the 3D provider. The Generate dialog shows no provider picker
while Forge is the only provider set up (enabled and ready); without Forge it
shows one install step (`./install.sh --with-models` in the Forge folder)
instead of the form. TRELLIS.2, Hunyuan3D-Omni, custom providers and Local:
Otto + Blender MCP wait behind a closed **Advanced** disclosure, in the dialog
and in the provider manager (with **Add provider**), until one is enabled and
ready; then it shows beside Forge (`split3dProviders` in
`ui/src/modules/design-hall/studio3d/providers.ts`).

Three built-in providers are seeded and cannot be deleted (only reconfigured
or disabled): **Otto Forge (local)** (`otto-forge`, `custom_cli` over
`local_cli`, enabled), **Microsoft TRELLIS.2** (`trellis-2`, HTTP) and
**Tencent Hunyuan3D-Omni** (`hunyuan3d-omni`, HTTP, carries a territory
licence warning and is never picked as the default).

The built-in Otto Forge provider is **Ready** only when Otto finds an
executable: its configured **Executable path**, else a `forge` on the daemon's
`PATH` or `~/otto-forge/bin/forge`, counted only when `forge-doctor` sits
beside it, so another tool named `forge` on `PATH` (the Forge Code agent CLI,
Foundry) is skipped (`detect_otto_forge_binary` in
`crates/otto-media/src/types.rs`). For the
in-repo pipeline, choose **Configure** and set **Executable path** to the
absolute path of `forge/bin/forge` in your checkout (it finds its own root, so
**Working directory** can stay empty). The built-in provider starts at the
3,600 s timeout cap because `generate` can run for about 25 minutes (see §7).
Then press **Test**: for a local CLI it only checks
that the executable (and the working directory, if set) exists.

The seeded argv is one line per argument:

```
{stage} --mesh {mesh} --image {image} --prompt {prompt} --output {output} --poly-budget {poly_budget} --seed {seed}
```

It matches `forge/forge/cli.py` exactly: `forge` has the subcommands
`generate`, `gameready`, `rig` and `animate`, every subcommand accepts all
six flags, and an empty value means "not given". There is no template/CLI
drift. Otto expands these placeholders: `{stage}`, `{prompt}`, `{image}`,
`{mesh}`, `{output}`, `{output_dir}`, `{format}`, `{seed}` (default `42`),
`{poly_budget}` (default `25000`). The built-in provider is the one Otto Forge
entry (it cannot be deleted, so the manager has no separate Forge preset). A
custom CLI provider needs an absolute **Executable path**: a bare name such as
`forge` is not looked up on `PATH`, so it stays "CLI executable not found".

### 1.3 Connect Voice Studio

Voice Studio is **Product → Voice** (the Stories · Designs · Voice ·
Learnings switcher), route `#/design/tts`; Design Hall's **⋯ → Voice Studio
(TTS)**, its **New ▾ → Voice / TTS Studio** and ⌘K **Go to Voice Studio (TTS)**
open it too. The built-in **macOS Built-in Voices** provider (`macos-say`,
default voice Samantha) works with no setup.

To use Forge voices:

1. Start the loopback server from `forge/`; it holds one heavy slot until
   stopped:
   ```sh
   tools/heavy.sh bin/forge-voice serve --port 7811
   ```
2. In Voice Studio, **Providers** opens **TTS voice providers**. Add a provider
   with **Provider type** `Kokoro (local server)` (or
   `OpenAI-compatible (/v1/audio/speech)`), **Endpoint URL**
   `http://127.0.0.1:7811/`, the **Allow local/LAN TTS server endpoints**
   box ticked (otherwise the SSRF guard refuses loopback), and no API key.
3. **Test** probes `GET /health`. Voices come from `GET /v1/audio/voices` and
   are cached for 5 minutes.

### 1.4 Access

Feature grants come from `crates/otto-server/src/policy.rs`; handlers add the
workspace role.

- **3D:** `design` View lists providers, presets and jobs; Edit (plus workspace
  Editor) starts and cancels jobs and exports to the workspace; provider
  create/update/delete/test need `design` Admin, are `RouteClass::Admin` and
  refuse agent session credentials.
- **TTS:** `product` View lists providers, voices and clips; Edit (plus
  workspace Editor) synthesizes, deletes and exports clips; provider writes
  and tests need `product` Admin, are `RouteClass::Admin` and refuse agent
  credentials.

## 2. Walkthrough

### 2.1 3D Studio jobs and what Forge runs

In a 3D artifact, **Generate ▾ → Generate 3D asset (Hunyuan3D / TRELLIS.2 /
Custom)…** or **Image → 3D model…** opens **Generate 3D asset**. On a GLB/glTF/OBJ
artifact, **Pipeline ▾** (shown when an enabled, ready provider supports a
non-generate stage) offers **Make Game-Ready (Retopo / Bake)…**, **Rig
Skeleton…** and **Animate Model…**. A job can also chain stages with
`then_stages` (for example `generate` then `rig` and `animate`): the daemon runs
them one after the other in the same job, each committing its own version, so
the chain keeps going when the screen that started it closes (Create's
one-button character uses this; see [Create](create.md)). Each posts `POST /design/3d/jobs`
(`202`) with `stage` and, for transform stages, the current artifact as
`source_artifact_id` (head or `source_version_id`) and target. Presets
(`low_poly_prop` 5k tris, `stylized_character` 25k, `environment_asset` 40k,
`hero_mesh` 100k) fill `poly_budget`.

The job waits for one of 2 slots, runs the provider, validates the bytes
(GLB `glTF` header, glTF JSON object, or non-blank UTF-8 OBJ; size cap),
reads `<output>.manifest.json` (≤ 256 KiB) into the version's provenance (kept
only on the job when it would push the provenance past otto-design's 16 KiB
cap, as a long prompt can) and commits a named version (`Generated via …` /
`Stage '…' via …`). The UI polls the job. Progress: Queued 5 %, then 20 %
with the stage's own label (`Generating 3D mesh`, `Making game-ready`,
`Rigging skeleton` or `Animating` `with <provider>`), then the CLI's
progress: every stderr line that parses as `{"stage","pct","message"}` sets
the job's `progress_pct` and `stage` (the `message`, else the stage name),
written at most once a second, never moving backwards and capped at 99 until
the version is committed (`Saving 3D artifact version`, then 100 % `Done`).
Other stderr lines are only kept: on failure the error is Forge's
`{"error": "..."}` text when the CLI printed one, else the last 8 KiB of
stderr (`run_cli_argv` in `gen3d/mod.rs`). A CLI that prints no progress
shows the 20 % label until it ends.

What each stage runs in `forge/forge/cli.py`:

| Stage | Forge does |
|---|---|
| `generate` | FLUX.2 klein 4B concept picture (skipped when an image is given), BiRefNet-lite cut-out, Pixal3D mesh ("about 5-10 minutes"), AssetFurnace finish to `--poly-budget` faces, then `bin/forge-gameready` (`--tris` budget, 3 LODs, collision) when installed. GLB only. |
| `gameready` | `bin/forge-gameready MESH --tris N --lods 3 --collision` (§2.2). |
| `rig` | SkinTokens humanoid skeleton and skin weights, Unity Humanoid bone names. |
| `animate` | Retargets AssetFurnace's bundled motion presets (default `idle,walk,run`; Otto passes no `--clips`) and merges them into one GLB. Needs a rigged mesh. |

Measured through Otto's API with the built-in provider on an M5 Pro, 48 GB,
2026-11-20 (one job at a time, GPU otherwise idle): `generate` from Forge's
knight `concept.png` at 25,000 faces 236 s (cut-out 3 s, Pixal3D 192 s,
finish 22 s, game-ready 18 s; 23,075 triangles, 3 LODs, 8 hulls); `gameready`
on its 7 MB finished knight at 15,000 18 s; `rig` 52-65 s; `animate` 15 s
(`idle`, `walk`, `run`). Forge's own text-to-asset runs took 289 s (knight)
and 1,535 s (chest), hence the 3,600 s timeout. Those runs predate the
progress reading above (they showed 5 / 20 / 80 / 100 % only).

The engine sets `I3D_NO_USAGE=1`, `DO_NOT_TRACK=1`, `I3D_NO_NOTICES=1`,
`HF_HUB_DISABLE_TELEMETRY=1` and `HF_HUB_OFFLINE=1`. **AnimationBar** plays the
clips of a loaded GLB. **Export ▾ → Export to Workspace…** writes the artifact
into the workspace repository and returns its SHA-256.

### 2.2 Game-ready (`bin/forge-gameready`)

```sh
tools/heavy.sh bin/forge-gameready in.glb --output out/asset.glb \
  --tris 15000 --texture 2048 --lods 3 --collision --pack out/pack
```

Headless Blender 5.2.2: join and clean (refuses skinned input), collapse
decimation to the budget (or `--quads` QuadriFlow at `tris/2`), xatlas UVs,
CPU Cycles bake of normal, base colour and packed ORM, LODs at 50% / 25% /
12.5%, up to 8 CoACD hulls of ≤ 64 vertices named `UCX_<name>_NN`. One Y-up
GLB with embedded maps; `--pack` adds per-LOD GLB/FBX and PNG maps. The first
run installs `coacd==1.0.14` and `xatlas==0.0.11` into
`forge/gameready/.venv`.

Measured on an M5 Pro, 48 GB, 2026-11-19, procedural inputs
([details](../../forge/docs/gameready.md)); add about 1 s of Blender start-up:

| Input | Output | Total |
|---|---|---|
| sculpt, 510,720 tris | 15,000 / 7,500 / 3,750 / 1,874 tris, 3 × 2048 maps, 8 hulls | 12.6 s |
| same, `--quads --tris 8000` | 4,075 quads / 4,074 tris, 3 × 1024 maps | 16.3 s |
| hollow tube, 204,812 tris | 4,999 / 2,499 / 1,249 tris, 2 × 1024 maps, 8 hulls | 33.9 s (29.7 s CoACD) |
| 6 parts, 39,360 tris | 8,000 / 4,000 / 2,000 tris, 3 × 1024 maps, 8 hulls | 18.3 s |

### 2.3 Animation library (`bin/forge-animlib`, CLI only)

```sh
bin/forge-animlib list --category locomotion | jq '.[] | {id, duration, root_motion, loopable}'
bin/forge-animlib apply --mesh character.glb --clips walk,run,idle --output out/character-animated.glb
```

895 clips: 85 KayKit actions (CC0) and 810 trimmed 100STYLE takes (CC BY 4.0);
24 are loopable. `walk` / `run` / `idle` select KayKit `Walking_A` /
`Running_A` / `Idle`; 100STYLE IDs are `100style-<style>-<movement>`. The
target needs one humanoid armature with identifiable hips, head and bilateral
arm/leg joints (Unity Humanoid or Mixamo names). The manifest lists clip
durations, licences and required credits. Otto's **Animate Model…** does not
call this tool; it runs `forge animate` (§2.1).

### 2.4 Voice Studio and `bin/forge-voice`

Voice Studio synthesizes one line, or a script of `CHARACTER: line` rows in
batch (a row's character picks a matching preset). Character presets are
stored per workspace in the browser's `localStorage` (`ttsPresets.ts`).
Clips land under `<data>/media/tts/`, play in the waveform player and export
with **Export to Workspace**. For `kokoro` / `openai_compat` providers Otto
offers WAV, MP3 and M4A (sent as `aac`) and sends
`{model, input, voice, response_format, speed}` to `/v1/audio/speech`; the
default model `kokoro` makes Forge resolve its character presets, so
`ember` and `flint` run on Qwen3-TTS.

Forge characters (`forge/forge/voice/characters.json`): `keeper` (Kokoro
`am_onyx,am_michael` blend, 0.92×), `scout` (Kokoro `af_heart`, 1.06×),
`ember` (Qwen VoiceDesign, low husky female), `flint` (Qwen VoiceDesign,
gravelly male storyteller, 0.94×). The CLI also takes 28 raw English Kokoro IDs,
comma-separated blends, or `--engine qwen --voice designed --description '…'`.

Mouth cues from the CLI:

```sh
tools/heavy.sh bin/forge-voice speak --voice keeper --text 'Follow the lantern.' \
  --format wav --output out/keeper.wav --visemes
```

PocketSphinx 5.1.1 aligns the final waveform to the transcript and writes
`<output>.visemes.json` (`{"duration_sec", "cues":[{start,end,value}]}`, shapes
`A`–`H` and `X` rest, 10 ms resolution).

Mouth cues over HTTP (the viseme contract with the Forge voice server): for a
`kokoro` provider Otto adds `X-Otto-Visemes: 1` to `POST /v1/audio/speech`.
A server that aligned the clip answers with the audio plus
`X-Otto-Visemes-Id: <id>`; Otto then fetches `GET /v1/audio/visemes/<id>`
(the speech URL's sibling, so the same origin, client and SSRF /
`allow_local_network` rules; id of letters, digits, `-`, `_`; ≤ 512 KiB;
10 s) and keeps the timeline in the clip's `visemes` (`visemes_json`) when it
has the Rhubarb shape `{duration_sec, cues:[{start, end, value}]}` (finite
times, `0 ≤ start ≤ end`, `value` `A`–`H` or `X`). No id header, a 404 (the
server keeps only its last 32 timelines), a timeout, an oversized or a
malformed body leave `visemes` null; the clip is still saved. A
`custom_http` provider can instead return JSON with `audio_b64` and
`visemes`. Voice Studio shows the cue count on the clip.

### Sounds and music (`bin/forge-sfx`, `bin/forge-music`)

Install Forge's audio stage (`./install.sh --stage audio --with-models`, 12.74 GB of weights). Otto finds `forge-sfx` and `forge-music` beside the `forge` it already uses; `GET /product/audio/engine` says `ready` or `not_installed` (with the install line). `POST /product/audio/jobs` with `kind` `sound` (an effect, or with `loop` an ambience bed) or `music` (optional `loop`, `bpm`), a `prompt` and `seconds` starts a job; poll `GET /product/audio/jobs/{id}` for the engine's own progress lines. An effect takes about 90 s, 20 s of music about 20 s, and jobs wait for each other and for Smart mode's 3D work. Lengths: sound 0.5–30 s (a loop at most 26.5 s), music 10–590 s (a loop at most 500 s). The WAV lands in the clip store as a clip of `kind` `sound` or `music`, so it plays, exports and deletes like a voice clip (`GET /product/tts/clips?kind=sound`), and its `manifest` keeps the model, licence (MOSS-SoundEffect Apache-2.0, ACE-Step MIT), loudness and loop-seam result. Listen before you ship: no one has checked the models' output by ear. Contract: `docs/contracts/api.md` → "Sounds and music".

### Smart mode: one description, the whole scene

`POST /design/productions` takes one plain description of a scene, for example "man pulling sword stuck in the ground, fights a monster which guards a chest, key on the monster's neck…", and an optional `use` (`phone`, `pc` or `closeup`, which sets the poly budgets). One agent turn plans it into items: characters, objects (every prop is its own item), voice lines, sounds and music. The plan also has a layout and plain notes about anything Otto cannot make yet. A wrong plan is retried once. You can edit the plan while it is `ready`.

`POST …/start` then makes the items one GPU job at a time on the built-in Otto Forge. A character runs generate, game-ready, rig and its moves (the plan's known moves reach `forge animate --clips`; an unknown one is left out with a note); an object runs generate; a voice line is made through TTS. Sounds and music are made by the sound engine (above), their length clamped to its limits with a plain message, and a nonverbal line (a grunt, a laugh) becomes a short sound; without the engine they are skipped with a plain message. Otto then writes one `scene3d` artifact with every model placed on the ground and the camera framing them all. Forge makes every model about one unit big, so the scene sizes each one by what it is: a character stands 1.7 m tall (a person; `scale_hint` `small` 1.0 m, `big` 2.7 m), and an object's longest side is 0.6 m (a chest; `small` 0.25 m, `big` 1.5 m). Spacing uses each model's bounds in world space, through its node transforms and, for a rigged character, its armature (`crates/otto-media/src/productions/scene.rs`). A failed item never stops the others. `…/cancel` stops the running job.

`…/steer` takes a comment and optional 3D pins, re-plans with as few changes as possible, and the next Start remakes only the changed and added items. `…/revert` brings back what a steering round replaced. After a daemon restart, a running production goes back to `ready` and Start makes the rest. A character's `holds` are gripped by its pose (`forge pose …,hold`), and `actions` are carried through. Contract: `docs/contracts/api.md` → "Smart mode: productions".

#### From an agent (MCP)

An agent session (or an outward MCP client) can do the same through the governed `otto.*` tools:
- `otto.list_3d_providers` shows which generator is ready.
- `otto.start_3d_job` starts `generate` (prompt and/or `image_artifact_id`) or `gameready` / `rig` / `animate` on `source_artifact_id`. Poll `otto.get_3d_job` for status, progress and the result artifact. `otto.cancel_3d_job` stops the job, and `otto.export_3d_artifact` writes the model into the workspace folder.
- Voice: `otto.list_tts_providers`, `otto.list_tts_voices`, `otto.synthesize_voice` (returns the clip id, `duration_ms` and the viseme timeline when one was stored), `otto.list_voice_clips`, `otto.delete_voice_clip`.
- Sounds and music: `otto.start_sound` / `otto.start_music`, then `otto.get_audio_job` (status, progress and the clip once made; `not_installed` when the engine is missing), `otto.list_audio_jobs`, `otto.cancel_audio_job`.
- Smart mode: `otto.create_production` (with `project_id` for a new scene of a project), then `otto.get_production` / `otto.list_productions`, then `otto.start_production` / `otto.cancel_production`; changes as in Create: `otto.steer_production` (words and pins), `otto.redraw_production_item`, `otto.revert_production`, `otto.rebuild_production_scene`.

Every writer costs local GPU minutes or writes artifacts or clips, so it is approval-gated. No tool manages providers or keys.

## 3. REST & WebSocket surface

All under `/api/v1`. Auth: feature grant from `policy.rs` + workspace role
from the handler; `[Admin]` = `RouteClass::Admin`, agent credentials refused.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /design/3d/providers | design View | — | `Media3dProvider[]` (built-ins `otto-forge`, `trellis-2`, `hunyuan3d-omni` + custom; `has_api_key` only) |
| POST /design/3d/providers | design Admin [Admin] | `CreateMedia3dProviderReq` | 201 `Media3dProvider` |
| PATCH /design/3d/providers/{id} | design Admin [Admin] | `UpdateMedia3dProviderReq` | 200 `Media3dProvider` |
| DELETE /design/3d/providers/{id} | design Admin [Admin] | — | 204; 409 for a built-in |
| POST /design/3d/providers/{id}/test | design Admin [Admin] | — | 200 `ProviderTestResult` |
| GET /design/3d/presets | design View | — | `GameAssetPreset[]` |
| POST /design/3d/jobs | design Edit + ws Editor | `StartMedia3dJobReq` (`stage`, body ≤ 24 MiB) | 202 `Media3dJob`; 409 disabled provider; 400 unsupported stage |
| GET /design/3d/jobs | design View + ws Viewer | `?workspace_id=&artifact_id=&limit=` (default 50; no filter = jobs in viewable workspaces) | `Media3dJob[]` |
| GET /design/3d/jobs/{id} | design View + ws Viewer | — | `Media3dJob` |
| POST /design/3d/jobs/{id}/cancel | design Edit + ws Editor | — | 200 `Media3dJob` |
| POST /design/3d/artifacts/{id}/export-workspace | design Edit + ws Editor | `{relative_path, version_id?, overwrite?}` | 200 `{path, absolute_path, size_bytes, format, sha256}` |
| GET /product/tts/providers | product View | — | `TtsProvider[]` (built-in `macos-say` + custom) |
| POST /product/tts/providers | product Admin [Admin] | `CreateTtsProviderReq` | 201 `TtsProvider` |
| PATCH /product/tts/providers/{id} | product Admin [Admin] | `UpdateTtsProviderReq` | 200 `TtsProvider` |
| DELETE /product/tts/providers/{id} | product Admin [Admin] | — | 204; 409 for `macos-say` |
| POST /product/tts/providers/{id}/test | product Admin [Admin] | — | 200 `ProviderTestResult` |
| GET /product/tts/providers/{id}/voices | product View | — | `TtsVoice[]` (cached 5 min) |
| POST /product/tts/synthesize | product Edit + ws Editor | `SynthesizeTtsReq` | 201 `TtsClip` |
| GET /product/tts/clips | product View + ws Viewer | `?workspace_id=` (required) `&story_id=&artifact_id=&kind=&limit=` (default 100) | `TtsClip[]` (`kind` voice, sound or music) |
| GET /product/tts/clips/{id} | product View + ws Viewer | — | `TtsClip` |
| GET /product/tts/clips/{id}/audio | product View + ws Viewer | — | audio bytes, `X-Content-Type-Options: nosniff`, `Cache-Control: private, max-age=3600` |
| DELETE /product/tts/clips/{id} | product Edit + ws Editor | — | 204 (row + file) |
| POST /product/tts/clips/{id}/export-workspace | product Edit + ws Editor | `{relative_path, overwrite?}` | 200 export receipt |
| GET /product/audio/engine | product View | — | `AudioEngine {state: ready\|not_installed, message}` |
| POST /product/audio/jobs | product Edit + ws Editor | `StartAudioJobReq {workspace_id, kind, prompt, seconds, loop?, bpm?, seed?, title?, story_id?, artifact_id?}` | 202 `AudioJob` (`queued`, or `not_installed`) |
| GET /product/audio/jobs | product View + ws Viewer | `?workspace_id=&kind=&limit=` | `AudioJob[]` |
| GET /product/audio/jobs/{id} | product View + ws Viewer | — | `AudioJob` (+ `clip` once made) |
| POST /product/audio/jobs/{id}/cancel | product Edit + ws Editor | — | 200 `AudioJob` |

`forge-voice serve` (binds `127.0.0.1`, default port 7811, accepts only a
`Host` of `127.0.0.1:<port>` or `localhost:<port>`): `GET /health` →
`{"ok":true}`; `GET /v1/audio/voices` → `{"voices":[{"id","name"}…]}`;
`POST /v1/audio/speech` → raw audio (`wav`, `mp3`, `aac`, `aiff`), plus
`X-Otto-Visemes-Id` when the request sent `X-Otto-Visemes: 1` and alignment
worked; `GET /v1/audio/visemes/<id>` → the timeline JSON (last 32 kept; 404
otherwise) (§2.4).

WebSocket: `otto-media` emits no events of its own. A committed job output
arrives as `design_artifact_updated` (`change` `created` or `content`) from
`otto-design`.

## 4. Capabilities & limits

| Limit | Value | Source |
|---|---|---|
| Concurrent 3D jobs / TTS syntheses | 2 / 4 | `service.rs` |
| Prompt | ≤ 4,000 chars | `types.rs` |
| Reference image | ≤ 12 MiB raw (job body ≤ 24 MiB) | `service.rs`, `http.rs` |
| Source mesh | ≤ 256 MiB | `service.rs` |
| 3D output | default 128 MiB, 1–256 MiB per provider | `types.rs` |
| 3D timeout | default 900 s, built-in Otto Forge 3,600 s (migration 0222 raises an unedited 900 s seed), 15–3,600 s (the UI form starts new providers at 300 s) | `types.rs`, `Provider3DManagerModal.svelte` |
| CLI argv template | ≤ 64 args, no NUL; `params` ≤ 64 KiB | `service.rs` |
| TTS text / voice id / speed | ≤ 10,000 chars / ≤ 128 chars / 0.5–2.0 | `service.rs` |
| TTS audio | ≤ 25 MiB; HTTP driver timeout 90 s (voice list 15 s) | `tts/http_drivers.rs` |
| Viseme timeline (kokoro) | ≤ 512 KiB, 10 s | `tts/http_drivers.rs` |
| 3D job progress writes | ≤ 1 per second, monotonic, ≤ 99 % before commit | `service.rs`, `store.rs` |
| Export extensions | 3D `glb gltf obj json`; audio `wav mp3 m4a aiff` | `export.rs` |

Not done yet:

- No stage guide for `generate`, `rig` or `animate` (§2.1 has one measured
  run of each through Otto); the §2.2 game-ready numbers come from procedural
  inputs.
- `forge generate` writes GLB only: the built-in provider offers only `glb`,
  and an API call that asks it for `gltf` or `obj` fails Otto's validation.
- Otto cannot pass a clip list or texture size to Forge, and its **Animate**
  stage uses the engine presets, not the 895-clip library.
- Topology is decimation class (default) or QuadriFlow (`--quads`): no
  deformation edge loops; no AO, emission or alpha bake; skinned input refused.
- The animation library needs a humanoid; contact is on a flat plane; no
  facial or finger animation.
- Voice tests check signal level, duration, formats and the viseme timeline,
  not acting quality; separate Qwen VoiceDesign lines can vary in perceived
  speaker identity.

## 5. Licences for a game team

Each stage manifest records the tools, models and licences used; keep it with
the exported asset. [`forge/NOTICE.md`](../../forge/NOTICE.md) is the full
list. The points that change what a studio does:

- **100STYLE clips (CC BY 4.0):** any shipped `100style-*` clip needs the
  credit "The 100STYLE Dataset - Ian Mason", links to the dataset and licence,
  and a note of changes (trimming, retargeting). KayKit clips are CC0.
- **Meta DINOv3** (image encoder in the Pixal3D download): custom licence;
  commercial use without a revenue/MAU threshold, prohibited end uses and
  reverse-engineering limits apply, and its weights may only be redistributed
  under Meta's agreement with a copy.
- **Pixal3D / TRELLIS.2 checkpoint set:** MIT per the publisher's per-file
  index; keep Tencent's and Microsoft's notices.
- **Hunyuan3D:** Tencent's licence excludes the EU, UK and South Korea
  (including outputs); Forge uses no Hunyuan weights, and Otto shows a
  **Territory restriction** badge on the `hunyuan3d-omni` provider.
- **Voice runtime:** Kokoro and Qwen3-TTS weights are Apache-2.0, but the
  English frontend loads GPL eSpeak NG and the tested FFmpeg is a GPL build.
  Baked WAVs and mouth cues do not contain these programs; shipping the
  runtime needs its own review.
- **Blender (GPL):** meshes, textures and animations made with it keep their
  own rights; redistributing Blender or bpy brings GPL obligations.
- Excluded from the path: nvdiffrast/nvdiffrec (research-only) and BRIA
  RMBG-1.4/2.0 (commercial grant needed).

## 6. Security notes

- Provider writes and tests are Admin-only, `RouteClass::Admin` and refused
  for agent credentials: a provider can name a local executable or an
  endpoint that receives a Keychain key.
- API keys go to the Keychain (`otto:media3d:<id>:api_key`,
  `otto:tts:<id>:api_key`); responses carry only `has_api_key`.
- A CLI provider runs strictly as argv (no shell), with stdin closed, a
  regular-file executable check and a timeout, in its own process group: a
  cancel or timeout sends SIGTERM to the group (Forge then stops its model
  and Blender children), waits 5 s and sends SIGKILL; dropping the job
  SIGKILLs the group.
- Every HTTP call goes through `otto-netguard`: loopback and private ranges
  need `allow_local_network`; cloud metadata and Otto's own listener are
  always blocked; a key over plain HTTP to a non-loopback host is refused.
- Workspace export requires a workspace `root_path` and a relative path with
  an allowed extension, refuses `..`, symlink escapes, Otto's data directory
  and credential directories, never overwrites without `overwrite: true`, and
  writes atomically.
- `forge-voice serve` listens on IPv4 loopback only and rejects other
  `Host` headers.

## 7. Troubleshooting

- **Otto Forge shows "Not installed" / "CLI executable not found":** set
  **Executable path** to the absolute path of `forge/bin/forge`; run
  `./install.sh --stage engine --with-models` and `bin/forge-doctor` if the
  stage then fails on a missing `.venv` or weight.
- **`generate` times out:** Pixal3D alone takes about 5-10 minutes, plus the
  concept image, finish and game-ready passes, and the heavy lock can queue
  the job. The built-in Otto Forge provider already uses the 3,600 s cap; raise
  **Timeout (seconds)** on a custom provider that runs `forge`.
- **Validation failed on a Forge job:** the output format was `gltf` or
  `obj` (an API call, or a custom provider running `forge`); Forge writes GLB.
- **Voice provider refused or unreachable:** tick **Allow local/LAN TTS server
  endpoints**, keep `forge-voice serve` running, and use the endpoint
  `http://127.0.0.1:7811/` (the server rejects other `Host` names).
- **TTS request fails at 90 s or 25 MiB:** the first request loads the models;
  synthesize short lines.
- **Rhubarb fails with `Bad CPU type in executable`:** the upstream 1.14 macOS
  binary is Intel-only; `--visemes` already uses PocketSphinx.
- **`--quads` refuses the mesh:** QuadriFlow needs a closed mesh after
  cleaning; run without `--quads` or repair the mesh, and check `clean` in the
  manifest.
- **Game-ready refuses a rigged mesh:** run game-ready before rig and animate.
- **Hulls render as meshes in Godot:** rename `UCX_` nodes to `-convcolonly`
  or hide them on import.
- **`forge-animlib` reports missing joints (`bone_0` …):** export the rig with
  Unity Humanoid or Mixamo bone names; stderr lists the missing joints.
- **"less than 55 GiB free":** free disk space; keep the 1.47 GB 100STYLE
  archive compressed (`apply` extracts only the selected BVH files).
- **Pipeline ▾ is missing:** it appears only on GLB/glTF/OBJ artifacts and only
  when an enabled, ready provider supports a non-generate stage.
