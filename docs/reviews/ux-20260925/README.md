# UX and design audit — 25 September 2026

Status: in progress. Scores are reviewer judgments supported by the linked evidence, not an automated quality guarantee. The target is at least 9.5 for each reviewed surface; known defects and untested main flows lower the score. No whole-app completion claim is made here.

The audit uses ten fresh reviewers per round, five mandatory rounds. Because only three reviewers can run concurrently, independent scopes are queued in batches; a later-round scope starts only after its preceding review and repairs finish. Each new reviewer verifies prior repairs and examines deeper loaded, error, keyboard, long-content, theme and device states. Browser tests use isolated daemon state and intercepted external services; production user data is not modified.

## Round 1

| Scope | Score / 10 | Evidence |
|---|---:|---|
| Git, graph, PRs, Focus | 7.94 | [Review](r1-git.md) |
| Shell, Home, dialogs | 7.94 baseline | [Review](r1-shell.md) |
| Database, API, connections, brokers | 7.90 baseline | [Review](r1-data.md) |
| Product, Vault, Canvas, Design, Browser | 8.72 | [Review](r1-content.md) |
| Automation and agent coordination | 7.82 | [Review](r1-automation.md) |
| Settings, MCP, Plugins, Skills Lab | 9.05 | [Review](r1-settings.md) |
| AWS and Kubernetes | 8.47 | [Review](r1-cloud.md) |
| Assistant, History, Usage, Insights | 8.90 | [Review](r1-insights.md) |
| Help and walkthrough player | 9.40 | [Review](r1-help.md) |
| Shared accessibility and variants | 9.2–9.6 by surface | [Review](r1-access.md) |

Per-page ratings and exact coverage limits are in each report. A group mean does not establish that every page meets the target. All documented feasible residuals are assigned to the next fresh review.

## Round 2

All ten fresh reviewers completed their repairs and verification. The combined round gate passed: zero type/style errors or warnings, 454 unit tests, and the production build.

| Scope | Current verified range / 10 | Evidence |
|---|---:|---|
| Git, graph, PRs, Focus | 9.4–9.5 by dimension | [Review](r2-git.md) |
| Database, API, connections, brokers | 9.3–9.5 by family | [Review](r2-data.md) |
| Product, Vault, Canvas, Design, Browser | 9.0–9.5 by dimension | [Review](r2-content.md) |
| Automation and agent coordination | 9.0–9.3 by family | [Review](r2-automation.md) |
| Shared accessibility | 9.5–9.6 by dimension | [Review](r2-access.md) |
| Settings, MCP, Plugins, Skills Lab | 9.16–9.44 by family | [Review](r2-settings.md) |
| Shell, Home, sessions and panels | 9.3–9.6 by family | [Review](r2-shell.md) |
| AWS and Kubernetes | 9.38–9.50 by family | [Review](r2-cloud.md) |
| Assistant, History, Usage, Insights | 9.44–9.52 by family | [Review](r2-insights.md) |
| Help and walkthrough player | 9.54–9.56 by variant | [Review](r2-help.md) |

Ratings above describe tested browser behavior, not a guarantee of every application path. The broader session test run exposed a real terminal selection loss during resize compaction; the shell reviewer repaired and rechecked it, including delayed responses and native clipboard fallback. Production-build LSP and nested-focus checks passed in both Chromium and WebKit (6 tests). One confirmed terminal readability issue remains assigned to round three: Split view can shrink text below 11px.

## Round 3 (in progress)

Three fresh reviewers have started: shell, content and settings. This round includes terminal sizing, save races, installed plugin routes, Snip editor, auth/share boot states, desktop auxiliary surfaces and complete onboarding flows. Ten fresh reviewers will run before the combined round gate. Rounds four and five are also mandatory.

## Integration and broad evidence

- Integrated `fix/git-tabs-use-free-width` through `997b9b5a`, including the delete preference repair, without editing its worktree.
- Twelve focused Git header/menu/delete-preference checks passed after integration.
- Expanded the shared page inventory from 21 to 33 routes, corrected obsolete Help/Plugins aliases, and passed 165 iPhone WebKit page/theme/RTL checks.
- UI type/style gate passed mid-round with zero errors/warnings; 454 unit tests and the production UI build passed. Later modifications require another final gate.
- [First-run phone repair](onboarding.md).
- [Instrumental walkthrough and playback evidence](media.md).

Screenshots and precise local test output paths are linked in the scope reports. Selected durable light/dark/phone screenshots will accompany the final PR. Merge, Actions, and installation verification are pending until the review and final gates finish.
