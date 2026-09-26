# UX and design audit — 25 September 2026

Status: four review rounds complete; final integration and delivery verification in progress. Scores are reviewer judgments supported by the linked evidence, not an automated quality guarantee. The target is at least 9.5 for each reviewed surface; known defects and untested main flows lower the score. No whole-app completion claim is made here.

The audit uses ten fresh reviewers per round, four rounds (the user made round4 final on26 September). Because only three reviewers can run concurrently, independent scopes are queued in batches; a later-round scope starts only after its preceding review and repairs finish. Each new reviewer verifies prior repairs and examines deeper loaded, error, keyboard, long-content, theme and device states. Browser tests use isolated daemon state and intercepted external services; production user data is not modified.

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

## Round 3

All ten fresh reviewers completed their repairs and scoped checks. The combined gate passed: full UI check with zero errors/warnings, 454 unit tests, and production build. This round found further terminal sizing, mutation ownership, startup draft, media-control and monitoring layout defects. At that checkpoint further rounds were planned; the user subsequently made round4 final.

| Completed scope | Verified dimensions / 10 | Evidence |
|---|---:|---|
| Settings, MCP, Plugins, Skills Lab | 9.1–9.5; per-family means below target | [Review](r3-settings.md) |
| Product, Vault, Canvas, Design, Reader, Snip | 9.3–9.5 on inspected variants; other variants explicitly unscored | [Review](r3-content.md) |
| Shell, Home, sessions and panels | 9.2–9.6 by dimension; Files/Notes and standalone bar need deeper coverage | [Review](r3-shell.md) |
| Automation and agent coordination | 9.1–9.5 by dimension; complex graph/coordination variants remain below target | [Review](r3-automation.md) |
| Shared accessibility, auth and guest sharing | 9.3–9.6 by inspected dimension; broader Modal variants are explicitly incomplete | [Review](r3-access.md) |
| Git, graph, PRs, Focus | 9.4–9.6 by dimension; active review flows remain unscored | [Review](r3-git.md) |
| Database, API, connections, brokers | 9.5 on verified main flows; network-profile composition remains 9.4 | [Review](r3-data.md) |
| AWS and Kubernetes | 9.3–9.5 by dimension; deeper fleet and cloud operations remain unscored | [Review](r3-cloud.md) |
| Assistant, History, Usage, Insights | 9.4–9.6 by dimension; History mean9.48, other reviewed families9.50–9.56 | [Review](r3-insights.md) |
| Help, walkthrough, guides, hotkeys and coach | 9.5–9.6 on individually inspected variants | [Review](r3-help.md) |

Content verification passed 73 scoped checks plus four diagnostic reload repetitions. One intermittent Product reload rejection did not reproduce with tracing and remains an explicit next-round investigation. This is not a clean-runtime claim. Mid-round UI checking passed with zero errors or warnings. The complete Rust rerun after Git normalization passed 3,563 tests with zero failures (66 ignored); the later Insights calendar extension separately passed12 focused tests. Full clippy passed with both Rust changes. Final gates will verify the final integrated revision. [Parent cross-checks](r3-parent.md) document a newly reproduced Canvas queue identity regression, its repair and both-engine validation, plus workflow response compatibility.

## Round 4 (final)

All ten fresh reviewers completed their repairs and scoped verification: forty fresh reviews across four rounds. No fifth round will run, per the latest user instruction. Final integration, CI and installation are tracked separately below.

| Scope | Verified dimensions / 10 | Evidence |
|---|---:|---|
| Settings, MCP, Plugins, Skills Lab |9.3–9.5; loaded Review9.5|[Review](r4-settings.md)|
| Automation and agent coordination |9.1–9.5 steady layouts; original breakpoint defect repaired and rechecked|[Review](r4-automation.md)|
| Shared accessibility, identity and onboarding |9.5–9.6 inspected dimensions|[Review](r4-access.md)|
| Shell, Home, terminal and panels |9.3–9.6; mobile terminal accessibility9.3|[Review](r4-shell.md)|
| Product, Vault, Canvas, Design, Reader, Snip |9.3–9.5; final tablet scene-width deduction repaired|[Review](r4-content.md)|
| Database, API, Kafka, connections and network profiles |9.5–9.6 on inspected flows|[Review](r4-data.md)|
| Git, graph, PRs and review |9.4–9.6; exact long-tab header9.5–9.6|[Review](r4-git.md)|
| AWS and Kubernetes |9.4–9.5; main repaired resource flows9.5|[Review](r4-cloud.md)|
| Assistant, History, Usage, Insights |9.5–9.6 on inspected flows; family means9.52–9.58|[Review](r4-insights.md)|
| Help, walkthrough, guides, hotkeys and coach |9.5–9.6 on inspected variants|[Review](r4-help.md)|

These ranges do not establish9.5 on every page or dimension. The reports distinguish actual design tradeoffs, unverified workflows and environment limits. Confirmed failures are repaired and rechecked before closure. [Parent cross-checks](r4-parent.md) include independent identity/save checks, production D2 under the desktop CSP, retained Swarm state, tablet Canvas and Product reload diagnosis.

## Integration and broad evidence

- Integrated `fix/git-tabs-use-free-width` through `997b9b5a`, including the delete preference repair, without editing its worktree.
- Twelve focused Git header/menu/delete-preference checks passed after integration.
- Expanded the shared page inventory from 21 to 33 routes, corrected obsolete Help/Plugins aliases, and passed 165 iPhone WebKit page/theme/RTL checks.
- Final UI type/style gate passed with zero errors/warnings; 454 unit tests, six deployment mock tests, production UI build and the full165-check phone inventory passed. The full Rust workspace suite passed3,565 tests, zero failures,66 ignored. Final-source Clippy, the final Insights prompt test and delivery outcomes are tracked on the combined PR.
- [First-run phone repair](onboarding.md).
- [Instrumental walkthrough and playback evidence](media.md).

Screenshots and precise local test output paths are linked in the scope reports. [Selected durable light/dark/phone evidence](evidence/README.md) accompanies the final PR. Actions, merge and installation verification follow the final integration gates.
