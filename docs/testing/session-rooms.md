# Session rooms verification

Validation used the isolated `feat/session-rooms-20260927` worktree. Tests used
temporary daemon state and synthetic room sessions, media and recap inputs.
No installed daemon was restarted and no physical microphone or desktop was
captured.

## UI and desktop

From `ui/`:

```sh
npm run check
npm run build
node --test unit/roomClient.test.ts unit/roomMedia.test.ts unit/roomMediaPolicy.test.ts unit/roomRecapCapture.test.ts unit/rooms.test.ts
npx playwright test --config playwright.rooms.config.ts
```

- UI guards, Svelte and all TypeScript configurations: zero errors/warnings.
- Production UI build: passed.
- The complete UI unit suite passed 561 tests, including 17 media regressions.
  Repeated sharing/audio cycles, binding-only relays and permission cancellation
  during pending sender replacement, simultaneous offers and valid ICE during
  offer collision are covered.
- Synthetic browser suite: 19 passed, eight intentionally skipped duplicate
  media/finalization runs. Layout and user flows run in desktop light, desktop
  dark and phone projects; synthetic four-peer WebRTC, AudioWorklet and host-End
  finalization pipelines run once in desktop light. A layout-only rerun also
  passed all nine cases.
- The room and recap routes are included in the shared responsive page and
  page-chrome inventories. This does not imply the entire application E2E suite
  was run for this feature.

With the UI build present, the standalone desktop workspace passed 27 tests:

```sh
TAURI_CONFIG='{"bundle":{"externalBin":[]}}' cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
```

The test override avoids requiring a bundled sidecar. It is compile/unit
evidence, not a signed application build or a packaged remote-window IPC test.

## Final integrated verification

The final integration includes main-worktree commit `707de5d2` (transport,
conversation updates and terminal backlog limits), with room driver fencing and
bounded viewer resynchronization preserved. The fresh daemon build passed in
3m55s, workspace/all-target Clippy passed with warnings denied in 2m43s, and the
separate workspace doctest gate passed. UI checks passed with zero errors or
warnings, all 561 unit tests passed, the production build passed, and the room
browser suite passed 19 cases with eight intentional project skips.

The fresh daemon also passed both live browser cases (18.3s total): room
admission/chat/control/revocation/session survival, and separate browser contexts
using actual HTTP/WebSocket signaling with synthetic media. The latter checks
advancing audio/video counters, five concurrent presentation replacement cycles,
bounded SDP and complete peer/track cleanup.

The workspace nextest run executed 3,750 tests: 3,748 passed and 69 additional
tests were skipped. Two inventory checks initially failed because inline unit
fixtures in the incoming `live_events` module were mistaken for production
routes. Those unchanged fixture tests now live under its `tests/` directory,
which the inventory scanners already exclude; production policies and route
coverage have not been relaxed. The focused rerun passed all five checks: both
moved unit tests, both route-inventory checks and the policy-coverage check.
The other 3,748 passing workspace results remain the evidence for unchanged
code; the complete workspace suite was not repeated after this test-only move.

## Scope of evidence

The initial focused Rust gate passed 43 library tests plus one real HTTP/WebSocket/PTY
integration test:

```sh
CARGO_BUILD_JOBS=2 cargo test -p otto-server --lib --test rooms_api room -- --test-threads=1
```

The route inventory passed both checks after adding every room and recap route
to the main API index. The full workspace build and
`cargo clippy --workspace --all-targets -- -D warnings` passed. All 28 new Rust files
pass `rustfmt --check`, and the rooms diff against integrated main is whitespace-clean. The repository-wide
advisory formatting check still reports existing formatting debt (including
untouched `otto-browser/src/cdp.rs`); no broad reformat was applied.

After integrating main-worktree commit `3c0f9478`, the full workspace nextest
run passed **3,730 tests, zero failed, 69 skipped**. Compilation took 5m54s and
execution took 189.195s on the shared development machine:

```sh
CARGO_BUILD_JOBS=8 cargo nextest run --workspace --profile ci --no-fail-fast
```

This includes the atomic PTY snapshot/subscription regression, viewer pause/resume
with real HTTP/WebSocket/PTY traffic, and safe cleanup of inactive room authority
state while queued writers and mutex holders retain their fences. The fresh daemon
build passed in 3m25s. The separate `cargo test --workspace --doc` gate passed. The integrated full
workspace/all-target Clippy gate passed with warnings denied (1m22s).

A separate live-daemon browser test passed with temporary state and a shim shell
session: host invitation and admission, denied viewer input, room-only chat,
editor control, revocation, stale-input rejection, and host End while the same
underlying PTY remains alive and accepts owner input. This is actual HTTP and
WebSocket transport, rather than a mocked API. Run it with distinct reserved ports:

```sh
OTTO_E2E_SLOT=rooms-live OTTO_E2E_PORT=7806 OTTO_E2E_PW_PORT=5296 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/absolute/path/to/target/debug/ottod npx playwright test --config e2e/rooms-live.config.ts
```

The later main-worktree database commit `fd0cc597` merged without conflicts.
Its API types and contract changes, which were still uncommitted in the main
worktree, were carried over explicitly; a schema-tree discriminant was narrowed
for TypeScript. The combined UI check passed, as did 540 UI unit tests and the
production build before the final media-startup regressions. Those regressions subsequently passed
with 543 unit tests, a clean UI check, a fresh production build and eight focused
desktop browser tests. The affected Rust
rerun (`otto-dbviewer`, `otto-state`, `otto-server`) passed **1,839 tests, 63
skipped**, in 59.592s after compilation. The combined workspace/all-target Clippy
rerun also passed with warnings denied (1m52s).

The browser fixtures test host admission, viewer/editor control, personal pins,
annotation grants, capture consent, bounded real PCM/JPEG ingestion requests,
archive loading, coverage gaps and retryable summary failures. The media fixture
uses four production `RoomMediaClient` objects and exercises resize, source stop
and presenter removal after all four sources reach every participant.

Independent reviews covered terminal write authority, room capability isolation,
native guest-window restrictions, consent epochs, archive ownership, capture
generations, recognition cancellation and summary process isolation.

See [native media validation](room-media-native.md) for measured synthetic
WKWebView results and unmet frame-rate targets, and [recap validation](room-recap.md)
for local speech/OCR and subscription-backed Codex probes. The ten-minute Chromium production-client run passed twenty presentation
replacements, all four participants joined/unmuted, and per-peer advancing audio
RTP and decoded video on six connected peers. Its source lifecycle is supplemented
by the final-source forced-relay run: twenty-five replacements and complete owned
resource cleanup. See the native-media report for exact fixture configuration,
prior control failures, and the timing of the local-preview subscription fix.

The isolated native Tauri probe also passed twenty synthetic audio cycles at each
origin and native guest IPC confinement. Forced TURN passed with six relay-selected
peers in a disposable loopback-only coturn fixture. Real two-Mac networking, physical
capture and sustained combined performance still need hardware acceptance; synthetic
and local-relay tests do not establish them.

## PR integration with current main

The September 28 integration includes `8e9480c0`, the merged performance PR.
Room authority and atomic snapshots are retained alongside the new ordinary
terminal credit protocol. A regression verifies that automatic emulator replies
(`user:false`) do not discard the credit gate's held output; 27 terminal WebSocket
tests passed. Room sockets retain their bounded pause/resume fallback.

The latest-main UI gate passed with zero errors/warnings, 599 unit tests, a fresh
production build, and 19 room/recap browser cases with eight intentional project
skips. Recap archive HTTP polling now uses visibility-aware, abortable polling;
room protocol/media clocks retain their explicit lifetime cleanup.

Representative synthetic fixtures: [room light](assets/session-rooms/room-desktop-light.png),
[room dark](assets/session-rooms/room-desktop-dark.png),
[screens light](assets/session-rooms/screens-desktop-light.png), and
[screens dark](assets/session-rooms/screens-desktop-dark.png).
