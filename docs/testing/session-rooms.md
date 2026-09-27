# Session rooms verification

Validation used the isolated `feat/session-rooms-20260927` worktree. Tests used
temporary state and synthetic media. No installed daemon was restarted and no
real microphone, desktop, user transcript or room was recorded.

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
- The complete UI unit suite passed 535 tests, including 13 media regressions.
  Repeated sharing/audio cycles, binding-only relays and permission cancellation
  during pending sender replacement are covered.
- Synthetic browser suite: 16 passed, eight intentionally skipped duplicate
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

## Scope of evidence

The focused Rust gate passed 43 library tests plus one real HTTP/WebSocket/PTY
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
for local speech/OCR and subscription-backed Codex probes. The isolated native Tauri probe also passed twenty synthetic audio cycles at each
origin and native guest IPC confinement. Forced TURN passed with six relay-selected
peers in a disposable loopback-only coturn fixture. Real two-Mac networking, physical
capture and sustained combined performance still need hardware acceptance; synthetic
and local-relay tests do not establish them.
