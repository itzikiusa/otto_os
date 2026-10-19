# Insights UX follow-up

Root closed the two remaining Insights findings from design-grok 16. The clock
uses the shared visibility-aware `now()` clock and the recorded run start,
independent of pollCount. Restored/attached runs use their actual started_at;
pollCount remains only the attempt budget. Failed refreshes render shared
LoadState with Retry above retained reports.

Named browser verification: `desktop-insights-closure.spec.ts`, 2/2 passed in
11.1s (`/tmp/otto-root-insights-browser.log`). One case changes wall time by ten
minutes without advancing the polling timer and delivers visibility restoration;
the banner catches up. The other ends a fixture run, makes the subsequent report
refresh return503 while an old report is listed, verifies visible Retry with the
old list retained, and retries successfully. No real agent runs are started.

Prior unpublished-run and wrapping repairs remain intact. Usage chart findings
belong to Ronaldo's existing usage ownership and are not closed by this report.

## Combined current-daemon browser gate

Root ran all four named closure specs together against the rebuilt isolated
debug daemon (SHA256 `267ad63c7779c3e780be60ef29d45e613abb302566d2dfc839ed0fb191c3a38b`).
**14 passed, 0 failed in 27.3s**. Log: `/tmp/otto-root-ui-closure-browser-current.log`.
Unit and E2E TypeScript checks also passed. Full-product unit/Svelte/guard gates
remain open for the in-progress Product/Vault changes recorded in current-verification.md.

## Original inventory follow-up

Closed the remaining phone-loading split and undersized period filters:
InsightsPage hides the loading detail skeleton on phones, gives filter buttons
36px minimum height on phone/coarse pointers, and applies the same sizing to the
period selector at narrow desktop-browser widths. Named browser suite
`desktop-insights-closure.spec.ts`: **3 passed, zero failed**, including delayed
loading/list-only visibility, rendered filter heights and no horizontal overflow.
Log: /tmp/otto-root-insights-phone-final.log. UI guards passed after this edit.
