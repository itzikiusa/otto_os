# Forge Audio

Generate sound effects, ambience beds and background music from a text brief, as 48 kHz WAV files for a game. MOSS-SoundEffect v2.0 (Apache-2.0) makes effects and ambience on MLX. ACE-Step 1.5 (MIT) makes music, with MLX for the diffusion model and the decoder and PyTorch for the text encoder. Both run on Apple Silicon without an NVIDIA GPU, an account or a token. [Licences](#licences)

## Generate

Run the commands from the Forge root. Each model run takes a slot from `tools/heavy.sh`.

```sh
tools/heavy.sh bin/forge-sfx --prompt 'two steel swords clashing, sharp metallic ring' \
  --seconds 2 --variations 3 --output out/sword.wav
tools/heavy.sh bin/forge-sfx --prompt 'forest at night, crickets, an owl, light wind in leaves' \
  --seconds 20 --loop --output out/forest.wav
tools/heavy.sh bin/forge-music --prompt 'cheerful medieval tavern folk, lute, fiddle and hand drum' \
  --seconds 32 --loop --bpm 110 --output out/tavern.wav
tools/heavy.sh bin/forge-audio scene --cue-sheet forge/audio/cues/castle-gate-night.json \
  --out out/castle-gate-night
```

| Option | Meaning |
| --- | --- |
| `--prompt` | The sound, or the music brief (mood, tempo, instruments, genre). At most 512 characters. English works best. MOSS was also trained on Chinese captions. |
| `--seconds` | Length of the file. Effects: 0.5–30 s, loops up to 26.5 s. Music: 10–594 s; music loops need margin, so they are shorter. |
| `--loop` | Make the file loop seamlessly (see [Loops](#loops)). With `forge-sfx` it also marks the file as an ambience bed for loudness. |
| `--seed` | Default 42. With the same seed, prompt and length, the output repeats on this build. In the tests the repeats were bit-identical. |
| `--variations K` | Effects only, 1–16. Writes `out/sword.wav`, `out/sword-2.wav` … with seeds `seed`, `seed+1` … |
| `--bpm` | Music tempo, 30–300, passed to ACE-Step. A loop is cut to whole beats at this tempo. |
| `--instrumental` / `--lyrics TEXT` | Instrumental is the default. `--lyrics` sings the given text instead. |

Use a fresh output path for each take. Each WAV gets an adjacent `.manifest.json`. Progress is one JSON object per line on stderr, for example `{"stage":"sfx","pct":45,"message":"Generating sword.wav"}`. A failure ends with `{"error":"..."}` and a nonzero exit, and cancellation ends with `{"error":"Cancelled"}` and exit 130. Generation runs inside the stage process, which starts no children. SIGTERM or Control-C stops it between model steps. Scratch files go to `out/work/audio-*`, and the stage removes them whether it succeeds, fails or is cancelled. `FORGE_KEEP_WORK=1` keeps them, with the model logs in `stage.log`.

## What comes out

| Kind | Channels | Level |
| --- | --- | --- |
| One-shot effect (`forge-sfx` without `--loop`, scene `spots`) | mono | Peak-normalised to −1 dBTP. A 2 ms fade-in and a 20 ms fade-out remove edge clicks. |
| Ambience bed (`forge-sfx --loop`, scene `ambience`) | mono | −18 LUFS integrated, ceiling −1 dBTP |
| Music | stereo | −16 LUFS integrated, ceiling −1 dBTP |

Files are 48 kHz, 16-bit PCM WAV, the native rate of both models. Loudness is ITU-R BS.1770-4 integrated loudness, measured with pyloudnorm. True peak is the 4× oversampled peak. The stage changes level by gain only, with no compressor or limiter. A bed whose peaks would pass −1 dBTP at its target stays quieter, and its manifest shows `loudness.target_met: false`. One-shots are matched on peak, so their integrated loudness varies with the sound: in the castle scene a torch flare measured −12.6 LUFS and gravel steps −27.7 LUFS. Set per-event volume in the game's mixer. Every file is high-passed at 20 Hz with a zero-phase filter. In one MOSS forest bed, 82% of the signal energy was below 20 Hz, sub-audio drift from the decoder that used up headroom.

The manifest records the tool, model id, revision and licence, runtime versions and licences, the prompt, seed, requested, generated and final durations, sample rate, the loudness result, loop and seam metrics, objective measurements, timings and peak memory.

## Loops

The model generates more audio than requested. The stage finds the steadiest stretch, the window whose quietest 10% of 100 ms blocks is loudest, which skips intros, fade-ins and endings. The audio after the loop end is crossfaded into the start of the loop. The last and first samples of the file are therefore consecutive samples of the generated audio, so the wrap point is continuous by construction, and the crossfade hides the change of content. Beds use a 2 s equal-power crossfade, or a quarter of the loop if that is shorter. Music uses a one-bar linear crossfade. Within half a beat of the requested length, the stage picks the loop length whose crossfade lines up best on the onset envelope, so a music loop can differ from `--seconds` by up to half a beat.

Each loop's manifest has `loop.seam` and `loop.seam_ok`. Each ratio compares the wrap point with the file's own variation:

| Metric | Seam value ÷ | A click or jump looks like |
| --- | --- | --- |
| `jump_ratio` | 99.9th percentile of \|x[n+1] − x[n]\| in the file | a step far outside the content's range |
| `slope_ratio` | 99.9th percentile of the second difference | a kink in the waveform |
| `spectral_ratio` | 99th percentile of the mean band-level change (24 log bands, dB) between adjacent 250 ms windows | a change of texture, level or tone larger than the content's own |

A seam passes when every ratio is at most 1.0. A seam that behaves like any other point in the file then fails rarely, by the percentile chosen: about 0.1% for each sample metric and 1% for the spectral one. Two controls check that the metric can fail. Without a crossfade, rejoining a loop at an arbitrary point scored jump or slope ratios of 1.5–12 in five of seven loops (music, wind, cave drips). Wrapping a non-loop file end to start scored a spectral ratio of 1.6–2.6 for music and for effects that decay (sword, wolf howl). The test suite asserts that the music case fails. The controls can miss. A naive cut through dense crickets and night insects stayed between 0.4 and 1.1 on every metric, because noise has large sample steps of its own. Effects that start and end in similar noise, such as footsteps (0.28), pass even when they are not loops. The metric cannot hear. Listen to loops before you ship them.

## Lengths and limits

- **Effects** are at most 30 s per call, MOSS-SoundEffect's trained window. The model always denoises a full 30 s latent and crops it, so a 2 s effect costs as much as a 30 s bed. A loop needs 3.5 s of margin, so loops are at most 26.5 s. For a longer bed, layer several loops, or add spot effects at random intervals in the game.
- **Music**: ACE-Step accepts 10–600 s in one pass. A 590 s request was generated in one pass in 80 s here. ACE-Step ends the piece early and pads the rest with silence: 2.5 s of a 12 s request, 5–6 s of 25–40 s, 2–3.5 s of 29 s, 240 s and 590 s requests. The stage asks for 6 s more, trims the silence, and cuts to the exact length with a 2 s fade-out. If the piece ends before the requested length, it lets the ending ring out into silence. A loop asks for 8 s + 15% + one bar and beat more, so the longest music loop is about 500 s. The manifest's `content_sec` records how much music the model actually wrote.
- **Prompt adherence** was measured, not judged by ear. Different prompts gave band-spectrum profiles 10.5–16.9 dB apart in mean level, against a 3 dB threshold. Two seeds of one prompt were 13.5 dB apart, and the same seed 0 dB. Onset autocorrelation found 70.0 BPM for a 70 BPM request and 110.25 for 110. A 90 BPM request came out at 177.5, and an 80 BPM request at 159.75: double the tempo, which counts as a match. For a 60 BPM ambient drone the estimate, 216 BPM, had no simple relation to the request. The estimator needs clear beats. Forge uses the DiT-only ACE-Step path. The optional 5 Hz planner LM, which writes song structure and refines captions, would add 3.7 GB beyond this stage's download budget.
- **Content** is what the models make. No one listened to these files during development. Vocals, especially, were not tested.

## Scene cue sheets

Otto's agent writes the cue sheet from a scene description. `forge-audio scene` renders every entry and writes `DIR/manifest.json`, which lists each file with its kind, prompt, seed, duration, loudness, true peak, seam result, model and licence.

```json
{"scene": "free text, ignored",
 "seed": 101,
 "ambience": [{"id": "gate-wind", "prompt": "night wind through a ruined stone wall", "seconds": 20, "loop": true}],
 "spots":    [{"id": "gravel-steps", "prompt": "boots on loose gravel, four slow steps", "seconds": 3, "variations": 3}],
 "music":    [{"id": "gate-theme", "prompt": "dark fantasy ambient underscore, low cello drone", "seconds": 32, "loop": true, "bpm": 60}]}
```

Ids are letters, digits, `_` and `-`, unique within the sheet, and become file names (`gravel-steps.wav`, `gravel-steps-2.wav` …). Ambience loops by default. Spots and music loop when `"loop": true`. Each cue takes `"seed"` if it has one, otherwise the sheet seed plus the cue's position. The stage renders all effects first, then releases MOSS before it loads ACE-Step. Two examples ship in [forge/audio/cues/](../forge/audio/cues/): a ruined castle gate at night and a busy market at noon.

## Install and verify

```sh
./install.sh --stage audio --with-models
bin/forge-doctor
tools/heavy.sh forge/audio/.venv/bin/python tests/audio/test_audio.py
```

The installer clones ACE-Step 1.5 at `ca1e85f` into `engines/ace-step` and builds `forge/audio/.venv` from ACE-Step's frozen `uv.lock` (CPython 3.11, 1.2 GB). It adds the MOSS MLX port at `4e8e3e3` and pyloudnorm. With `--with-models`, `forge/audio/fetch_models.py` prints the sizes, then downloads the pinned public revisions into `models/audio`: MOSS-SoundEffect 4-bit at 6.40 GB and the ACE-Step turbo DiT, VAE and text encoder at 6.34 GB, 12.74 GB in all. Downloads resume after an interruption, are checked against the Hub's size and SHA-256, and stop when less than 60 GiB is free. Run the command again after a failure. The launchers keep Hugging Face offline, with its cache under `models/audio`.

The suite generates real files into `out/audio-tests/`, about 11 model runs. It checks duration, signal level, clipping, the loudness targets, loop seams with a positive control, that different prompts differ, that the same seed reproduces, that variations differ, the tempo against `--bpm`, manifests, a failing call and a cancellation that leaves no process or file. It then writes spectrograms and a listening page to `out/audio-evidence/index.html`. To add other folders to the page, run `forge/audio/.venv/bin/python tests/audio/evidence.py out/audio-tests out/castle-gate-night`.

## Timings on this Mac

MacBook Pro M5 Pro, 48 GB, measured with other agents' jobs running (load average 10–20):

| Job | Model load | Generation | Whole command | Peak RSS | Peak MLX memory |
| --- | --- | --- | --- | --- | --- |
| One effect or bed, 1.5–20 s (16 runs) | 2–3 s, then lazy | 82–100 s | 90–93 s | 6.0 GB | 11.6 GB |
| Music 12–20 s (4 runs) | 9–24 s | 7–9 s | 18–24 s | 6.6–14.2 GB | 13.6–14.7 GB |
| Music loop 32 s at 60 BPM | 28.5 s | 21 s | n/a, in a scene | 7.1 GB | 17.3 GB |
| Music 240 s | 9 s | 57 s | not timed | 14.2 GB | 18.1 GB |
| Music 590 s | 12 s | 80 s | not timed | 14.2 GB | 18.4 GB |
| Castle-gate scene: 2 beds, 7 effects, 1 music loop | | | 857 s | | 17.3 GB |

MLX loads MOSS weights lazily, so the load column understates them and the first generation absorbs the cost. Every MOSS call costs about the same because it always denoises a full 30 s latent, 100 steps with classifier-free guidance. The model card reports 45 s on an M5 Max. RSS undercounts Metal allocations, and the scene process reported only 7.1 GB with ACE-Step loaded. Peak MLX memory is the better guide. Leave about 20 GB free for music.

## Licences

Both models' terms allow commercial use and place no conditions on the generated audio. [NOTICE.md](../NOTICE.md#sound-effects-ambience-and-music)

- **MOSS-SoundEffect v2.0**: weights under Apache-2.0, like the MLX 4-bit conversion, its MLX port and the Qwen3-1.7B text encoder. The model card does not say what the model was trained on.
- **ACE-Step 1.5**: code and weights under MIT. The card says "You can strictly use the generated music for **commercial purposes**" and describes licensed, royalty-free and synthetic training data. These are the developers' statements.
- **Not used**: Stable Audio 3.0 Small-SFX and Medium are gated (HTTP 401 without an account), and the Stability Community License has a USD 1M revenue cap. According to the research notes, TangoFlux, AudioGen, MusicGen, MMAudio, AudioX, ThinkSound and YuE2 are non-commercial.
