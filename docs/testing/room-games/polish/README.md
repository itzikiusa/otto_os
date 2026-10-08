# Playability and visual correction — 2026-10-08

User feedback: Easy shooter was effectively impossible; art was too basic;
kart motion jumped and steering felt rough. Baseline commit `f6033b30a`.

## Causes and corrections

- Easy previously held fire continuously after tracking the player; its small
  angular error was nearly irrelevant at close range. A three-seed exposed
  target scenario averaged its first kill at 0.85 seconds (0.63 against a moving
  target). Easy now has 2.2-second acquisition, brief firing windows separated
  by long rests, slower tracking/movement and spatial aim error. The same
  scenarios now average 29.23 and 34.23 seconds. This is a deterministic balance
  probe, not a promise of human survival time. Damage ordering Easy < Normal <
  Hard is tested over 10 and 30 seconds on all arenas.
- Kart speed was clamped from 23 to 8 m/s in one tick on grass, and from 33 to
  23 at boost expiry. Speed now approaches its target progressively. Steering
  is persisted and eased, with less turn authority at high speed. Nearby grass
  no longer triggers a four-second teleport; distant recovery and explicit R
  reset remain. Rendering damps fixed-tick positions/short-arc yaw, follows a
  smoothed camera target and uses frame-rate-independent FOV. Wheel steering
  composes before axle spin, avoiding a visible wobble each revolution.
- Pale materials and strong fill washed out the first art pass. Saturated
  Blender materials, redesigned front/rear silhouettes, feathered palms,
  textured pavement/floor panels, curb paint, readable signs, trackside
  structures, sky gradients and restrained lighting now provide more detail
  and contrast. Coast uses lower, distant islands with a visible ocean.

## Verification

- Final full UI unit run: **1,879 passed**, zero failures or skipped tests.
  Difficulty, driving, interpolation, geometry and transport cases run against
  production code, including the final scenery/wheel regressions.
- Final `npm run check` passed: UI guards, Svelte (zero errors/warnings), and
  app tooling, E2E and unit TypeScript checks.
- Production build and bundle budget passed. Independently lazy-loaded Rooms
  destinations reduce the Rooms entry bundle to 2,279 gzip bytes. No Rust
  changes in this correction pass.
- Final production-preview browser run: **15/15 passed in 5.9 minutes**, zero
  flaky/skipped cases. Covers six maps, real two-client play, load/retry,
  focus/input/camera/menu, light/dark, phone/tablet, touch driving and returning
  to the normal Rooms lobby. The isolated test daemon does not touch user data.
- Three nine-second continuous-driving scenarios sampled 539–542 frames, each
  spending >5 seconds on nearby grass with zero resets. Coast/neon maximum
  position step was 0.271 m and camera step <0.294 m; Forest had a 50 ms frame
  with 0.711 m position and 0.873 m camera steps. Raw observations are
  `../screenshots/*-driving-continuity.json`.
- Mac Metal GPU desktop short samples: 58.5–63.1 FPS (approximately 60;
  measurement windows can read slightly above refresh rate), phone emulation
  53.2 FPS under shared host load. Zero unexpected shader/page/console errors.
  These are local short samples, not guarantees for other GPUs or physical
  phones. Native pointer lock is unavailable in headless Chromium; real
  click/drag/firing and Escape fallback were verified.
- Earlier cold Vite mobile runs timed out while development modules were still
  loading under shared host load. Removing unrelated eager Rooms imports cut
  that route's dependency graph; final verification used the built UI through
  Vite preview (`OTTO_GAMES_PREVIEW=1`). The development play URL is unchanged.
  `../screenshots/production-validation.json` records final asset hashes and
  observations; `production-playwright-results.json` records the full run.
- Blender re-import and SHA-256/size validation passed. Public art 8.13 MiB;
  six GLBs 5.65 MiB. Editable original source and CC0 dedication retained.
- Independent review identified and corrected sign UV removal during batching,
  invisible Dunes collision gaps and front-wheel Euler-order wobble.

Before images: `../screenshots/before-polish/`. Updated game images:
`../screenshots/`. Character/vehicle comparisons:
`ui/assets-src/room-games/previews/comparison.html` from the repository root.
