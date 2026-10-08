# Character racing and combat experience — 2026-10-08

Baseline: `187ed746b`. The user requested a complete character kart experience
with countryside, underwater sections and jumps, and a much stronger shooter.

## Delivered behavior

- Four original drivers: Rory (fox), Bao (panda), Pip (rabbit), Bolt (robot).
  Separate shared GLB roots attach to authored DriverSocket nodes. Both human
  players choose their own driver; input/snapshot transport synchronizes it.
- Three smooth, elevated circuits: Coral Coast resort and underwater reef,
  Fernwood country village/bridges, Neon elevated expressway. Two launch points
  per course; gravity, airborne trick and one landing boost per jump.
- Turbo, shockwave, shield and bounded homing seeker items; minimap, race
  position, lap clock, item/speed/drift HUD, original synthesized score, reactive
  exhaust/sparks/bubbles, fullscreen and a character result portrait.
- Pulse rifle, scatter blaster and rail lance have separate behavior/capacity;
  health/shield pickups and collision-safe dash add movement and recovery.
  Recoil/muzzle/impact feedback, shield effects, readable combat HUD and
  architectural arena scenery preserve first/third-person shooting.
- Original Blender roster/props and humanoid fighter update. Final public art
  is 11,757,538 bytes; eight GLBs are 8,050,808 bytes. Editable sources, CC0
  dedication, hashes and re-import checks are retained with the asset bundle.

## Verification record

- Full UI check passed with zero Svelte errors/warnings and all TypeScript
  targets. Full unit suite passed 1,897/1,897 with concurrency four before final
  review regressions. An earlier unrestricted run failed only the unrelated
  sessionBuckets timing threshold (15.48 ms/pass); its isolated rerun and the
  complete constrained run passed without changing its threshold.
- Initial real-browser course controller uses normal keyboard events and only
  reads game state: one Coast lap, 116 airborne frames, 373 underwater frames,
  elevations -5.187..3.996, no unexpected console errors. Those intermediate
  captures and asset hashes are preserved under `../screenshots/experience-initial/`.
- Review identified and corrected reticle projection order after recoil/FOV,
  kart Euler order on slopes, duplicate scatter audio, malformed checkpoint
  snapshot acceptance, HUD overlap and toolbar contrast. Final physics review
  also identified lateral road-height mismatch, off-deck hovering and reload
  transfer on weapon switch; regression results are recorded after correction.

Final build/browser validation is recorded separately with exact built asset
hashes; physical mobile devices and native pointer-lock are outside headless
Chromium coverage. The development entry remains `http://127.0.0.1:5173/#/rooms/games`.

## Final accepted validation

All **17 named browser cases have a passing latest result**. The complete run
passed 16 cases in 2.7 minutes; Neon initially failed an obsolete requirement to
stay on grass for two seconds. The elevated course correctly fell and recovered
sooner. The corrected test requires an actual fall, recovery only after dropping
below the deck, and return within one metre of the last earned checkpoint. Speed
continuity exempts a drop only when a new recorded opponent-hit event and increased
damage timer prove the item impact; ordinary terrain transitions keep the original
speed threshold.

Screenshot review also caught perimeter scenery enclosing the Station and Dunes
starting cameras. Circular placement had cut inside the square play area. The
accepted build projects the decorations onto the square perimeter and moves their
actual rotated/scaled bounding boxes outside it. The three shooter cases and Neon
recovery were rerun: **4/4 passed in 44 seconds**. The other 13 cases were unaffected.
No game state, clock, camera or API response was fabricated. The intentional failed
model-download case aborts one GLB request and verifies real Retry recovery.

Final integration gates: **68/68 game unit tests**, focused Svelte check with zero
errors/warnings, accepted production build and bundle budget passed. The full UI
check and 1,897-test UI run are recorded above. The new arena-boundary regression
covers 28 angles with rotated large decorations, alongside exact road-triangle
height regressions.

The keyboard-driven Coast lap recorded **117 airborne frames, 371 underwater
frames**, one completed lap and elevations **−5.187 to 4.000 metres**. The underwater
capture additionally waits for the actual follow-camera Y below −1.1 m. Neon
recorded three real falls/recoveries. Both independent multiplayer contexts
verified input relay and Panda/Robot driver synchronization, guest invitation
fragment scrubbing, no owner token/API use, ready/start, leave and disconnect.

All six maps, the complete Coast lap and the emulated phone recorded **zero
unexpected browser/console errors**. Observed rendering used Chromium with Apple
M3 Max Metal; these short measurements are not a low-end hardware benchmark.

| Map | Observed FPS | Draw calls | Triangles |
| --- | ---: | ---: | ---: |
| Orbital Station | 60.2 | 369 | 262,996 |
| Ember Foundry | 60.2 | 369 | 273,826 |
| Sunken Dunes | 60.3 | 300 | 111,144 |
| Coral Coast | 60.2 | 373 | 500,306 |
| Fernwood Rally | 59.4 | 245 | 200,734 |
| Neon Overdrive | 60.2 | 248 | 583,156 |

Phone (390×844) observed 60.3 FPS. Phone and tablet (768×1024) both displayed all
four driver choices without horizontal overflow; real touch input drove the kart.
Native pointer lock remains unsupported by this headless browser; the actual
click/drag fallback, Escape release, full-screen enter/exit and focus-loss input
release were verified.

### Exact artifacts and reproducibility

- [Aggregate results and measurements](../screenshots/experience-validation.json)
- [Original 17-case raw results](../screenshots/experience-suite17-results.json)
- [Accepted four-case raw results](../screenshots/experience-accepted-results.json)
- [Final built asset hashes](../screenshots/experience-final-artifacts.json)
- [17-case run artifact hashes](../screenshots/experience-suite17-artifacts.json)

All **14 GLB/PNG files** in the accepted build match their public-source SHA-256.
Final scene chunk: `scene-BNHrwlmD.js`, SHA-256
`205067cff3d9a27e3d5aff8f7d801368b0da6057e3a54232f4fec5ac75b93b8d`.
Kart/mobile/multiplayer screenshots come from the complete run; shooter screenshots
and Neon recovery evidence were refreshed after the arena-only correction.

Run only this named suite, with a fresh debug daemon and a built `ui/dist`:

```sh
cd ui
OTTO_GAMES_PREVIEW=1 OTTO_E2E_SLOT=games \
OTTO_E2E_PORT=7898 OTTO_E2E_PW_PORT=5298 OTTO_E2E_SWEEP_ORPHANS=0 \
OTTO_E2E_BIN=../target/debug/ottod \
npx playwright test --config e2e/room-games.config.ts
```

The affected rerun adds
`--grep 'shooter (station|foundry|dunes):|kart neon: leaving'`.

### Actual player-camera captures

- [Four drivers](../screenshots/experience-four-driver-roster.png),
  [phone roster](../screenshots/chooser-touch-390.png),
  [tablet roster](../screenshots/chooser-touch-768.png)
- [Coast jump](../screenshots/experience-coast-jump.png),
  [underwater reef](../screenshots/experience-coast-underwater.png),
  [completed lap](../screenshots/experience-coast-completed-lap.png)
- [Coast start](../screenshots/kart-coast-starting-camera.png),
  [country village start](../screenshots/kart-forest-starting-camera.png),
  [elevated city start](../screenshots/kart-neon-starting-camera.png)
- [Station start](../screenshots/shooter-station-starting-camera.png),
  [Station fullscreen](../screenshots/experience-station-fullscreen.png),
  [Foundry start](../screenshots/shooter-foundry-starting-camera.png),
  [Dunes start](../screenshots/shooter-dunes-starting-camera.png)

These are ordinary gameplay views, not repositioned inspection cameras. Starting
views and underwater/jump captures show the intended worlds; the sustained-driving
captures deliberately point away from the track to test off-road behavior.
