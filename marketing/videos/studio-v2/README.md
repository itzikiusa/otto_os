# Otto walkthrough films

Eight original Remotion films follow complete jobs across Otto. The scripts are
based on the current sidebar, feature guides, and the source files attached to
each chapter. The old video compositions, storyboards, audio, and renders were
not used as creative reference.

The films use **illustrated product demonstrations**, with fictional Atlas
workspace data. They teach the product's actions and relationships, but are not
screen recordings or pixel-exact reproductions of the shipped interface.

## The series

| Film | Chapters |
| --- | --- |
| Your workspace | Sessions, Mission Control, Appearance and command palette |
| From change to proof | Git, review findings, Proof Packs, Run with Otto |
| From idea to context | Product and mockups, Canvas, Browser, Vault |
| Work that keeps moving | Swarm, Goal Loops, Workflows, Scheduled Tasks, Personal Agents |
| Follow the data | Connections and SFTP, Database Explorer, Kafka, API client |
| Operate the cloud | AWS services, resource metrics, Kubernetes workloads, pod logs |
| Improve the system | MCP, Skills, Skills Lab, Usage and Insights, Plugins |
| Take Otto with you | Channels, remote access and session sharing, multi-window, Snipping Tool |

The shared catalog is `ui/src/lib/walkthroughs/catalog.json`. Its chapter times
come from measured voice recordings. `TRANSCRIPT.md` contains all spoken copy
and its feature/source references. The coverage check fails when a new built-in
sidebar route has no chapter.

## Art and audio

- 1920 × 1080, 30 fps. H.264 CRF 18, AAC 192 kbit/s stereo.
- Instrument Sans, bundled locally with its SIL Open Font License.
- Original vector aperture, typography, interface demonstrations and cursor motion.
- English synthetic narration: Microsoft Edge's Andrew Neural voice, generated
  using edge-tts 7.2.8. The checked-in MP3 takes are content-addressed. No API key,
  real account information, user content or production recording is involved.
- Original procedural score: 96 BPM, evolving four-chord pads, mallet motif,
  soft bass/percussion, stereo air transitions, and interaction tones. No stock
  music or sampled commercial audio. Speech automatically ducks the music.
- Final mix targets −16 LUFS, −1.5 dBTP, 48 kHz. Media verification measures the
  encoded AAC track to catch peak overshoot and missing/silent soundtracks.
- On-screen instructions are always visible during narration. Optional WebVTT
  captions contain the same words, timed to each recorded instruction.

## Build and preview

Run from `marketing/videos/`. Node dependencies are already declared in the
parent package. Python 3.9 with the pinned `studio-v2/requirements.txt` is the
reference audio environment; `ffmpeg` and `ffprobe` must be on PATH.

```bash
npm ci
python3 -m pip install -r studio-v2/requirements.txt
npm run check
npm run soundtrack       # creates the lossless mixes from versioned voice takes
npm run studio           # new Remotion entry, independent of the old compositions
npm run stills           # posters and one QA frame per chapter
npm run render-all       # all eight films, bundled once
npm run package:films    # captions, screening page, chaptered Complete.mp4
npm run verify:media     # full decode, stream properties, loudness, timing
npm run verify:frames    # encoded chapter frames match the current Remotion stills
npm run screening        # http://127.0.0.1:8780
npm run verify:player    # screening server must be running; requires ui npm ci
```

Render one or more films with `npm run render-all -- Cloud Data`. The renderer
fails on an unknown ID. `npm run studio:legacy` and `npm run render-all:legacy`
remain available for maintaining the already-published edition.

Only regenerate narration when the script changes:

```bash
python3 studio-v2/scripts/narration.py
npm run soundtrack
```

Narration generation uses the online Edge TTS service; a network/service failure
is reported rather than silently substituting another voice. Cached takes are
reused. It updates measured chapter starts/durations in the shared catalog;
re-render films, regenerate captions, and re-run checks after such a change.

`out-v2/` contains the films, combined master, posters, VTT files, screening page,
catalog, QA stills, and verification report. Generated deliverables and lossless
mixes are ignored by git. Sources, font/license and individual voice takes are
versioned. Rendering overwrites only the selected outputs in `out-v2/`.

## In-app preview and release

The existing published catalog remains the default until the new assets are
available. The player opts into this edition using a separate base URL:

```bash
# From ui/, with the screening server running:
VITE_WALKTHROUGHS_V2_BASE=http://127.0.0.1:8780 npm run dev
```

The new player includes per-film chapters, feature-guide links, searchable
chapter commands, posters, captions, and explicit playback of narrated films.
The override is compiled into the UI; it is not a runtime preference.

**Publishing is a separate action.** Upload the eight `.mp4`, matching `.jpg`
and `.vtt` files from `out-v2/` to a versioned release or media host, preserving
filenames. Serve media with byte-range support, `video/mp4` / `text/vtt` MIME
types and CORS headers. Verify URLs before building the app with
`VITE_WALKTHROUGHS_V2_BASE` pointing at that location. The old
`packaging/publish-walkthroughs.sh` forces 720p and only uploads MP4s, so it must
not be used to publish these masters. Nothing in the new production scripts
uploads assets, creates releases, or changes the installed app.

## Implementation references

- [Remotion audio](https://www.remotion.dev/docs/html5-audio)
- [Remotion server renderer](https://www.remotion.dev/docs/renderer/render-media)
- [edge-tts](https://github.com/rany2/edge-tts)

`Film.tsx` owns the editorial timeline, typography and branding. `ProductStage`
provides the demonstration shell; `stage/` groups the individual demonstrations.
Audio synthesis, rendering, packaging and validation each have their own script.
The browser check mounts the actual Svelte player in a temporary Vite harness;
it never starts or calls ottod.
