# Iteration 07 — root L21–L24: layout and browser regression

Re-read Modal flex sizing, scrollable body, focus semantics and title association;
NewSession's optional height changes no other dialog default. Re-executed the
named browser spec in isolated slot 8 with the workspace debug daemon: four cases
passed at 1440x1200 and 390x844, light and dark. Each switches all available
providers, compares all four bounding-box coordinates, verifies footer visibility
and scrolls to the final field. This is Chromium browser evidence, not a claim
that a packaged WKWebView test was run.

| Lens | Score | Evidence |
|---|---:|---|
| L21 routing | 9.6 | Current UI unit suite: 1996 passed, including script routing cases. |
| L22 dialog | 9.8 | Four browser cases passed; viewport-based cap retained without vh height. |
| L23 guidance | 9.7 | UI type check and guards passed with zero warnings/errors. |
| L24 release | 9.2 | UI production build passed; Rust/CI/install still pending. |

Logs: /tmp/otto-parity-final-sizing.log, /tmp/otto-parity-final-ui-check.log,
/tmp/otto-parity-final-ui-unit.log, /tmp/otto-parity-final-ui-build.log.
