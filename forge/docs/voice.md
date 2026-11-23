# Forge Voice

Generate an English game line as a mono PCM WAV, with optional mouth cues. Kokoro provides fast stock voices and equal-weight blends. Qwen3-TTS VoiceDesign builds an original speaking voice from an English description of its register, texture and delivery.

## Speak

From the Forge root on Apple Silicon, run:

```sh
tools/heavy.sh bin/forge-voice speak --voice keeper --speed 1.0 \
  --text 'Take the lantern and follow me across the bridge.' \
  --format wav --output out/keeper.wav --visemes

tools/heavy.sh bin/forge-voice speak --engine qwen --voice designed \
  --description 'A mature woman with a low, husky voice. She speaks softly, with quiet urgency.' \
  --text 'There is still time. Stay with me.' --output out/ember-line.wav
```

Use a fresh output path for each take. The first command creates `keeper.wav`, `keeper.wav.manifest.json` and `keeper.wav.visemes.json` together. Progress is JSONL on stderr, with `stage`, `pct` and `message`. A failed command exits nonzero and emits a final `{"error":"..."}`. Cancel using SIGTERM or Control-C.

| Character | Gender | Engine | Default delivery |
|---|---|---|---|
| `keeper` | male | Kokoro | Low Onyx/Michael blend at 0.92×. With Qwen: an older man, deep, slow and slightly tired |
| `scout` | female | Kokoro | Light Heart voice at 1.06×. With Qwen: a young woman, quick, bright and cheeky |
| `ember` | female | Qwen | Low, husky female voice with quiet urgency |
| `flint` | male | Qwen | Warm, gravelly male storyteller at 0.94× |

Both commands and HTTP use [characters.json](../forge/voice/characters.json). A request speed from 0.5 to 2.0 multiplies the preset speed. For raw Kokoro voices, supply one of the 28 English IDs, such as `af_heart` or `bm_george`, or a comma-separated blend such as `af_alloy,af_nicole`. The first ID selects US or UK pronunciation. For Qwen, select a character (`--engine qwen --voice keeper` designs Keeper from his description) or use `--voice designed --description '...'`. VoiceDesign takes text only. It uses a fixed sampling seed for repeatable takes on this build. Different lines can still vary in perceived speaker identity. Qwen pace adjustment uses FFmpeg's pitch-preserving tempo filter.

## Mouth cues

`--visemes` aligns the final waveform after speed adjustment. PocketSphinx 5.1.1 aligns the known English transcript to words, then aligns phones within each word. It uses the bundled US English acoustic model and CMU dictionary. For names outside the dictionary the English phonemizer supplies a pronunciation. Timing has 10 ms resolution, with the final cue extended or padded to the exact PCM duration. The Intel-only upstream Rhubarb 1.14 macOS binary failed with `Bad CPU type in executable` on this Mac. PocketSphinx installs from a native wheel without sudo or Rosetta.

The sidecar has exactly two top-level fields. Times are seconds from the first PCM sample. Consecutive equal shapes are merged. For example:

```json
{"duration_sec":0.17,"cues":[{"start":0.0,"end":0.05,"value":"X"},{"start":0.05,"end":0.17,"value":"A"}]}
```

| Shape | Target for the face rig |
|---|---|
| A | Lips closed for M, B, P |
| B | Narrow consonant opening |
| C | Medium vowel opening |
| D | Wide open vowel |
| E | Partly rounded mouth |
| F | Pursed lips |
| G | Lower lip touches upper teeth for F, V |
| H | Tongue raised for L |
| X | Rest |

Map the letter to a character's blend shape. The file supplies mouth timing, while expression, intensity and transitions remain under the animation system's control.

## Otto speech provider

```sh
tools/heavy.sh bin/forge-voice serve --port 7811
```

Set the Otto provider kind to `kokoro` (or `openai_compat`), endpoint to `http://127.0.0.1:7811/`, and enable **Allow local network**. Leave the API key empty. Set **Default model** to `qwen` to have every character designed by Qwen3-TTS VoiceDesign from its description, or leave it empty (`kokoro`) for the fast stock Kokoro voices. Until a provider is enabled, Otto's productions speak with the macOS voices. The server binds IPv4 loopback only, loads engines on first use and serializes synthesis. Keep it running for warm dialogue generation. The foreground command holds one heavy slot until stopped.

| Method and route | Contract |
|---|---|
| GET `/health` | HTTP 200 with `{"ok":true}`, as used by Otto's connection test |
| GET `/v1/audio/voices` | JSON `{"voices":[{"id":"keeper","name":"Keeper"}, ...]}`, with the installed English voices and characters |
| POST `/v1/audio/speech` | Returns encoded audio. Send `X-Otto-Visemes: 1` to request lip-sync. Successful alignment adds `X-Otto-Visemes-Id` to the response headers. |
| GET `/v1/audio/visemes/<id>` | Returns the Rhubarb-shaped timeline as JSON. An unknown or evicted ID returns HTTP 404. |

To fetch audio and its mouth cues using Otto’s Kokoro request format, run the following from the Forge root.

```sh
curl -fsS http://127.0.0.1:7811/v1/audio/speech \
  -H 'Content-Type: application/json' -H 'X-Otto-Visemes: 1' \
  -d '{"model":"kokoro","input":"Take the lantern.","voice":"keeper","response_format":"wav","speed":1.0}' \
  -D out/keeper-http.headers -o out/keeper-http.wav
id=$(awk 'tolower($1) == "x-otto-visemes-id:" {sub(/\r$/, "", $2); print $2}' out/keeper-http.headers)
if [ -n "$id" ]; then
  curl -fsS "http://127.0.0.1:7811/v1/audio/visemes/$id" -o out/keeper-http.visemes.json
fi
```

`kokoro`, `tts-1` and `tts-1-hd` select the default preset lookup. Thus an Otto kokoro provider can select `ember` and automatically use Qwen. `qwen` designs every listed character from its description; a stock Kokoro ID such as `af_heart` stays Kokoro. `instructions` (the OpenAI field) designs the named voice from that text with any model, for example `"voice":"fox", "instructions":"Little red fox. Quick, bright and cheeky."`. The voice name is only a label: the description and the fixed seed decide the voice, so the same description and line return the same take.

Otto picks a voice per character from `GET /v1/audio/voices` by gender words in the plan (he, deep, woman, …) and list order, and does not yet send the plan's voice description. A deep male character therefore gets `keeper` and the next character `scout`. A voice that fits each description needs Otto to send it as `instructions`. Otto's driver sends `aac` when the user selects M4A, so that request returns an AAC stream in an MP4 container. WAV, MP3 and AIFF are also supported. Otto's client times out after 90 seconds and accepts at most 25 MiB of encoded audio. Use short lines for game dialogue.

Otto opts in for Kokoro-kind providers. Without the request header, the server skips PocketSphinx alignment. With it, PocketSphinx aligns the final speed-adjusted waveform. The GET response uses the same `duration_sec` and contiguous `cues` as the CLI sidecar above, with times measured from the start of the PCM audio. If alignment fails, the server still returns the requested audio and omits the ID. Fetch and save the timeline with the clip. The server holds the 32 most recently created timelines in memory, evicts the oldest on the next successful alignment, and clears them all on restart.

## Choosing the engine

On 2026-11-20 the owner's scene lines (a deep, tired robot keeper and a quick, cheeky fox) were made with Kokoro, Qwen3-TTS VoiceDesign 4-bit, VoxCPM2, MOSS-VoiceGenerator and the MOSS-TTSD dialogue model. The listening page is `out/voice-evidence/index.html` (local, not tracked). Measured on the M5 Pro: Qwen designs a voice at 0.36–0.47 s of compute per second of audio, keeps the same take for the same description and seed, gives the two characters clearly different voices and keeps mouth cues. It is the engine served by `"model":"qwen"`. VoxCPM2 made the two characters sound alike. MOSS-VoiceGenerator pads short lines with up to 2.9 s of silence. MOSS-TTSD speaks both lines in one pass but needs 24 GB and 20–34 s per second of audio. VoxCPM2 and the two MOSS models were removed after the test; re-run the commands in `out/voice-evidence/scripts/` to fetch them again. Nobody has judged naturalness by ear yet.

## Install and verify

Requires Apple Silicon, Python 3.11, the project's `tools/heavy.sh`, and FFmpeg on PATH for HTTP compression and Qwen tempo adjustment. The integration tests also use ffprobe. On this machine the runtime and both selected checkpoints have already been provisioned. From a fresh checkout:

```sh
tools/heavy.sh python3.11 -m venv forge/voice/.venv
tools/heavy.sh env PIP_CACHE_DIR="$PWD/models/voice/pip" \
  forge/voice/.venv/bin/python -m pip install -r forge/voice/requirements.txt
tools/heavy.sh forge/voice/.venv/bin/python forge/voice/fetch_models.py kokoro qwen
tools/heavy.sh forge/voice/.venv/bin/python tests/voice/test_voice.py
```

No account or token is required. Public checkpoint revisions are pinned in [fetch_models.py](../forge/voice/fetch_models.py). The launcher runs Hugging Face offline and sets its cache inside `models/voice`. The functional tests synthesize real files, verify signal level and duration, walk the viseme timeline, exercise all four driver formats, check designed voices over HTTP (listed genders, `"model":"qwen"`, repeatable `instructions` takes), and cancel a running transcode.

## Licences

Kokoro and Qwen3-TTS VoiceDesign publish Apache-2.0 model weights. MLX and mlx-audio use MIT. The Kokoro English frontend includes Apache-2.0 Misaki, an MIT spaCy pipeline, and a GPL phonemizer/eSpeak fallback. PocketSphinx and its acoustic model have commercial-compatible BSD terms. FFmpeg's licence depends on the installed binary. Baked game assets and redistribution of the voice runtime are distinct release decisions; retain the applicable notices when shipping software, model data or installer dependencies. The CLI manifest records the stages and checkpoint revision for each take. [Kokoro weights](https://huggingface.co/hexgrad/Kokoro-82M) · [Qwen model](https://huggingface.co/Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign) · [Qwen licence](https://github.com/QwenLM/Qwen3-TTS/blob/main/LICENSE) · [PocketSphinx licence](https://github.com/cmusphinx/pocketsphinx/blob/v5.1.1/LICENSE) · [Acoustic-model licence](https://github.com/cmusphinx/pocketsphinx/blob/v5.1.1/model/en-us/en-us/README)
