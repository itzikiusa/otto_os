# Native and transient-state acceptance — 2026-10-07

Finite follow-up: four plugin visual/keyboard journeys and two existing native probes. This supplements the 15 representative browser cases; it is not an obligation to run every conceivable configuration to obtain a target score. The reviewer prepared source only; the coordinator owns all compilation/execution. No installed Otto instance or user state is an acceptance target.

## Safe native capability found

`apps/desktop/src-tauri/examples/pane_spa_probe.rs` is an existing debug-only Tauri/Wry probe that loads the **actual bundled SPA**. It clears ordinary app windows from the generated configuration, uses a process-specific application identifier and incognito/nonpersistent WKWebView stores, and registers only the probe/production pane commands. It does not run the desktop application's daemon supervisor, window-registry restore/save, global shortcuts, capture devices or installed-app setup.

`ui/scripts/probe-detachable-panes.mjs` creates a throwaway daemon through the standard E2E setup and a workspace inside its temporary directory. It passes a mode-0600 fixture config to the example; the example requires loopback and rejects port 7700. The wrapper reserves daemon port 7821, fixture origin port 5201 and slot `native-pane-probe`; these must be free. It cleans up its own daemon and temporary data in `finally`. Provider CLIs, plugin home and secret storage remain fixture-isolated. Do not print its temporary token/config contents.

The worktree had no desktop target before this follow-up. Root has now built the standalone desktop dependencies with two jobs and disabled debug information/incremental output; no dependency installation or global accessibility change is needed. Existing dependencies provide real native page zoom, focus, window bounds and JavaScript evaluation. No existing VoiceOver/AX-tree automation harness was found.

## Six finite cases

| Case | What it establishes | Boundary |
|---|---|---|
| Existing `pane_lifecycle_probe` | Production pane IPC ownership, actual native focus transfer, detach/return, live synthetic draft/DOM/scroll retention, window geometry restoration | Synthetic HTML in real WKWebView; not actual application layout or VoiceOver |
| Strengthened `pane_spa_probe` | Actual bundled-SPA detach/return continuity, production Add Workspace field retains unsaved draft/focus through native menu zoom 100→200→190→100%, dialog/field remain in viewport and no horizontal page overflow | Programmatically emits the real native-menu event through the production listener/IPC; does not simulate physical Cmd-key delivery or macOS menu UI. Cancel closes without creating a workspace. |
| Plugin 390 light | Actual report list→viewer→nested Download; Tab wrapping/Escape/focus return; comment draft preservation; scan running→stale→Retry→stopping→stopped | Chromium, real plugin UI with intercepted synthetic report/status transport |
| Plugin 390 dark | Same journey in dark phone layout | Same boundary |
| Plugin 1280 light | Same journey in light desktop layout | Same boundary |
| Plugin 1280 dark | Same journey in dark desktop layout | Same boundary |

The plugin cases use production views, not a synthetic parent dialog. They check the full forwarding/masking explanation in Download, covered-parent inertness, top-layer focus trapping, both Escape levels and return to the actual Open trigger. No download is confirmed, no comment submitted and no report generated. Status requests are intercepted; Stop is checked exactly once and its disabled stopping state is visible, followed by the retained-results explanation. Five screenshots per case are optional through `OTTO_TP_SCREENSHOT_DIR`: report, nested Download, stale, stopping, stopped. They must be visually inspected after execution.

The native zoom check removes only the probe's previously appended synthetic draft elements before checking production overflow, so those helper nodes cannot manufacture clipping failures. Its new draft lives in the actual Add Workspace form. The existing pane identity assertions remain.

## Coordinator commands

From repository root, after a fresh UI build and with the heavy slot available:

```bash
TAURI_CONFIG='{"bundle":{"externalBin":[]}}' CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build --locked --jobs 2 --manifest-path apps/desktop/src-tauri/Cargo.toml --example pane_spa_probe
```

From `ui/`, after confirming ports 7821/5201 are unused:

```bash
OTTO_E2E_BIN=/Users/itziklavon/claude_ade-quality-20261007/target/debug/ottod OTTO_SECRETS=file node scripts/probe-detachable-panes.mjs
```

From `examples/plugins/team-performance/`, after the native process/teardown finishes:

```bash
OTTO_TP_REQUIRE_BROWSER=1 OTTO_TP_SCREENSHOT_DIR=/tmp/otto-quality-20261007-plugin-transients node --test --test-concurrency=1 --test-name-pattern='quality visual closure' test/browser.e2e.test.js
```

The browser-required flag prevents missing Playwright/Chromium from silently counting as success. Other unmatched test names may appear as filtered skips in Node's output; report the four selected outcomes separately. Do not infer the full plugin suite passed from this focused run.

## Evidence already received

`/tmp/otto-quality-20261007-native-lifecycle.log` was independently read. It reports a native 1000→666 CSS-pixel width change at 1.5×, both focus transfers and a final PASS. **One display** was present. Its geometry restoration checks passed on that display; this is not multi-monitor/unplug evidence. The old zoom section logs dimensions without asserting a ratio, which is why the full-SPA zoom assertions were strengthened.

The strengthened SPA and new plugin cases are pending coordinator execution at this document's initial checkpoint. No pass is claimed from source preparation.

## Remaining native limits

A successful full-SPA probe would establish current-source behavior in real Tauri/WKWebView and close the blanket “native zoom unexecuted” gap for its form/scale/continuity scenario. It would not be a packaged-release installation test, a general visual audit at every zoom level, native menu-key event delivery, or a VoiceOver review. DOM focus and bounds do not prove spoken announcements, rotor order or accessibility-tree quality. Global VoiceOver remains untouched; those observations stay explicitly unverified. The historical browser-only zoom test continues to be reported as browser evidence, not upgraded retroactively.

## Execution checkpoint and newly discovered focus cases

- The first strengthened native SPA run reached 200% with draft and dialog/field bounds preserved, but its focus assertion failed. The diagnostic rerun (`/tmp/otto-quality-20261007-native-spa-diagnostic.log`) separates OS/window focus from DOM focus: the document remains focused; the connected `nw-name` input blurs at CSS width 1000 and focus moves to another button. Two seconds of observation do not restore it. This is a real observed focus transfer, with cause still being traced; native zoom acceptance is not passing yet. Additional non-mutating diagnostics record the destination button/dialog and the JavaScript focus call stack.
- A separate focused browser regression, `quality: report opening owns focus while its HTML is still loading`, failed as expected in `/tmp/otto-quality-20261007-report-focus-red.log`. `components.js` attempted to focus the real report viewer's disabled Download button and left focus outside its dialog. A minimal repair now filters the initial target for enabled, visible, non-inert controls before falling back to the dialog. Verification is pending coordinator execution. This adds one regression to the original six-case set; no new broad sweep was introduced.

## Verified repair progress

- The report-opening regression and four final visual journeys passed **5/5** in `/tmp/otto-quality-20261007-plugin-focus-green.log`; all twenty resulting images were inspected and retained in `evidence/native-transient/`.
- The new browser breakpoint regression failed on the old Modal implementation and passed after wrapping its mount-only focus lifecycle in `untrack` (`/tmp/otto-quality-20261007-modal-resize-green.log`, 1/1). It waits for actual FloatingBar mount/unmount and two frames before checking unchanged focus, without refocusing.
- The exact native stack showed the old Modal autofocus and queued trigger restoration at width 1000. FloatingBar's synchronous focusin observer had contributed its reactive root element to the modal effect; removing the bar retriggered that effect. The fresh native run no longer shows this defect: `activeElement` stays `nw-name`, `focusCalls` is empty, and the draft and bounds remain correct. It nevertheless fails the unchanged strict focus assertion because the whole probe window and both documents become inactive during zoom. A failure-only read-only AppKit diagnostic now distinguishes the probe's activation from the current foreground application. No activation/refocus workaround was introduced, and that native run is still reported as failed pending diagnosis.

## Final outcome

The strict actual-SPA native rerun passed (`otto-quality-20261007-native-foreground.log`) at 100→200→190→100%, without refocusing or weakening assertions. The earlier whole-application activation loss remains unexplained, not reclassified as external. Browser resize 1/1 and nested parent fallback 2/2 passed; full plugin suite 487 passed/1 skipped/0 failed; strict native example Clippy passed. Logs and all twenty inspected plugin images are retained with SHA-256 provenance in `evidence/native-transient/`. Final design/UX rubric and scope limits are in `round-2-design-ux.md`: **9.7/10 each for the bounded reviewed scope**.
