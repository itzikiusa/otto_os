# Design Hall and Canvas — implementation and verification follow-up

Root took component UI ownership; Ronaldo retains Canvas store identity and
panel lazy imports. No new agents or backend permission changes.

Implemented against design-grok reports 06 and 12:
- Canvas phone layout no longer forces read-only editors. Phone title keeps the
  scene name without consuming space on a breadcrumb. Code/Sketch and diagram
  controls have 36px touch targets. Shared PointerZoom handles two-finger scale
  around the moving midpoint, single-pointer pan, pointer cancellation and bounds.
- Assistant controls wrap; the phone Close stays beside the title, above controls.
- Site Studio narrow layouts select Page, Blocks or Design rather than stacking
  an inspector ahead of the library. Checks opens Design. Publish is secondary.
  The section toolbar wraps within stage bounds and uses its measured height
  when clamping placement; touch controls are 36px.
- Narrow 3D studios expose the previously hidden scene tools through Scene.
- Version chips scroll into view in RTL, keyboard direction follows RTL, and
  touch chips are 36px. Phone artifact badges keep only status.
- Brand/learning headers have phone Back, brand forms align to the page, presets
  are touch-sized, title filters use automatic direction, and shortcut metadata
  no longer changes control names. Brand Back goes to Design Hall because the
  brand landing route automatically reopens the latest kit.
- Lobby has one primary, readable search failure copy and touch-sized controls.
  Variant cards retain usable controls; spatial artwork has visible captions.

Actual verification:
- 28 focused Canvas ownership/recovery/banner/pointer tests pass. New pointer
  tests were first run red before the implementation existed; midpoint and
  remaining-pointer behavior are asserted numerically.
- Named phone Canvas light/dark source edits persist through the real isolated
  API; Site panel navigation succeeds. Combined Design/Proof browser run: 7/7,
  13.2s, `/tmp/otto-root-design-review-browser.log`.
- Fixtures use `otto_scheme`; actual light/dark screenshots were inspected and
  retained under screenshots/canvas-phone-{light,dark}.png. Earlier fixtures
  used the wrong preference key and do not establish dark coverage.
- Latest additional Brand and 3D browser cases are pending. First Brand test
  exposed a real Back-to-auto-redirect loop; fixed by returning to Design Hall.
  First 3D fixture ambiguously selected two Scene buttons; scoped to aria-controls.
- npm check passed after initial components and pointer integration (0/0); final
  check after header and toolbar follow-ups is pending.

Boundaries: browser uses current Vite UI with existing debug daemon, not an
installed release. Real-device multi-touch and all desktop/tablet combinations
have not been exercised. No full-product or 9.8 completion claim yet.

Final named browser verification: `desktop-design-phone-closure.spec.ts` 5/5
passed in 10.4s. Includes real phone source save in both schemes, assistant Close
first-row/viewport checks, Site panels and section-toolbar bounds/touch size,
Brand Back, and 1024px 3D Scene controls. Toolbar width is explicit max-content
within the cap; this removed a ResizeObserver feedback loop seen during the first
selection test. Separate toolbar rerun passed without that console error.
Full UI units before the subsequent API/browser group: 2049/2049, zero skips
(`/tmp/otto-root-ui-final-units.log`).

## Combined current-daemon browser gate

Root ran all four named closure specs together against the rebuilt isolated
debug daemon (SHA256 `267ad63c7779c3e780be60ef29d45e613abb302566d2dfc839ed0fb191c3a38b`).
**14 passed, 0 failed in 27.3s**. Log: `/tmp/otto-root-ui-closure-browser-current.log`.
Unit and E2E TypeScript checks also passed. Full-product unit/Svelte/guard gates
remain open for the in-progress Product/Vault changes recorded in current-verification.md.
