# Detachable panes verification

The native and browser boundaries require separate checks. Chromium cannot
prove WKWebView reparenting, and a synthetic native document cannot prove the
Svelte bootstrap and controls.

## Automated checks

From `ui/`:

```sh
npm run check
npm run test:unit
npm run build
OTTO_E2E_SWEEP_ORPHANS=0 npx playwright test \
  e2e/desktop-detachable-panes.spec.ts \
  e2e/desktop-side-by-side.spec.ts \
  e2e/desktop-agent-ui-control.spec.ts \
  --project=desktop-browser --workers=1
```

Set `OTTO_E2E_BIN` to the daemon binary under test and select unused
`OTTO_E2E_SLOT`, `OTTO_E2E_PORT`, and `OTTO_E2E_PW_PORT` values when another
test run is active. The harness creates temporary state and provider stubs.

The detachable spec uses a controlled native IPC fixture to exercise the real
shell: both directions, preserved primary DOM, one child across narrow-window
resizing, creation failure/Retry, native return events, overlay occlusion,
agent shell commands, event targets, and light/dark menus. The existing agent
suite covers real daemon grants, write confirmation, Deny and Stop/revoke.

From the repository root, on macOS:

```sh
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml \
  --example pane_reparent_probe --example pane_lifecycle_probe --example pane_spa_probe
apps/desktop/src-tauri/target/debug/examples/pane_reparent_probe
apps/desktop/src-tauri/target/debug/examples/pane_lifecycle_probe
```

These examples do not run normal app setup, launch the installed daemon, or
restore/write the real window registry. They use nonpersistent native webview
storage. The first checks repeated movement of an unchanged document, draft,
scroll position and timer. The second runs production pane IPC and tests
ownership, event delivery, visibility, focus, close-to-return and quit policy.
It also exercises Keep on top, fullscreen, maximize, available-display
movement and restoration after moving the primary pane between scale factors.

After building the UI and `pane_spa_probe`, run from `ui/`:

```sh
OTTO_E2E_BIN=/absolute/path/to/ottod node scripts/probe-detachable-panes.mjs
```

The wrapper reserves test port 7821, creates a throwaway daemon and workspace,
and supplies temporary credentials only to the isolated probe. Both native
documents use separate nonpersistent stores seeded with fixture settings.
The probe loads the actual bundled SPA and exercises real toolbar controls,
child bootstrap/Return, unchanged documents/drafts, native sizing and menus.
The wrapper tears down its daemon and temporary data in `finally`.

## Physical acceptance

In the built desktop app, verify with the displays and window settings used
in practice:

- Agents + Connections: run a query, leave an unsaved edit, detach either
  side, Return, and confirm results and edits remain without a second query.
- Move the floating window between displays with different scale factors;
  disconnect its display and confirm controls remain reachable.
- Enter native fullscreen, Return, and verify original host geometry.
- Keep on top on/off; close either detached surface; quit and reopen.
- While an agent has a grant, detach its target, issue another command, then
  Stop. No action should run after revocation.

Physical display removal and screen scaling require the corresponding hardware;
the browser fixture is not evidence for those behaviors.

## Recorded run (2026-09-28)

- UI check: zero errors/warnings; production build passed.
- 608 UI unit tests passed with `node --test --test-concurrency=1
  'unit/**/*.test.ts'`. The default parallel run had one timing-only failure
  in the unchanged JSON preview benchmark (6.35 ms versus 4.14 ms); that file
  passed in isolation. No performance threshold was weakened.
- Native lifecycle probe exercised three connected displays (one 2× and two
  1×), and verified exact original host position/size after primary-pane
  moves to each display. Live document identity, draft and scroll survived.
- Real daemon agent-control E2E: all three scenarios passed, including
  read-only query execution, write confirmation, Deny and Stop/revoke.
- Full bundled-SPA native probe passed: real host/child bootstrap, both detach
  directions, child-toolbar Return, unchanged DOM/drafts, native sizing and
  targeted native menu delivery. The probe waits for native page-load completion
  before polling the DOM; its initial run exposed that missing harness wait.
- Detachable host fixture and browser split regression: all 13 scenarios
  passed. Light/dark fixture screenshots were inspected; menus stay in bounds
  at 780 × 640. These screenshots exercise Chromium plus the native IPC
  fixture, while the separate native probe exercises WKWebView.
- Desktop unit tests: 37 passed; all-target Clippy passed with warnings denied.
  The example-only startup wait also passed focused Clippy after the full-SPA run.
- Physical display unplug/replug remains a manual acceptance check.
