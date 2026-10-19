# API and Browser — UX closure follow-up

Root took remaining presentation work from design-grok 09/15, preserving E08
functional repairs and Ronaldo's banner regression. No backend access changes.

Implemented: one primary on request views; named close/tab edit/outcome states;
36px request and browser controls with matching virtual-row estimates; phone
response reveal and timeout-settings navigation; failed Git pull stays open with
Retry; reachable Site credentials modal; reader article h2; summary engine,
time and degraded-fetch disclosure; responsive URL toolbar based on available
container width; mark deletion and send previews; send names the target session.

Two review suggestions were deliberately implemented without extra approval
modals: TLS-off shows persistent explicit certificate-trust copy, and automation
Run shows request count, environment and destination effects inline. The existing
explicit toggle/Run remain usable, consistent with the user's instruction to
avoid unnecessary security friction. These are alternative UX resolutions, not
claims that the proposed modal gates were added.

Verification so far:
- 11 targeted actual-handler/store tests pass: Git pull error/retry, mark cancel/
  send, browser ownership and retained/cleared summary provenance.
- Named browser `desktop-api-browser-closure.spec.ts`: phone tab labels and
  response reveal, credentials reachability and degraded summary, failed pull
  and retry all pass (2/2 in 5.9s; pull case 1/1 in 6.3s).
- Existing browser-reader send-to-session case updated for the preview; 1/1
  passes in 4.7s, real isolated mark/session route.
- Svelte check was 0/0 before the final row/stream/copy additions. Full check
  currently stops on Ronaldo's in-flight AttachmentsPanel retry guard hits,
  reported to its owner; no baseline change. Final check still required.

Browser tests use current Vite UI with an existing isolated debug daemon.
No full-product/9.8 signoff yet. Settings integrations and usage/insights remain
separate report surfaces; current API/browser changes do not close those by fiat.

## Combined current-daemon browser gate

Root ran all four named closure specs together against the rebuilt isolated
debug daemon (SHA256 `267ad63c7779c3e780be60ef29d45e613abb302566d2dfc839ed0fb191c3a38b`).
**14 passed, 0 failed in 27.3s**. Log: `/tmp/otto-root-ui-closure-browser-current.log`.
Unit and E2E TypeScript checks also passed. Full-product unit/Svelte/guard gates
remain open for the in-progress Product/Vault changes recorded in current-verification.md.

## Bundle follow-up

The production build exposed growth from the newly reachable credentials editor.
`BrowserView` now loads that editor only when Site credentials opens, with shared
loading/error/Retry UI. Rebuild passed; Browser page is 78.4 KB against its 76.6 KB
baseline and Browser panel is 51.3 KB against 50.1 KB, both within the existing
gate tolerance. No budget file changed. The only remaining bundle failure in
that run was the separately owned Plugin page. The credentials browser case
passed again (1/1, 3.9s); direct Svelte check passed with zero errors/warnings.
Logs: `/tmp/otto-root-ui-bundle-closure-r2.log`,
`/tmp/otto-root-browser-credentials-lazy.log`, `/tmp/otto-root-ui-svelte-lazy.log`.
