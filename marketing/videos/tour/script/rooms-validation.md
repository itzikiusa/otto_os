# Rooms walkthrough — 2026-09-28

Candidate: `out/otto-tour-rooms-20260928.mp4` (ignored generated output).
SHA-256: `febf5a63901f87a9129f0f2ad698d1a2389a477acd151a838617d973c37f21a1`.
Captions SHA-256: `5f7cbaceab525b382bcf3679a8ef8a3dfbc44e776c9a979704a3f79bd6c062bb`.
Poster SHA-256: `2c608278eff7b8c6961593da05938b4e7a18b183eec32da3934190013312c490`.

The 4:13.57 film retains the released 2026-09-25 tour, adds a 76.53-second
Rooms chapter at 2:45.20, and replaces the full music bed with the original
124 BPM drums/bass/chord composition. File size is 56.2 MiB.

## Verified

- Real production UI and isolated daemon, two separate browser participants.
  Admission, chat, consent, microphone input, WebRTC shared screens,
  annotation requests/grants, highlight/pen marks, second presenter/pin,
  terminal control request/grant, actual shell test execution, host takeover,
  archive reading and Codex draft generation all used production operations.
- Fictional Acme canvas sources and locally synthesized Samantha speech were
  media inputs. The transcript was produced by real local whisper.cpp using
  the multilingual small model. The draft was produced by the real signed-in
  Codex subscription with the production read-only tool-disabled invocation.
- All eleven final source frames and five rendered review frames inspected:
  room start, drawing, transcript, summary and the shifted original outro.
  The recap dialog is opaque and readable; final capture checks the theme's
  surface token and the actual modal background before accepting footage.
- `node scripts/validate-rooms.mjs` passed: 1920×1080 at 30 fps, H.264 video,
  stereo 48 kHz AAC audio, duration/chapters consistent, 56 ordered caption
  cues within the movie, genuine recap event types and valid summary citations,
  zero browser runtime errors.
- Full FFmpeg audio/video decode exited 0 with no errors.
- Chromium and WebKit loaded and played the final movie, sought through the
  Rooms chapter and shifted outro, and parsed all 56 caption cues. Captions
  used the same Blob URL approach as the app; a plain file-URL track failed
  the first harness check because it did not load across file origins.
- Final audio measurement: **−16.15 LUFS integrated, −1.58 dB true peak**.
  `out/review-music.wav` contains a nine-second mono 16 kHz listening preview.
  Listening QA was not performed: this tool runtime cannot accept audio input.
  Loudness/peak checks and review of the synthesis do not replace listening.
- Capture/render scripts pass `node --check`; source diff whitespace check passes.
- After publication, the rebuilt production UI loaded the released movie and
  all 56 bundled captions in Chromium light/dark and WebKit dark. The Rooms
  chapter seeks to 2:45.20, opens its guide, and the guide's Watch this part
  action returns to that position. All runs reported zero browser runtime
  errors. WebKit was exercised with an explicit chapter-play gesture; the
  initial harness incorrectly waited for media before that gesture.
- The activated manifest passed `npm run check` with zero errors/warnings,
  `npm run build`, and all eight Help guide unit tests.
- Serial capture/render, whisper threads limited to two and H.264 encoder
  threads limited to two. Both isolated room browser contexts and the daemon
  were stopped after capture.

## Honest coverage limits

The real archive contains an unavailable initial guest-screen frame, a failed
capture upload, a screen OCR timeout and a trailing short speech fragment.
The saved images and usable transcript remain available. The actual Codex draft
reports these limitations; no screen OCR text, transcript or summary was invented
to conceal them. This is a capability walkthrough, not evidence of perfect
recognition or complete continuous screen recording.

The new footage is the browser room workflow, not native macOS window chrome.
It demonstrates shared-terminal control, not arbitrary OS remote control, and
one source per presenter. It does not claim webcam support or multiple sources
from one presenter. Physical multi-Mac/microphone/display acceptance is separate.

## Publication state

The MP4, poster and captions were uploaded to the existing `walkthroughs`
release after review and approval. Each published asset was downloaded again
and its SHA-256 verified against the values above. The app manifest now selects
this edition, and `ui/src/lib/walkthroughs/otto-tour-rooms.vtt` supplies its
bundled caption track. Rooms links to the matching in-app guide.
Original baseline metadata/captions are preserved under `baseline-20260925/`
so later rerenders never append Rooms twice.
