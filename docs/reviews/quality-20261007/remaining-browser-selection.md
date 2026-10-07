# Representative browser selection — 2026-10-07

Source-only selection against baseline `196048df` plus the quality worktree repairs. These tests were read, not executed by this reviewer. This is a finite next evidence set: **14 Chromium cases and one mobile WebKit case**, run sequentially with one worker. A pass would support the named flows; it would not certify the whole app or automatically justify a numerical target.

## Selected cases

| # | Spec under `ui/e2e/` and exact test title | Meaningful evidence / fixture boundary |
|---|---|---|
| 1 | `desktop-review5-scheduled-draft.spec.ts`: `scheduled task deep link and save retry preserve newer edits and the leave decision` | Failed save, pending-save edits and Cancel on leaving. Fixture task is disabled, destination `none`; no scheduled agent execution. |
| 2 | `desktop-goal-draft-ownership.spec.ts`: `goal draft Keep stays in A; workspace Discard closes the persistent form and does not resurrect` | Workspace ownership and Keep/Discard outcomes in the real router/modal. Synthetic workspaces; define endpoint mocked; no agent launch. |
| 3 | `desktop-workflow-recovery.spec.ts`: `workflow preflight identifies a broken step and cron editing preserves disabled state` | Actionable invalid-step validation and retained disabled trigger state after editing. Missing HTTP URL is validated; workflow is not run. |
| 4 | `desktop-database-changes.spec.ts`: `database change draft binds validation to selected executor before submission` | Validation is tied to executor selection; review gate stays disabled appropriately. Synthetic `example.invalid` connection; DB/change endpoints intercepted. |
| 5 | `desktop-vault-recovery.spec.ts`: `trash restores a file and edit history compares recoverable versions` | Trash recovery and restored content verified via API. File deletion/restoration occurs only inside `seedVaultDir`'s fixture vault. |
| 6 | `desktop-vault-recovery.spec.ts`: `properties edit preserves note body and the local graph is reachable` | Structured properties save and note-scoped graph navigation in the fixture vault. Its title is broader than the explicit assertions: the test does not independently compare the note body. |
| 7 | `desktop-ux-r4-data.spec.ts`: `Kafka consume validates selectors before reading and produce preserves failed drafts` | Invalid-input prevention; row selection with mouse, Enter and Space; retained failed Produce draft and successful retry. Broker responses and writes intercepted. |
| 8 | `desktop-ux-r4-data.spec.ts`: `redis result editing stages the native command for review without external writes` | Inline edit → review native command → Cancel; query count remains one. Synthetic Redis connection and intercepted query; no external write. |
| 9 | `desktop-ux-r4-content.spec.ts`: `Design Hall responsive editing warm dark` | Warm/dark at 375px, preview/details navigation, notch clearance, source save/version, overflow. Real synthetic HTML artifact; no publication. |
| 10 | `desktop-ux-r4-access.spec.ts`: `loaded shared sheet and setup contrast warm-light` | RTL 375px, yellow custom accent, real computed contrast/text size, setup Axe scan, visible sheet bounds, reduced-motion animation, wallpaper/transparency toggle. |
| 11 | `desktop-ux-r4-access.spec.ts`: `loaded shared sheet and setup contrast warm-dark` | Same checks at RTL 834px with near-black accent. Setup/auth are fixtures; shared production sheet is mounted. |
| 12 | `desktop-ux-r4-access.spec.ts`: `custom accent keeps actual Markdown links and focus visible warm-light` | Production Markdown primitive in a real sheet; measured link contrast and focused-input border against yellow accent. Scoped component check, not arbitrary document accessibility. |
| 13 | `desktop-ux-r4-access.spec.ts`: `custom accent keeps actual Markdown links and focus visible warm-dark` | Same focused contrast checks with near-black accent. |
| 14 | `desktop-shell.spec.ts`: `shell applies NO CSS zoom in a browser, even with otto_zoom set` | Stored `otto_zoom=2` does not apply CSS zoom or produce browser overflow. Seeds a throwaway shell PTY. **Does not exercise Tauri native webview zoom.** |
| 15 | `rtl.spec.ts`: `rtl — home: applies dir=rtl, no overflow, accessible` | Existing iPhone WebKit project: actual RTL direction, horizontal overflow and shared Axe check on Home (zero critical violations; serious violations checked against the existing page baseline). Automated accessibility scope is limited. |

## Exact sequential commands

Run from `/Users/itziklavon/claude_ade-quality-20261007/ui`. Ports 7873 and 5273 and slot 73 are proposed dedicated values: the coordinator must confirm they are unused before starting. Use the fresh binary built from this worktree. Do not run while another heavy gate owns the machine. Keep the default isolated setup/teardown; do not substitute a live daemon or user vault.

```bash
env -u OTTO_E2E_UI -u OTTO_A11Y_RECORD OTTO_E2E_BIN=/Users/itziklavon/claude_ade-quality-20261007/target/debug/ottod OTTO_E2E_SLOT=73 OTTO_E2E_PORT=7873 OTTO_E2E_PW_PORT=5273 OTTO_E2E_GATE=0 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_SECRETS=file npx playwright test \
  e2e/desktop-review5-scheduled-draft.spec.ts \
  e2e/desktop-goal-draft-ownership.spec.ts \
  e2e/desktop-workflow-recovery.spec.ts \
  e2e/desktop-database-changes.spec.ts \
  e2e/desktop-vault-recovery.spec.ts \
  e2e/desktop-ux-r4-data.spec.ts \
  e2e/desktop-ux-r4-content.spec.ts \
  e2e/desktop-ux-r4-access.spec.ts \
  e2e/desktop-shell.spec.ts \
  --project=desktop-browser --workers=1 --retries=0 --reporter=line \
  --output=/tmp/otto-quality-20261007-representative-chromium \
  --grep '(scheduled task deep link and save retry preserve newer edits and the leave decision|goal draft Keep stays in A; workspace Discard closes the persistent form and does not resurrect|workflow preflight identifies a broken step and cron editing preserves disabled state|database change draft binds validation to selected executor before submission|trash restores a file and edit history compares recoverable versions|properties edit preserves note body and the local graph is reachable|Kafka consume validates selectors before reading and produce preserves failed drafts|redis result editing stages the native command for review without external writes|Design Hall responsive editing warm dark|loaded shared sheet and setup contrast warm-(light|dark)|custom accent keeps actual Markdown links and focus visible warm-(light|dark)|shell applies NO CSS zoom in a browser, even with otto_zoom set)$'
```

After that invocation and its teardown finish:

```bash
env -u OTTO_E2E_UI -u OTTO_A11Y_RECORD OTTO_E2E_BIN=/Users/itziklavon/claude_ade-quality-20261007/target/debug/ottod OTTO_E2E_SLOT=73 OTTO_E2E_PORT=7873 OTTO_E2E_PW_PORT=5273 OTTO_E2E_GATE=0 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_SECRETS=file npx playwright test e2e/rtl.spec.ts \
  --project=iphone-portrait --workers=1 --retries=0 --reporter=line \
  --output=/tmp/otto-quality-20261007-representative-webkit \
  --grep 'rtl — home: applies dir=rtl, no overflow, accessible$'
```

`global-setup.ts` supplies temporary data, fake provider CLIs, isolated plugin home and file-backed test secrets; `OTTO_E2E_SWEEP_ORPHANS=0` avoids its cross-run orphan sweep. The config defaults to reusing an existing Vite server outside CI, so dedicated unused ports matter. `OTTO_E2E_GATE=0` prevents the CI smoke allowlist from silently excluding this selection. Keep one worker even though the config permits four.

Capture actual outcomes, including skips, in coordinator-owned logs. Access tests place images and measured contrast JSON in their Playwright output directories. Content screenshots use fixed `/tmp/otto-ux-r4-content-design-warm-dark-desktop-browser.png`; data screenshots use `/tmp/otto-ux-r4-data-screenshots/{kafka-consume,redis-review}.png`. Retain these immediately after the run to prevent later runs replacing them. Inspect images before claiming visual closure; a layout assertion alone is not an image review.

## WebKit, zoom and remaining limits

- Existing `iphone-portrait` and `ipad-landscape` projects can run non-`desktop-*` theme/RTL specs in WebKit. Case 15 uses that path without modifying the harness. This is WebKit engine evidence, not the packaged Tauri app.
- The existing `desktop-webkit` project explicitly matches only `desktop-.*perf.*.spec.ts` (`ui/playwright.config.ts:90`). Passing the selected desktop filenames with that project does not provide desktop accessibility coverage; several desktop specs also skip outside `desktop-browser`. Do not present a no-test/skip result as native evidence.
- No new smoke spec is needed for the selected Warm/custom-accent/reduced-motion gaps: the shared-sheet cases measure the actual animation duration and contrast, rather than merely setting preferences. The alternative Help reduced-motion test requires `OTTO_E2E_TOUR_VIDEO` and skips without it; it is intentionally not included here.
- The existing shell test is a useful CSS-zoom regression, but cannot establish native magnification, hit testing, reflow or focus visibility under Tauri's webview zoom. A future desktop-WebKit functional project could reuse fixtures after checking project-name skips, but still would not exercise Tauri's native zoom integration.
- VoiceOver reading order, announcements, rotor navigation, native menu shortcuts, real macOS accessibility settings and packaged WKWebView behavior remain manual/native verification limits. Browser keyboard and Axe checks do not replace those observations. These limits constrain the scope of claims; they do not create an invented checklist that must be exhausted to reach a requested score.
