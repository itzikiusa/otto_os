# Hosted room windows and navigation

The root auth effect used to read the route on every navigation. A section
change called `auth.boot()`, reset the phase to loading, and destroyed the shell.
The regression test keeps an actual shell DOM reference and checks both its
identity and the number of metadata requests across sidebar navigation.

Desktop room creation/opening now calls dedicated native IPC without changing
the originating route. The separate authenticated entry loads only room UI,
with credentials fetched from a native registry bound to that exact webview.
The guest invitation window remains unprivileged.

## Reproduce

From `ui/`, select unused isolated ports and an appropriate daemon binary:

```sh
OTTO_E2E_SLOT=host-room OTTO_E2E_PORT=7832 OTTO_E2E_PW_PORT=5232 \
  OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/absolute/path/to/ottod \
  npx playwright test --config=rooms.playwright.config.ts --workers=1
```

From the repository root, on macOS:

```sh
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml --example host_room_native_probe
```

The native probe uses blank bundled fixture assets and no real daemon or user
state. It exercises production commands in WKWebView. The UI tests exercise
the real components with a controlled native IPC boundary. Neither alone is
a full installed-app room/media acceptance test.

## Recorded evidence — 2026-09-28

- Sidebar remount regression failed before the independently authored fix
  was brought over unchanged from `e88f1b88`.
- Five host-window UI cases passed: shell lifetime, start/reopen without
  navigation, retry without duplicate room creation, context loading/retry,
  secret-free URL/storage and light/dark layout.
- Five existing guest cases passed on Chromium, including four-client media,
  admission/chat/driver handoff, annotations and terminal backpressure/resync.
  An initial iPad WebKit run failed because mocked same-origin join requests
  returned 404; that fixture run is not claimed as passing.
- 44 desktop unit tests and all-target Clippy passed.
- The native probe passed eight concurrent opens producing one window,
  caller/route isolation, denied remote navigation/new windows, context cleanup,
  close/reopen with a fresh identity and unchanged source DOM/draft/hash.
- UI check reported zero errors/warnings; production build passed.
- Correctness/security review found no confirmed defects.
- The complete `rooms.playwright.config.ts` suite passed again before
  publication: 10 tests in 32.5 seconds, one worker and isolated ports.

Review screenshots: [host light](screenshots/host-rooms/host-room-light.png),
[host dark](screenshots/host-rooms/host-room-dark.png),
[walkthrough light](screenshots/host-rooms/walkthrough-light.png), and
[walkthrough dark](screenshots/host-rooms/walkthrough-dark.png).

Closing a hosted room disconnects that view and pauses recap capture. The
underlying session stays alive; End room remains explicit. A room can keep Otto
alive after the source window closes, but ephemeral host windows are never
restored from the workspace registry. Microphone/screen permission dialogs and
physical cross-device behavior remain hardware acceptance checks.
