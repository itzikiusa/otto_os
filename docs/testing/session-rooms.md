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
- Focused room/recap unit tests: 27 passed.
- Synthetic browser suite: 13 passed, eight intentionally skipped duplicate
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
pass `rustfmt --check`, and `git diff --check` is clean. The repository-wide
advisory formatting check still reports existing formatting debt (including
untouched `otto-browser/src/cdp.rs`); no broad reformat was applied.

The full workspace test run also passed: 3,620 tests passed, zero failed and
66 ignored across 126 suite/doc-test results. It used
`CARGO_BUILD_JOBS=2 cargo test --workspace -- --test-threads=1` to bound validation
resource use and preserve the existing suite's sequential test assumptions.
The final source-lifecycle cleanup was then verified separately: its focused
regression passed, including 100 source replacements without retaining obsolete
revocation tokens or screen-sequence keys. The full workspace suite was not
repeated for that isolated cleanup.

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
for local speech/OCR and subscription-backed Codex probes. Real two-Mac
networking/TURN, physical capture and sustained combined performance still need
hardware acceptance; synthetic tests do not establish them.
