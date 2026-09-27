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
  source permission denial, real source resize behavior, host sleep, signed-release
  remote-window IPC denial, physical cleanup and two-Mac networking remain
  unverified. ScreenCaptureKit's picker requires macOS 14; Otto's deployment
  target remains 12.0. No native ScreenCaptureKit bridge is claimed.
- For this WKWebView benchmark, the ten-minute reference workload, constrained-
  network and TURN fixtures, pin-to-sharp-frame time, voice latency, terminal
  latency delta, twenty full room cycles, GPU/power/thermal measurements and Intel
  hardware remain untested. Separate Tauri lifecycle and Chromium relay evidence
  appears below. This benchmark does not justify claiming performance acceptance or rejecting the
  architecture in favor of an SFU based on per-host resource cost.

## Production implementation checks

### Isolated Tauri runtime probe

The standalone desktop example deliberately omits Otto's daemon supervisor,
window restoration, shortcuts, plugins and product SPA. It uses the production
`rooms.rs` command, Tauri context and capabilities, a blank bundled-origin page,
and a temporary loopback guest server. Its generated app has a separate bundle
identifier and ephemeral windows. Build and run it without installing Otto:

```sh
TAURI_CONFIG='{"bundle":{"externalBin":[]}}' cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --example room_native_probe
python3 scripts/room-media-probe/native.py --binary apps/desktop/src-tauri/target/debug/examples/room_native_probe --cycles 20
```

When using `CARGO_TARGET_DIR`, pass the corresponding executable path. The runner
records its results in a new temporary directory and has a 45-second deadline.
It checks local IPC as a positive control, guest denial of native window creation,
window access and event subscriptions, secret-free rejection messages, blocked
navigation and popups, and the production recap AudioWorklet with generated
oscillator audio in both webviews. A harmless, unguarded `daemon_restart` stand-in
must run exactly once for the local positive control and never for the guest;
the native counter independently detects entry to that handler. `--cycles 20`
repeats audio-context creation, module loading, synthetic stream processing,
flush and cleanup twenty times per window. This is audio lifecycle coverage,
not twenty full room/presentation cycles or a memory-leak measurement.
It never calls `getUserMedia` or
`getDisplayMedia`, starts a daemon, or loads the user's Otto state. API exposure
is only a capability check; this fixture cannot establish real capture, picker
selection, full production startup, packaging/signing or hardware performance.

The existing plain WKWebView benchmark has no `WKUIDelegate`, unlike Wry. A
separate temporary delegate fixture on 2026-09-27 exposed both capture methods
and ran the production recap worklet: 105,728 samples at 48 kHz, synthetic RMS
0.699, successful flush, closed audio context and ended destination track. It
made no capture request. Historical delegate-related display-capture failures
were [fixed upstream in WebKit in June 2024](https://bugs.webkit.org/show_bug.cgi?id=274896);
they do not prove a current failure or success. Real source-picker validation
must still cover both the current system and Otto's oldest supported macOS.

The isolated Tauri example passed on 2026-09-27 with `--cycles 20` using the
actual desktop dependency lockfile and production room command/capabilities:

```text
Artifacts: /var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-room-tauri-probe-8mzdh_dq
Exit: 0; passed: True
custom_command_calls: 1; navigation_confined: true; window_count: 2
main: 20 audio cycles passed; room-1: 20 audio cycles passed
```

Both origins exposed capture APIs. The local custom-command positive control
ran once; the guest's call was rejected before the handler, leaving the native
counter at one. Guest creation, local-window read/mutation and event listening
were denied without echoing the synthetic invitation secret. Navigation to
`tauri://localhost/`, `https://tauri.localhost/` and another path at the guest
origin was blocked, and the popup created no additional window. Every audio
cycle verified synthetic PCM, flush completion, ended owned tracks and a closed
context. This establishes these boundaries in an isolated unsigned Tauri app;
it does not establish physical capture or a complete signed Otto release.

For a person at the test Mac, the same example supports an opt-in physical check:

```sh
python3 scripts/room-media-probe/native.py --binary apps/desktop/src-tauri/target/debug/examples/room_native_probe --physical
```

This opens separate local and guest diagnostic windows for up to fifteen minutes.
Nothing captures on launch. In each window, explicitly enable the microphone,
speak briefly, choose a neutral display/window, resize it, use the system stop
control, and press Stop all. After checking the OS indicators clear, check the
confirmation box and finish that window. Only sample/frame counts, RMS, geometry,
permission error names and cleanup metadata are saved; no media, device names,
screen titles or transcripts are retained. This mode has been prepared but has
not been run with a person or physical capture. It is not full room-call or
performance acceptance.

### Production-client stability and local TURN

`stability.mjs` runs four real `RoomMediaClient` objects in a blank Chromium page
served by an isolated Vite instance. Captures are generated 640×360 canvases
at 3 fps and oscillator tones;
signaling is in-process. It repeatedly stops/restarts a presenter, checks all four
sources at every client and all six connected peers, verifies advancing received
audio bytes/packets and decoded video frames, requires all four voice members
to be joined and unmuted, and checks that every owned track, peer and audio context
is ended/closed during cleanup. The standard workload is:

```sh
node scripts/room-media-probe/stability.mjs --stun --task-signaling --seconds 600 --cycles 20
```

The `--stun` option starts a temporary loopback-only STUN Binding responder.
`--task-signaling` delivers fixture messages as event-loop tasks, like incoming
WebSocket messages; the default retains immediate microtask delivery for stress
investigation. Both modes retain the strict six-connected-peer startup gate.
The runner records the selected modes and bounded signaling/RTC diagnostics.
Audio startup waits for each membership/mute snapshot acknowledgment before
continuing, matching server-mediated room state rather than assuming an
immediate in-process response. The fixture rejects self-subscriptions, as the
real backend does.

A separate control imports no product code and implements W3C perfect
negotiation with generated audio/video. Compare startup conditions with:

```sh
node scripts/room-media-probe/negotiation-control.mjs
node scripts/room-media-probe/negotiation-control.mjs --stun
node scripts/room-media-probe/negotiation-control.mjs --stun --permission
node scripts/room-media-probe/negotiation-control.mjs --stun --fake-capture
```

On this Chromium build, simultaneous offers sometimes left the polite peer
permanently gathering with zero candidates, independently of `RoomMediaClient`.
Noncolliding control offers connected. Loopback STUN, browser microphone
permission alone, and task delivery did not reliably resolve the independent
collision failure. Explicit fake-device capture passed five collision controls,
but a production-client run with the same fake-capture mode still failed startup.
This evidence does not establish a general workaround or physical-capture
behavior. `--fake-capture` launches Chromium with both explicit fake-device
flags; it never opens a physical device and stops its generated capture track.
The four clients share a page, so this mode also cannot establish behavior of a
separate muted host browser. Reports preserve failed runs; a connected sustained
run is lifecycle evidence, not proof that all startup conditions pass.

The ten-minute synthetic run passed on 2026-09-27: 20 presentation replacements,
21 samples, six connected peers, all four voice participants joined/unmuted,
advancing audio bytes/packets and decoded video frames at every sample, and
four media sections per peer. Cleanup closed all six peers and eight audio
contexts and ended all 31 owned tracks. Artifacts:
`/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-room-stability-UmB7ei`.
This run loaded the lifecycle/negotiation fixes before the later own-source
subscription suppression change; the subsequent 25-cycle relay run below
includes that change and a fixture that rejects self-subscriptions. The earlier
25-cycle presentation-only result is not counted as voice acceptance: delayed
fixture snapshots had left guests out of audio. Awaited membership and explicit
audio RTP assertions corrected that test gap. This 640×360/3 fps single-browser
workload is not the native reference-performance workload or a leak measurement.

The initial sustained test exposed a production defect at the fifth presentation
restart: new transceivers accumulated until SDP exceeded the eight-media-section
limit. The parser correctly rejected that SDP, leaving two guests without the
new source. Sender/receiver reuse fixes this without raising the protocol limits.

An optional local relay fixture uses the [official coturn image](https://github.com/coturn/coturn/tree/master/docker/coturn)
for release `4.18.0-r0`, pinned to manifest digest
`sha256:bbefd3e1fdfdc0d58770fe01b581fd8b00d9f3a5580d00acb77cf719a6bc78e3`.
Download it explicitly before running the fixture:

```sh
docker pull coturn/coturn@sha256:bbefd3e1fdfdc0d58770fe01b581fd8b00d9f3a5580d00acb77cf719a6bc78e3
node scripts/room-media-probe/stability.mjs --relay --task-signaling --seconds 30 --cycles 2
```

The container has a unique name, loopback-only published ports, a bounded
32-port relay range, no host-network mode or mounted user data, and is removed
in cleanup. Test credentials are temporary. Loopback peers are allowed only in
this disposable fixture. The run passed with all six selected local candidates
of type `relay` and production `iceTransportPolicy: relay`. The final-source
60-second run passed 25 presentation replacements and 26 samples with all four
voice participants joined/unmuted, increasing audio bytes/packets and decoded
video frames on every peer, four media sections per peer throughout, and six
closed peers, 36 ended tracks and eight closed audio contexts. Artifacts:
`/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-room-stability-kHSgfA`.
A deliberate SIGTERM test also wrote an interrupted report and removed its
container. Vite middleware mode and runner-owned signal handling keep cleanup
under the fixture's control.
This demonstrates forced relay on one machine through Docker; it does not
establish cross-network NAT traversal, TURN/TLS, lossy-network performance,
WKWebView relay behavior, or per-Mac resource budgets.

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
the packaging sidecar. That suite is compile/unit evidence; the separate isolated Tauri runtime
boundary checks are described above.
