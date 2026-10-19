# Proof, Run with Otto and review UX — closure implementation

Root owns this follow-up to design-grok reports 08 and 14. Existing Ronaldo and
Pele sessions remain the only delegated sessions. No backend permissions changed.

Implemented:
- Phone Proof list occupies the page; open detail retains Back. Loading/error
  states remain reachable. Partial pagination does not advertise workspace totals.
- Done/risk scores have visible labels. Artifact titles wrap; filters, artifact
  controls, run source chips and proof links have 36px phone/coarse-pointer hits.
- Run with Otto uses phone Back/list-detail navigation, retaining the hidden
  launcher's draft rather than destroying/recreating it.
- Review launch controls now precede the primary action in normal flow, including
  cancelled/error retry states. PR/local review commands register with the palette.
- Failed merge-readiness loads show Retry. Older overlapping responses cannot
  overwrite newer readiness or another review. CI labels use readable states.
- Proof modal load failures offer Retry and disable Export. Load/export generation
  guards prevent an obsolete response appearing in a different review's modal.
- Waive/false-positive ask for a reason and submit it to existing audit APIs;
  cancellation sends nothing. Switching finding during the prompt aborts the action.
- Finding titles take a full line; paths wrap. Local review finding text is no
  longer inside the checkbox label, so selecting text does not toggle selection.
- Existing local review Stop/cancelled/provenance repairs are preserved.

Actual verification so far:
- `npm run check`: passed, Svelte 0 errors/0 warnings, UI guards and TS configs.
- `node --test unit/reviewClosure.test.ts unit/proofOwnership.test.ts
  unit/runLauncherOwnership.test.ts`: 12 passed, zero failed/skipped.
- Named Playwright `desktop-review-phone-closure.spec.ts`: first three cases
  passed (5.7s), phone Proof light/dark and Run Back/draft retention/hit sizes.
  Uses isolated daemon and route fixtures for run data. Current UI tested against
  existing debug daemon; this is not an installed-app/backend release validation.
- Full UI units: 2046 passed, one failed in Ronaldo's in-flight Canvas ownership
  regression (late A save/finish into B). Sent exact failure to owner. Not a green
  full UI gate. Further modal/reason browser case currently pending.

No 9.8/full-product completion claim. Existing 165-open crosscheck is an earlier
snapshot; other modules still need implementation and evidence-based closure.

Additional mounted-browser case passed (1/1, 5.9s): real isolated finding API
remains open on prompt Cancel, stores the supplied waive reason in its audit
events, and proof modal disables Export during load failure and recovers via
Retry. Initial fixture mistakenly opened clean-slate local review instead of the
existing review deep link; corrected navigation, preserving all assertions.

Corrected scheme verification: fixtures now set `otto_scheme`, not the theme
name key. Combined named Proof/Design browser run passed 7/7 in 13.2s
(`/tmp/otto-root-design-review-browser.log`), including both actual light/dark
Proof cases and the mounted reason/proof-retry case. Prior theme-labelled runs
did not establish dark-scheme coverage.

## Combined current-daemon browser gate

Root ran all four named closure specs together against the rebuilt isolated
debug daemon (SHA256 `267ad63c7779c3e780be60ef29d45e613abb302566d2dfc839ed0fb191c3a38b`).
**14 passed, 0 failed in 27.3s**. Log: `/tmp/otto-root-ui-closure-browser-current.log`.
Unit and E2E TypeScript checks also passed. Full-product unit/Svelte/guard gates
remain open for the in-progress Product/Vault changes recorded in current-verification.md.
