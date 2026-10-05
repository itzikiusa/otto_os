# Otto product tour film

One music-led film (2:20, 1920×1080, 30 fps, H.264 + AAC) that walks through
every main area of the app, chaptered by sidebar section. The Help →
Walkthroughs page plays it from `ui/src/lib/walkthroughs/film.json`.

The edit is cut to its own score. The score runs at 120 BPM, so a beat is
exactly 15 frames and a bar is 60. Every chapter starts on a bar line, and every
cut, camera move and callout lands on a beat. Chapter starts get a fill and a
crash in the music. Section starts get a riser and an impact, and the visuals
answer with a soft light ring and a kinetic title.

Everything here is self-contained: its own `package.json`, footage captured from
the **current** UI, an original score synthesized from code, and short captions.
No samples or commercial music are used.

```
script/chapters.json      the edit plan: chapters → Help sections, length in bars, music sections, captions (edit this first)
scripts/timing.mjs        chapters.json → src/generated/timing.json + otto-tour-<edition>.vtt (bar grid)
scripts/soundtrack.mjs    the score + UI accents, arranged from timing.json → public/audio/
scripts/capture.mjs       isolated daemon + demo DBs + seed + Playwright → public/capture/
scripts/capture-rooms.mjs genuine two-person Rooms footage → public/capture/rooms/
scripts/lib/shots.mjs     the shot list (routes, clicks, stills, flow clips)
scripts/render.mjs        Remotion render → loudnorm → poster → out/film.candidate.json
scripts/validate.mjs      checks the candidate (media, chapters, captions, loudness, Rooms provenance)
scripts/activate.mjs      points the app at the candidate (only after the assets are published)
src/                      the Remotion compositions (Tour, Poster) and the shot specs (src/scenes.ts)
```

## The score

`node scripts/soundtrack.mjs` writes `public/audio/music.wav`. The score is
electronic pop in D major at 120 BPM. It uses an uplifting vi–IV–I–V loop (Bm G D A)
and is arranged from the edit's music sections in `chapters.json`:

| Section | Chapters | What plays |
|---|---|---|
| Intro build | Meet Otto | The filter opens on a supersaw pad, the pluck arp comes in, then the kick from bar 3, a snare roll, a riser and half a beat of silence |
| Drop / chorus | Home → Run with Otto | Four-on-the-floor kick, clap with a short room, swung 16th hats with open off-beats, pumping 8th bass, side-chained pad, arp and the lead hook. The last 4 bars are a post-chorus. |
| Verse | Swarm, Workflows | Lighter hats, a syncopated bass, a darker pad and a prominent arp, with the hook's figure answered on the pluck |
| Chorus | Git → Design Hall | Riser and impact in, then the hook again |
| Verse | Database, Connections | As the first verse |
| Breakdown | Insights | Drums out, a resonant pad sweep, the hook on the pluck, then a 2-bar riser and snare roll |
| Final chorus | Rooms | Full energy, with the hook harmonised a third below |
| Outro | Everywhere | vi–IV–V resolves to I under the logo lockup, followed by a tonic hit and the hook's tag. The track then rings out. |

Sound design (all in code):

- polyBLEP saws, which suppress aliasing
- a 7-voice supersaw pad through a resonant state-variable filter
- a side-chain pump from the kick
- a filtered-saw pluck arp with a dotted-8th ping-pong delay
- a detuned saw lead whose 3+3+2 hook recurs in every chorus
- a layered kick (pitched body, click and saturation, so it reads on laptop speakers) and 808-style metallic hats
- a Freeverb hall on a high-passed send

The master high-passes at 34 Hz. No fundamental sits below 43 Hz. The mix keeps
its weight in the mids, where the hook, arp and bass harmonics sit. A two-pass
**linear** loudnorm to −16 LUFS keeps the section contrast: the intro builds
from about −25 to −20 dB RMS, the choruses sit near −17, the verses near −19 and
the breakdown near −24. The UI accents (`sfx-tick` and the rest) are tuned to
the key.

To check the score objectively without listening:

```bash
ffmpeg -i public/audio/music.wav -lavfi showspectrumpic=s=1400x600:scale=log:fscale=log:legend=1 /tmp/spec.png
ffmpeg -hide_banner -nostats -i public/audio/music.wav -af ebur128=peak=true -f null - 2>&1 | tail -12
```

## The picture

- **Opening (4 bars).** "Your agents. Your repos. Your data. Your
  infrastructure." lands one word every two beats, each with its screen. On
  bar 3 the screens rush into a wall and "One home." appears. On bar 4 the logo
  slams in. The camera then zooms through the logo onto the drop.
- **Chapters.** On the downbeat, a kinetic title (the group plus the words
  rising from masks) holds for about 1.5 beats. It then docks into a header
  band above the window. Its second line follows the shot on screen. A
  progress rail, top right, fills chapter by chapter in the sidebar group
  colours.
- **Shots** are 2–8 beats long. Cuts rotate through whip-pan (with motion
  blur), push and diagonal wipe. Chapter changes zoom through to the next
  window. The camera springs to each key: zoom to a callout, then a slow drift.
  Callouts pulse with the beat. Highlight rings glow around a region.
- **Backdrop.** Drifting light fields in the chapter's group colours, taken
  from `tokens.css` (`--accent` and `--cat-*`). It has a soft ring on each
  chapter downbeat and a gentle on-beat lift in the choruses. There is no
  flashing.
- **Montages.** Design Hall's studios and Connections are quick cuts, one
  every two beats, each with a glass name card.
- **Rooms** is the final chorus. It uses five genuine room clips: start and
  invite, live screen share, annotation, terminal hand-over, and the recap
  draft.
- **Outro.** Snip, Slack & Telegram and the phone view land on successive beats.
  The logo, the line and "Download Otto for macOS" arrive on the tonic hit.

## Prerequisites

- ffmpeg on PATH (or set `FFMPEG`).
- A daemon binary built from this tree: `cargo build -p ottod` (or point
  `OTTO_E2E_BIN` at an existing one).
- The production UI build: `cd ui && npm run build` (→ `ui/dist`).
- Docker (optional) for the demo databases. The capture uses local images
  `mariadb:11.8.4`, `mongo:8.2` and `redpandadata/redpanda:v24.2.7`. It skips
  any image that isn't present, and the chapters that need it look emptier.
- `npm install` in this folder. This installs Remotion, which downloads its own
  headless Chrome on first render. It also installs Playwright 1.63, which reuses
  the Chromium in `~/Library/Caches/ms-playwright`.
- **Rooms only.** A local whisper.cpp binary and multilingual model, macOS `say`,
  the UI's installed Playwright, and `CODEX_HOME` pointing to a signed-in Codex
  subscription. Alternatively, reuse an existing genuine capture (see below).

## Producing the film: one heavy sequence

Run these one at a time; each is CPU-bound. `nice` keeps the Mac responsive.

```bash
cd marketing/videos/tour

# 1. Footage from the current UI (isolated daemon on its own port; never touches :7700).
OTTO_E2E_BIN=/path/to/ottod OTTO_E2E_PORT=7811 OTTO_E2E_PW_PORT=5211 \
  nice -n 10 node scripts/capture.mjs
node scripts/contact.mjs public/capture            # optional: contact sheets in .cache/cs/ to eyeball

# 2. Rooms footage: re-capture, OR reuse a previous genuine capture verbatim.
OTTO_E2E_BIN=/path/to/ottod OTTO_E2E_PORT=7831 OTTO_E2E_PW_PORT=5231 \
OTTO_TOUR_WHISPER=/path/to/whisper-cli OTTO_TOUR_WHISPER_MODEL=/path/to/ggml-small.bin \
  nice -n 10 node scripts/capture-rooms.mjs
#   (or: cp -R <checkout>/marketing/videos/tour/public/capture/rooms public/capture/)

# 3. Timeline and score (seconds, light).
node scripts/timing.mjs
nice -n 10 node scripts/soundtrack.mjs

# 4. Check a few frames, then render (concurrency ≤ 4).
nice -n 10 node scripts/render.mjs --stills 120,262,900,3500,4060
nice -n 10 node scripts/render.mjs --concurrency 3

# 5. Verify the candidate.
node scripts/validate.mjs
```

`render.mjs` writes these to `out/`:

- `otto-tour-<edition>.mp4`, with a two-pass loudnorm to −16 LUFS and −1.5 dBTP
  and the video stream copied
- `otto-tour-<edition>-poster.jpg`
- `otto-tour-<edition>.vtt`
- `film.candidate.json`

The active app manifest is **not** touched.

To iterate on one capture without reseeding each time:

```bash
node scripts/capture.mjs --serve          # bring up + seed, keep running
node scripts/capture.mjs --attach --only db-builder,browser
```

`npx remotion studio src/index.ts` previews the edit interactively.

## Publish (after the owner approves)

`out/`, `public/capture/` and `public/audio/` are git-ignored. The mp4, poster
and captions are published as assets of the `walkthroughs` GitHub release:

```bash
gh release upload walkthroughs out/otto-tour-<edition>.mp4 out/otto-tour-<edition>-poster.jpg out/otto-tour-<edition>.vtt
node scripts/activate.mjs   # → ui/src/lib/walkthroughs/film.json + the bundled .vtt; commit both
```

Captions are bundled next to `film.json` because release assets are served
without CORS headers. `TourFilm` resolves the VTT by the manifest's `captions`
basename.

## Editing the film

- **Timing.** Change a chapter's `bars` in `script/chapters.json`, then run
  `timing.mjs`. Next, make that chapter's shots in `src/scenes.ts` add up to
  `bars × 4` beats; `layout()` refuses a mismatch. Finally re-run
  `soundtrack.mjs`, which re-arranges itself to the new chapter starts.
- **Music sections.** The `music` key on a chapter starts a section there:
  `intro`, `chorus`, `verse`, `breakdown`, `final` or `outro`. Keep sections a
  multiple of 4 bars so the chord loop lands.
- **Captions.** These are the `captions` lines in `chapters.json`. They are
  split evenly across the chapter, or by `captionBeats`.
- **What's on screen.** Edit `src/scenes.ts`. Each shot has a length in
  `beats` and may have any of these:
  - camera keys `{b, x, y, z}`, in beats and normalized footage coordinates
  - `callouts` and `highlights` from beat `b` to `until`
  - a header `label`
  - a montage `card`
  - an optional `transition` (`whip`, `push` or `wipe`)
- **After a UI change.** Re-run the capture, then render.

## Rooms footage: what is genuine

The Rooms capture drives two independent browser participants through the
production UI and an isolated daemon. No room endpoints, WebSocket events,
transcripts or summary responses are mocked.

- **Synthetic media inputs.** The fictional Acme screen content is live canvas
  video, carried by the real WebRTC pipeline. Locally synthesized speech is the
  microphone input, recognized by real local whisper.cpp.
- **Real execution.** A real shell runs the demonstrated regression test.
- **Real summary.** The recap draft comes from the actual signed-in Codex
  subscription, after the UI confirmation, with the production read-only,
  tool-disabled invocation.
- **Credentials.** None are copied into artifacts or printed.

`validate.mjs` requires the capture's `recap-evidence.json`, with a ready draft,
and `provenance.json`, with no runtime errors. `script/rooms-validation.md`
records the 2026-09-28 capture.

**Limits shown honestly.**

- This is browser room footage, not native macOS window chrome.
- Takeover controls the shared session terminal, not arbitrary OS apps.
- The product does not imply webcam video.
- Screen recap images are periodic samples, not continuous video.
