# Round 4 — parent cross-checks

## API initial-load draft ownership: suspected issue rejected

Round3's `openApiEditor` helper waits for saved-request discovery because an untouched editor can become the empty-workspace onboarding view. That timing fix must not conceal draft loss. A new independent browser regression holds the real workspace's requests-list GET, types an actual URL into the visible editor, releases the empty response, and verifies that the URL remains and onboarding does not replace it. **Both Chromium and iPhone WebKit passed**. The existing `isDirty` condition protects the accepted draft; no production patch was justified. This differs from the separately reproduced/repaired first-run coach lifecycle defect.

From `ui/`: `OTTO_E2E_SLOT=ux4parent OTTO_E2E_PORT=7871 OTTO_E2E_PW_PORT=5371 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod npx playwright test e2e/desktop-ux-r4-parent-api.spec.ts --project=desktop-browser --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-r4-parent-api-results`. Log `/tmp/otto-ux-r4-parent-api.log`:2passed25.8s, exit0. No outgoing API request is sent; fixture URL is `fixture.invalid`. All daemon writes are the isolated workspace fixture.
