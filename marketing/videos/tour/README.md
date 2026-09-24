# Otto product tour film

One narrated film (~3 min, 1920×1080, 30 fps, H.264 + AAC) that walks through
every main area of the app, chaptered by sidebar section. The Help →
Walkthroughs page plays it from `ui/src/lib/walkthroughs/film.json`.

Everything here is self-contained: its own `package.json`, footage captured from
the **current** UI, narration from macOS `say`, and a soundtrack synthesized
from code. Nothing is downloaded or licensed.

```
script/chapters.json     narration text + chapter → sidebar section map (edit this first)
scripts/capture.mjs      isolated daemon + demo DBs + seed + Playwright → public/capture/
scripts/lib/seed.mjs     the fictional "Acme Storefront" demo data (API calls only)
scripts/lib/shots.mjs    the shot list (routes, clicks, stills, flow clips)
scripts/voice.mjs        say → ffmpeg, per-sentence timing → src/generated/timing.json + out/otto-tour.vtt
scripts/soundtrack.mjs   synthesized music bed + UI sound accents → public/audio/
scripts/render.mjs       Remotion render → loudnorm → poster → film.json
src/                     the Remotion compositions (Tour, Poster) and scene specs (src/scenes.ts)
```

## Prerequisites

- macOS with `say` and ffmpeg (`/opt/homebrew/bin/ffmpeg`, or set `FFMPEG`).
- A daemon binary built from this tree: `cargo build -p ottod` (or point
  `OTTO_E2E_BIN` at an existing one).
- The production UI build: `cd ui && npm run build` (→ `ui/dist`).
- Docker (optional) for the demo databases. The capture uses local images
  `mariadb:11.8.4`, `mongo:8.2` and `redpandadata/redpanda:v24.2.7`. It skips
  any image that isn't present, and the chapters that need it look emptier.
- `npm install` in this folder. This installs Remotion and Playwright 1.61,
  which reuses the Chromium in `~/Library/Caches/ms-playwright`.

## 1. Capture footage

```bash
OTTO_E2E_BIN=/path/to/ottod OTTO_E2E_PORT=7811 OTTO_E2E_PW_PORT=5211 \
  nice -n 10 node scripts/capture.mjs
```

What the capture sets up:

- **Isolated daemon.** It spawns its own `ottod` on a temp data dir with a
  **fake `HOME`**. Agent CLIs (`claude`, `codex`, …) are replaced on `PATH` by a
  scripted stand-in (`scripts/lib/agent-sim.sh`). Secrets are file-backed and
  there is no Keychain access. It never touches the real daemon on `:7700`,
  `~/Library/Application Support/Otto`, `~/.claude`, `~/.codex` or `~/.hermes`.
- **Demo databases.** Throwaway MariaDB, MongoDB and Redpanda containers with
  fictional data (`scripts/demo-db/`).
- **Seed data.** Everything is seeded through the daemon API:
  - workspace and git repos, agent sessions
  - Assistant threads, memory and tasks
  - DB connections and dashboards, SSH hosts, Kafka topics
  - placeholder AWS and Kubernetes registrations
  - API requests, an MCP server, a swarm, a goal loop, workflows, scheduled
    tasks, personal agents, Run with Otto runs
  - an AI review and findings, a proof pack, a product story, a Vault bundle,
    Design Hall artifacts, a skill review, and channel settings (disabled)
  - fake transcripts for History and Usage, and insights reports
- **Driving the UI.** Playwright drives the production build at 1600×900 @2x
  with one worker:
  - stills are `public/capture/*.jpg`
  - flow clips (⌘K, agents, git, query builder) are recorded through the
    DevTools screencast as `*.mp4`

When it finishes, the daemon, the static server, Chromium and the containers
are all stopped.

To iterate on one shot without reseeding each time:

```bash
node scripts/capture.mjs --serve          # bring up + seed, keep running
node scripts/capture.mjs --attach --only db-builder,git
node scripts/contact.mjs public/capture   # 4-up contact sheets in .cache/cs/ for review
```

## 2. Narration and captions

```bash
node scripts/voice.mjs            # VOICE=Samantha RATE=182 by default
```

Each sentence of `script/chapters.json` is synthesized separately, trimmed, and
joined with a short breath. Each chapter is then EQ'd, compressed and
normalized to −16 LUFS.

Outputs:

- `public/audio/vo-<chapter>.wav`
- `src/generated/timing.json`, the edit timeline (commit this)
- `out/otto-tour.vtt`, WebVTT captions from the real sentence timings

Chapter length is the narration plus a short lead-in and tail, so rewriting a
line re-times the whole film automatically.

## 3. Soundtrack

```bash
node scripts/soundtrack.mjs       # sized to timing.json
```

This produces an original ambient pad (Dmaj9 – Bm9 – Gmaj7 – A6sus at 96 BPM)
plus a soft plucked pulse, with a riser on the intro and a swell and ring-out
on the outro. It is normalized to −20 LUFS. It also writes the UI accents
(`sfx-click|whoosh|tick|riser|impact.wav`).

The composition ducks the music about 20 dB under the voice (`src/Tour.tsx`)
and opens it up in the gaps.

## 4. Render

```bash
nice -n 10 node scripts/render.mjs              # concurrency 4, CRF 23
node scripts/render.mjs --stills 300,1200,2400  # preview frames → .cache/frames/
npx remotion studio src/index.ts                # interactive preview
```

The render produces:

- `out/otto-tour.mp4`, with a two-pass loudnorm to −16 LUFS integrated and
  −1.5 dBTP, and the video stream copied
- `out/otto-tour-poster.jpg`
- `ui/src/lib/walkthroughs/film.json`, the manifest the app reads (chapters
  with `section` ids)
- `ui/src/lib/walkthroughs/otto-tour.vtt`, a copy of the captions. It is
  committed because release assets are served without CORS headers.

## 5. Publish

The mp4 and poster are **not** committed; `out/`, `public/capture/` and
`public/audio/` are git-ignored. They are published as assets of the
`walkthroughs` GitHub release, and only after the owner approves:

```bash
gh release upload walkthroughs out/otto-tour.mp4 out/otto-tour-poster.jpg out/otto-tour.vtt --clobber
```

## Editing the film

- **Words.** Edit `script/chapters.json`, then run `voice.mjs`,
  `soundtrack.mjs` and `render.mjs`.
- **What's on screen.** Edit `src/scenes.ts`. Each chapter is a list of shots:
  - a `screen` shot has camera keys (`{t, x, y, z}` in normalized footage
    coordinates), callouts and chips
  - a `montage` shot is a quick beat of several stills
  - a `custom` shot is used for the desktop-app and phone beats
- **After a UI change.** Re-run the capture, then render.
