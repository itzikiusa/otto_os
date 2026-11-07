# Focus and mobile review evidence

Personal Agents uses one Focus conversation with an agent switcher, latest result,
and contextual routine/history/settings sheets. Threads keeps drafts and pending
work tied to their session. Mobile navigation preserves desktop preferences without
opening overlays automatically; terminals distinguish taps from scrolling. Existing
coding-agent terminals can move to Threads and back without spawning a second session.

Connections terminals now keep usable phone height. Vault shows note-load Retry
while browsing and opens Note details on request. Product uses Back navigation and
a phone disclosure for story metadata.

## Screenshots

Screenshots use isolated test data from the running implementation.

| View | Light | Dark |
|---|---|---|
| Personal Agents Focus | [Light](focus-light.png) | [Dark](focus-dark.png) |
| Product phone | [Light](product-phone-light.png) | [Dark](product-phone-dark.png) |

[Connections terminal](connections-phone.png) · [Vault Note details](vault-phone.png)

## Verification

- `scripts/check.sh --check`: 2,370 unit tests, zero type-check errors/warnings,
  production build, source guards and bundle budgets passed.
- Threads/terminal/handoff scoped browser checks: 26 passing checks across the
  combined run and affected-file rerun; one existing WebKit keyboard test skipped.
- Expanded Connections/Vault/Product suite: 65 passed, eight expected opposite-layout
  skips. Personal Agents: seven passed. These include iPhone WebKit portrait/landscape,
  Android Chromium touch emulation and desktop Chromium regression checks.
- Light/dark and terminal-output rerun: ten passed. Final styled Vault Retry: three passed.
- Real local shell and Redis fixtures; provider/Jira operations use deterministic fixtures.
  Physical iPhone keyboard behavior, full-screen CLI gestures and actual Tailscale
  latency remain device checks. No measured usability score is claimed.
