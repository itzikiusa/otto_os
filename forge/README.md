# Otto Forge

Otto Forge runs game-asset tools locally on Apple Silicon. It generates mesh assets from text or an image, prepares meshes for games, rigs any body (two legs, four, eight, one or none), animates it, retargets animation clips and synthesizes character dialogue. Otto can launch the same commands as a custom CLI provider. Otto’s Kokoro HTTP driver can request lip-sync by sending `X-Otto-Visemes: 1` with speech. Forge returns the audio and, when alignment succeeds, an `X-Otto-Visemes-Id` for fetching the timeline at `GET /v1/audio/visemes/<id>`. [Speech and lip-sync](docs/voice.md#otto-speech-provider)

## Install

Run these commands from the directory containing this README. The same layout works in a standalone checkout and in the Otto repository’s top-level `forge/` directory.

```sh
./install.sh --plan --with-models
./install.sh
bin/forge-doctor
```

The default install provisions runtimes. Select `--with-models` to download weights and animation packs. A fresh full data install needs about **24 GB** of downloads in addition to Python packages, the engine checkout, build tools and their caches. The installer prints the selected payloads and approximate sizes before downloading. For a single stage, include its name.

```sh
./install.sh --stage voice --with-models
./install.sh --stage animlib --with-models
bin/forge-doctor --json
```

The stage names are `engine`, `gameready`, `animlib` and `voice`. Engine covers text/image generation, rigging any body and the bundled animation presets. A repeat run reuses the pinned checkout and local packages. On a network interruption, run the same command again. The pinned AssetFurnace runtime is **v0.6.0**, commit `5f9040bfa00e8269104930a0f918a68d828ccd87`. Its frozen uv lock sets the engine Python dependencies. Voice and SkinTokens use separate Python 3.11 environments. Game-ready loads CoACD and xatlas into Blender’s own Python 3.13.

You need native macOS arm64 and `uv`. The engine build needs Git, CMake, full Xcode and the Metal toolchain. Install Blender **5.2.2 LTS** in Applications for the Blender stages, and put FFmpeg and ffprobe on PATH for dialogue encoding and its tests. Python 3 must be on PATH for the animation and doctor launchers. Setup runs as your user. The working volume should have at least 55 GiB free throughout provisioning, with additional headroom for the selected downloads. Check distribution terms before shipping, especially for DINOv3, the Pixal checkpoint bundle and motion-capture credits. [Third-party notices](NOTICE.md)

## Commands

| Stage | Example from the install root |
| --- | --- |
| Generate from text | `bin/forge generate --prompt 'stylized knight in blue and silver armour' --kind character --poly-budget 15000 --output out/gatekeeper.glb` |
| Generate from an existing concept | `bin/forge generate --image chest.png --kind prop --poly-budget 5000 --output out/chest.glb` |
| Prepare geometry, textures, LODs and collision | `tools/heavy.sh bin/forge-gameready raw-scan.glb --tris 15000 --texture 1024 --lods 3 --collision --output out/prepared.glb` |
| Optional armour weight sharpening | `bin/forge-skinfix --mesh out/rigged.glb --stiffness 0.5 --output out/rigged-stiffer.glb` (default 0). |
| Rig | `bin/forge rig --mesh out/gatekeeper.glb --output out/rigged.glb` |
| Animate with engine presets | `bin/forge animate --mesh out/rigged.glb --clips idle,walk --output out/presets.glb` (a humanoid gets the presets; any other body gets procedural idle, walk and run for its own legs. [Any body](docs/engine.md#rig-and-animate-any-body)) |
| Browse and apply library clips | `bin/forge-animlib list --category locomotion` · `bin/forge-animlib apply --mesh out/rigged.glb --clips walk,run,idle --output out/animated.glb` |
| Hold props, close hands, author reaches, check the fit | `bin/forge-fit hold --character out/rigged.glb --prop sword.glb --hand right --output out/held.glb` · `bin/forge-fit reach --character out/rigged.glb --targets pull_sword --prop sword=sword.glb --output out/pull.glb` · `bin/forge-fit check --mesh out/animated.glb --output out/fit.json` |
| Speak with lip-sync | `tools/heavy.sh bin/forge-voice speak --voice keeper --text 'Follow the lantern.' --visemes --output out/keeper.wav` |
| Serve speech to Otto | `tools/heavy.sh bin/forge-voice serve --port 7811` binds to `127.0.0.1`. Set the Otto provider's Default model to `qwen` for voices designed from each character's description. |

Set a physical size with `--height-m 1.8` or `--longest-side-m 0.15` on `bin/forge generate` and `bin/forge gameready`. The manifest records `size_m` as [width, height, depth] in metres. A second game-ready pass preserves an existing prepared package.

For a close-up character, pass `--kind character`. Standard and high quality preserve head and hand detail, sharpen the calibrated concept views 4×, and paint the colour into the shared texture for every LOD. Draft keeps the detail allocation and skips projection. Use `--source finish` to compare the engine’s Finish mesh. [Character generation](docs/engine.md#character-detail-and-colour)

Generated packages already contain a budgeted display mesh, LODs and collision hulls. Rigging a Forge package fits the skeleton to its display geometry. LODs and hulls stay in the generated package. Use fresh output paths. Stages emit progress on stderr and write adjacent `<output>.manifest.json` receipts. The 30 engine presets were generated with NVIDIA Kimodo and carry NVIDIA Open Model License provenance. The Pixal3D bundle includes a converted DINOv3 encoder under Meta’s DINOv3 licence. The separate upstream TRELLIS.2 route requires gated DINOv3 access. For motion packs with explicit asset licences, the animation library includes 85 KayKit game actions and 810 trimmed 100STYLE capture takes. See the stage guides for formats, quality limits and examples. [Engine](docs/engine.md) · [Game-ready](docs/gameready.md) · [Animation library](docs/animlib.md) · [Voice](docs/voice.md) · [Fit](docs/fit.md)

On the M5 Pro with 48 GB, a prompt-driven knight took 4 min 49 s to generate and a denser chest took 25 min 35 s. The chest exceeded Otto’s default 15-minute local-stage timeout. Allow a longer timeout for comparable props. The knight’s rig and clips ran, but the face, fingertips and armour deformation need work for close-up gameplay. The chest is suitable for a static front-facing placement and needs rear material cleanup for inspection from all sides. [Measured prompt runs](docs/engine.md#first-prompt-driven-verification)

`bin/forge generate`, `gameready`, `rig` and `animate` delete their scratch files after success, failure or cancellation. To retain a single run’s intermediates and child logs for inspection, prefix its command with `FORGE_KEEP_WORK=1`. The manifest or final error identifies the retained directory under `out/work/`. The output GLB and its manifest remain at the requested destination. `forge rig --seed N` controls SkinTokens sampling and defaults to 42.

## Sound effects, ambience and music

`bin/forge-sfx` makes one-shot effects and seamless ambience loops with MOSS-SoundEffect v2.0 (Apache-2.0). `bin/forge-music` makes background music with ACE-Step 1.5 (MIT), instrumental by default and optionally loopable. `bin/forge-audio scene` renders a whole cue sheet of beds, spot effects and music, and writes a `manifest.json`. Output is 48 kHz WAV. Beds are levelled to −18 LUFS, music to −16 LUFS, and effects are peak-normalised to −1 dBTP. Install with `./install.sh --stage audio --with-models`, which downloads 12.74 GB of weights. On this Mac an effect takes about 90 s and 20 s of music about 20 s. Nobody has listened to the output yet: open `out/audio-evidence/index.html` and listen before you ship. [Audio guide](docs/audio.md)

```sh
tools/heavy.sh bin/forge-sfx --prompt 'wind on a ruined wall' --seconds 20 --loop --output out/wind.wav
tools/heavy.sh bin/forge-music --prompt 'dark fantasy underscore, low cello' --seconds 60 --loop --bpm 60 --output out/gate.wav
tools/heavy.sh bin/forge-audio scene --cue-sheet forge/audio/cues/market-noon.json --out out/market
tools/heavy.sh forge/audio/.venv/bin/python tests/audio/test_audio.py
```

## Check the working install

`bin/forge-doctor` reports the Blender version, engine revision, virtual environments, model paths and sizes, available disk and readiness per stage. Add `--json` for automation. A nonzero exit identifies a broken component or failed disk check. The diagnostic runs local probes without fetching models. Use the functional suites below for inference, deformation, rendering and media verification.

```sh
tools/heavy.sh forge/voice/.venv/bin/python tests/voice/test_voice.py
python3 tests/animlib/test_animlib.py
python3 -m unittest discover -s tests/skinfix
engines/assetfurnace/.venv/bin/python -m unittest tests.test_forge_argv
tools/heavy.sh tests/gameready/run.sh
tools/heavy.sh tests/fit/run.sh
```

The animation suite and `forge-animlib apply` acquire `tools/heavy.sh` internally for every Blender invocation. The game-ready suite rebuilds its own `out/gameready-tests/` directory. The speech suite starts and stops a temporary loopback server.

## Include the sources in Otto

Keep the active working install where its runtimes and models were provisioned. Copy the approved source files into Otto’s `forge/` directory, preserving executable bits and the nested ignore files. The engine checkout in `engines/`, downloaded assets and caches in `models/`, generated files in `out/`, stage `.venv/` directories and ignored test scratch belong to each working install. A fresh checkout can reproduce them with the installer.

After the intended sources have been tracked in the standalone repository, the following command previews the copy. Set `otto_root` to the receiving repository. Review the file list before removing `--dry-run`.

```sh
git ls-files -z | rsync -a --dry-run --from0 --files-from=- ./ "$otto_root/forge/"
```

## Motion from words

`bin/forge-motion --prompt 'jumps forward and lands' --character out/rigged.glb --seconds 3 --seed 1 --output out/jump.glb` uses NVIDIA Kimodo (code Apache-2.0, weights NVIDIA Open Model License) to generate the motion locally. It retargets the motion onto the character with forge-animlib's grounding and records a forge-fit check in the manifest. Install it with `./install.sh --stage motion --with-models`. The prompt encoder is Meta Llama 3 8B, a gated download covered by the Llama 3 Community License: accept the licence on its Hugging Face page and run `hf auth login` first. Since 2026-11-20 it runs on real prompts, on MPS by default, in about 40 s per clip plus the wait for a heavy slot. Seven test prompts were checked. Whole-body motion works: a jump, a backward fall, a slow walk, an idle and, partly, a sword swing. Actions where a hand must reach a prop do not: in the sword pull and the bow draw the hands miss the grip or quiver by 30-50 cm. [Motion guide](docs/motion.md)
