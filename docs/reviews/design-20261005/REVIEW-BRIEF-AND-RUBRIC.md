You are one of 10 independent DESIGN reviewers of Otto, a macOS desktop app (Tauri + Svelte 5 UI). Repo (read-only for you): /Users/itziklavon/claude_ade-design. UI code: ui/src (modules in ui/src/modules/*, shared components in ui/src/lib/components, tokens in ui/src/lib/tokens.css, shell in ui/src/shell + ui/src/App.svelte, sidebar registry ui/src/lib/sidebar.ts).
The binding design rules are docs/design/guidelines/*.md (README, foundations, layout, components, patterns, content, accessibility, review-checklist) plus the "Design guidelines (UI)" section of AGENTS.md. Read the guideline files relevant to your lens FIRST, then audit the code against them.

Brief from the user: "design wise, no small thing" — be exhaustive and strict; small inconsistencies count. Target bar is 9.8/10.

Rules:
- Do NOT edit any file. Do not run builds. Read and grep only.
- Every finding must be verified in the code (cite file:line you actually read). No speculative findings; if unsure, say so or skip.
- Prefer findings that are concrete and fixable in code (a specific file + a specific change). Group repeated instances of one pattern into one finding with all locations listed (up to ~15 locations, then "+N more" with the grep that finds them).
- Severity: blocker (breaks a must-not-break rule or makes UI unusable/unreadable), major (clearly visible inconsistency or broken state), minor (small visible inconsistency), nit.

Output (your final message, plain markdown, this exact shape):
## Lens: <your lens>
**Score: X.X/10** — one-line justification (what keeps it from 9.8)
### Findings
For each: `[severity] short title` — locations (file:line, ...) — what is wrong (rule cited) — concrete fix.
Order most severe first. Aim for 15–35 findings; quality over volume.
### What would get this lens to 9.8
3–6 bullets.

ITERATION 4 NOTE: review /Users/itziklavon/claude_ade-design (branch fix/design-iter-4 = main 03f2bc3e). It contains three previous design passes: tokens/ratchets sweep (ui/scripts/ui-guards.mjs rules), shared components (ApprovalActions/ApprovalOutcome, confirmProd/confirmOutward, PaneDivider/LIST_PANE, PageBody on split views, lib/tabKeys, LoadState/Skeleton grace, toastError, lib/labels, plural, navBadge, goToEntries, Switch, AgentChip, LiveWorkingDot, Drawer with title), Composer/History/DB fixes, copy sweep (Couldn’t-titles, US spelling). Verify relevant ones (credit them; flag regressions), then report what REMAINS.

SCORING RUBRIC (use exactly, so iterations are comparable): start at 10.0; subtract 1.0 per blocker, 0.3 per major, 0.1 per minor, 0.03 per nit; a systemic pattern counts ONCE (list all its locations in that one finding); floor 0. Show the arithmetic on the score line. Report at most 25 findings, most important first.
