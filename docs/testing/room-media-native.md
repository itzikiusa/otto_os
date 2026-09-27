# Native room media validation

This is feasibility evidence, not native Otto acceptance. No user's desktop,
camera or microphone was captured, no OS capture permission was requested, and
no installed Otto process or daemon was restarted.

## Repeatable synthetic WKWebView probe

From the repository root on macOS with Python 3 and the Command Line Tools:

```sh
python3 scripts/room-media-probe/run.py
```

The command compiles a temporary unsigned `.app`, opens a synthetic fixture
window without requesting focus, and runs for approximately 80 seconds. It
starts a temporary UDP STUN fixture bound only to `127.0.0.1`, with an ephemeral
port. It prints the artifact directory containing `environment.json`,
`benchmark.jsonl`, `benchmark.stderr`, and `resources.json`. An explicit
`--output /absolute/new-directory` is accepted; a nonempty directory is rejected.
It never runs `getUserMedia` or `getDisplayMedia`.

Four generated 1920×1080 editor canvases and oscillator audio sources represent
four participants in one WKWebView. Three guest↔host peer pairs transmit through
native WebRTC. The host adds received video tracks directly to its other peer
connections: no incoming video is canvas-recaptured for transport. The probe
briefly draws received video into a measurement canvas solely to decode the
synthetic timestamp, not to retransmit it. Host audio mixes exclude their
recipient's synthetic source; this prototype uses the generated source nodes
within the single document, so it does not validate received-audio mixer
correctness or voice latency. Production-client tests separately exercise the
actual client with four browser peers.

Phases are different pins, grid, hidden outgoing views, and resumed pins.
`repin` currently resumes the original pin assignment; it does not measure
switching to a different source. The harness explicitly drives synthetic canvas
updates from a native timer because background WebKit timers can throttle or
suspend. The fixture keeps the publishers active while hiding host forwarding,
so hidden-phase publisher traffic is intentional probe overhead, not evidence
that the production demand coordinator stops publisher upload.

## Recorded native run

Run on 2026-09-27: Apple M5 Pro, 48 GiB RAM, macOS 27.0 (`26A428`). This is a
plain system WKWebView fixture, not a packaged Otto release. Otto's checked-in
lockfile uses Tauri 2.11.2 and wry 0.55.1. Successful repeatable-tool output:

```text
Artifacts: /var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-room-media-eaycys3t
Exit: 0; samples: 40; fixture errors: 0
```

All three peer pairs connected (six endpoints), with nine host outgoing video
copies for four concurrent presenters. The audio context was running. At the
end all eleven owned tracks were `ended`, all six peers were `closed`, and the
audio context was `closed`.

| Phase | Mean host video payload egress | Synthetic timestamp latency p50 / p95 |
| --- | --- | --- |
| Different pins | 1.318 Mbps | 63 / 92 ms |
| Grid | 2.153 Mbps | 62 / 84 ms |
| Hidden forwarding | 0.008 Mbps | Not an acceptance measurement; host still views publisher streams |
| Resume original pins | 1.832 Mbps | 62 / 82 ms |

Payload rates come from outbound RTP `bytesSent` differences. They exclude
transport/TURN overhead and include a warmup sample. Synthetic latency combines
host and guest video observations on one clock over localhost; it is not
physical capture-to-display latency or the specified 50 ms network fixture.

The sum of sampled process CPU percentages averaged **64.81 percentage points**
and peaked at **98.1**. Aggregate RSS peaked at **1020.34 MiB**, versus **147.13
MiB** at the early idle sample and **561.02 MiB** after cleanup. These values
include *all four simulated clients* in one web content process plus the probe,
new WebKit networking and GPU processes. They cannot establish the per-Mac host
CPU or 400 MiB memory acceptance budget. Sampling `ps %cpu` is an approximation,
not integrated CPU time. Concurrently started WebKit apps can contaminate XPC
process attribution. The retained post-cleanup RSS also does not establish a
leak: the run did not perform twenty cycles or force collection.

## Findings and limits

- A bare executable exposed neither capture function. The same executable in
  an app bundle with `NSMicrophoneUsageDescription` exposed both
  `getUserMedia` and `getDisplayMedia`. Actual OS capture and picker behavior
  remain untested. The unsigned fixture emitted a sandbox-extension warning
  but completed the synthetic workload successfully.
- Web Audio produced independent destination tracks, and WebRTC negotiated
  audio, video and Opus. Offline audio rendered the expected synthetic sample.
  `MediaStreamTrackProcessor` and `MediaStreamTrackGenerator` were unavailable.
- Empty ICE configuration without capture permission did not reliably gather
  candidates. Loopback STUN enabled the complete topology without granting
  capture access. This supports providing explicit host ICE configuration;
  it does not establish TURN or cross-network connectivity.
- Downscaling and bitrate parameters worked. **Preview/grid cadence targets
  were not met:** WebKit accepted `maxFramerate=2` / `5` but reported roughly
  6–8 fps, following the synthetic source. Per-recipient `maxFramerate` remains
  best effort. The production client additionally constrains local capture frame rate to
  the highest subscribed tier, without lowering a full viewer for a preview.
- Actual microphone capture/playback, display/window/application selection,
  source permission denial, real source resize behavior, host sleep, packaged
  remote-window IPC denial, physical cleanup and two-Mac networking remain
  unverified. ScreenCaptureKit's picker requires macOS 14; Otto's deployment
  target remains 12.0. No native ScreenCaptureKit bridge is claimed.
- The ten-minute reference workload, constrained-network and TURN fixtures,
  pin-to-sharp-frame time, voice latency, terminal latency delta, twenty-cycle
  cleanup, GPU/power/thermal measurements and Intel hardware remain untested.
  The run does not justify claiming performance acceptance or rejecting the
  architecture in favor of an SFU based on per-host resource cost.

## Production implementation checks

`ui/e2e/fixtures/room-media-smoke.ts` uses four real `RoomMediaClient` objects
with generated canvas/audio streams and local signaling. Its Playwright test
caught a production failure: an unnegotiated Chromium sender exposed encodings
but no codecs; `setParameters()` never settled and blocked forwarding a later
presenter. The client now waits for stable signaling and negotiated codecs.
The four-client test passed after the fix, with all four sources at all four
participants. The strengthened lifecycle regression also verifies source resize
advances generation, stopping a presentation preserves the remaining sources,
and removing a presenter clears its own view and downstream copies. It exposed
a geometry feedback loop from capture resolution constraints; resolution now
scales only at RTP senders, while capture demand changes only frame rate. The
synthetic canvases repaint continuously so frame limiting cannot swallow the
only resize frame. Browser verification is supplementary, never a substitute for
packaged WKWebView validation.

Focused Node tests cover capture races, host generation replacement, server
coordinator shutdown, moderator mute, source geometry versus quality scaling,
minus-self contributor selection, and aggregate bitrate policy. Geometry
changes use capture-track settings, not screenshot polling. Resizing that the
platform hides behind unchanged delivered dimensions cannot be detected by
this API; native geometry acceptance remains unverified.

Native desktop tests cover invitation schemes, loopback exceptions, reserved
`tauri.localhost` rejection, secrets absent from error strings, navigation
confinement and capability policy. The full standalone test suite passed
27/27, first with fixture `frontendDist`, then again with the actual production UI
build and a test-only empty `externalBin` override. The isolated worktree lacks
the packaging sidecar. This is compile/unit evidence, not a packaged runtime IPC
exploit test.
