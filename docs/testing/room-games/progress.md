# Rooms games execution record

Branch: `feat/room-games`. Base: `4ffd00681e2afc909076665d107d4edc289b8b61`.
Design and plan: `docs/superpowers/{specs,plans}/2026-10-08-room-games*.md`.

## Delivered

Arena Duel and Circuit Clash, each with three environments, human or computer
opponents and Easy/Normal/Hard AI. Shooter first/third-person cameras; racer
ordered laps, drift boost, pickups and recovery. Standalone game-room invitations,
readiness, countdown, results and rematch. Original Blender art, animation and
sound, loading/retry and connection states. Games and Three.js load lazily.

Host-authoritative friendly multiplayer uses a capability-scoped Rust relay.
No listener/tunnel is enabled automatically; there is no competitive anti-cheat.
The installed app and original main checkout were not replaced during this work.

## Verification

- Rust integration gate: 1,509 tests executed; 1,507 passed initially. Two
  expected route-inventory/policy-snapshot failures were corrected and both
  focused reruns passed. Three scale tests skipped by the existing profile.
- All-target `otto-server` clippy with `-D warnings`, formatting, doc tests for
  affected libraries and LOC ratchet passed. One existing server documentation
  example remains ignored.
- UI unit suite: 1,858 passed. Subsequent client-only additions: six client
  tests passed (four added), and unit TypeScript checking passed.
- UI guards, Svelte checking (zero errors/warnings), TypeScript checks,
  production build and bundle budget passed. Ordinary Rooms stays within its
  existing bundle budget after making GamesHub lazy.
- All 11 desktop browser cases validated with the real isolated daemon and
  real GLBs. Ten passed in the final combined run; the remaining station case
  passed after correcting a headless-only pointer-lock expectation. Coverage:
  light/dark setup; all six maps; movement, focus release, camera/audio/menu;
  failed model load followed by successful Retry; host+guest input relay;
  explicit departure; disconnected-match pause; guest owner-API isolation.
- A twelfth browser case passed on the final lazy route: phone (390×844) and
  tablet (768×1024) setup fit without horizontal overflow; real touch input
  moved the kart, and touch Menu/Leave worked. Screenshots are included.
- Actual Mac Metal GPU measurements at 1440×1000: approximately 59.6–60.4 FPS.
  Per-map measured geometry/draw calls and console results are in
  `screenshots/*-render-report.json`. These are a short local sample, not a
  cross-device performance guarantee.
- Six GLBs validated by Blender re-import: bounds, roots, clips, node names,
  bytes and hashes. About 4 MiB geometry / 6.4 MiB complete public assets.

## Review corrections

Independent backend, simulation and UI reviews found and corrected neutral
kart rollback, bot navigation against cover, lost finish acknowledgement after
connection-generation changes, downward road faces, retained pointer capture,
ignored initial mute/skin preference, and misplaced impact effects. Regression
tests cover gameplay/geometry/input/finish cases. Real browser testing found and
fixed Svelte state mutation during focused-canvas teardown.

Reconnect tests use controlled clocks and sockets; in-memory production
mutations proved they fail if the retry bound, heartbeat cleanup or retry timer
cleanup is removed. Full malformed/stale snapshot validation is exercised.

## Verification boundaries

Chromium headless rejects native pointer capture with `WrongDocumentError`,
including an independent minimal canvas reproduction. Real-click drag-to-look,
firing and Escape release were browser-tested; pointer-lock release has a unit
regression. Packaged WKWebView/native pointer capture, a physical remote
computer and WAN latency were not tested. Simulation/unit and real socket tests
cover results/rematch; browser cases do not wait through a complete multiplayer
match. The host game must remain visible because background tabs suspend rAF.
