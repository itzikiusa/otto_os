## Lens: Page layout & toolbar consistency — iteration-5 re-score

**Score: 9.60/10** (10.00 − 4 minors × 0.10 = 9.60: three Partial and one Open from iteration 4, no new findings).

Judgement: I agree with the arithmetic. The two Partial items that remain are intentional exceptions (the Settings nav and Database's wider default), but neither is documented as one. Documenting them would remove them from the count.

| # | Iteration-4 item | Status | Evidence |
|---|---|---|---|
| 4 | Resizers bypass `PaneDivider`/`LIST_PANE` (minor) | Partial | Fixed: Mission Control now uses `PaneDivider` (`MissionControlPage.svelte:5,383`) and has no `startDetailResize` or `paneResizer` use left. Still open: `DatabasePage.svelte:1066` passes `defaultWidth={SIDE_DEFAULT}`, and `:827` sets `SIDE_DEFAULT = 300` against the 280 `LIST_PANE` default (the 220 min override is gone). |
| 5 | Fixed-width list panes (minor) | Partial | Fixed: `aws/LogsView.svelte:530` now uses `PaneDivider`. Still fixed: `settings/AccessGroups.svelte:492` (`minmax(180px, 240px)`) and the `settings/Settings.svelte:323` nav (200px). |
| 8 | Header buttons mix `.btn` and `.btn.small` (minor) | Partial | Fixed (now `.small`): `VaultPage:304`, `AwsPage:196`, `ProductPage:675`, `CanvasPage:195` and `DatabasePage:973`. Still plain `.btn` in header actions: `Walkthroughs.svelte:217`, `PersonalAgentsPage.svelte:161` and `settings/McpServers.svelte:273`. `PageHeader.svelte:574-580` still applies padding only to `.btn.small`. |
| 10 | Git header over five controls (minor) | Open | `GitToolbar.svelte:254-351` is unchanged. It still has Fetch, Pull and its caret, Push, Branch & stash (`data-keep`) and the ⋯ button. |
| 12 | "Pick a …" panes (minor) | Fixed | Neither "Pick a queue" nor "Pick a cluster" remains in `SqsView`/`BrokersPage`. Both files use `initialSelection` (`SqsView.svelte:73`, `BrokersPage.svelte:112`). |
| 13 | Inconsistent list-pane collapse control (minor) | Fixed | Brokers has a header `sidebar` icon at `BrokersPage.svelte:336-345` (`aria-expanded`, `aria-controls="brokers-cluster-list"`). Database has one at `DatabasePage.svelte:955-961`. Skills Lab (`SkillsLabPage:92`) and Canvas (`CanvasPage:188`) already used it. |

Spot-checks of items I previously marked Fixed:
- The `PaneDivider` imports remain on Workflows, Assistant, Walkthroughs, Workbench, RoomsView and ContextLibrary (I checked the file list in iteration 4 and re-checked Mission Control and LogsView now).
- The Brokers `titleContent` duplicate is still gone. In `BrokersPage.svelte:334-378` the leading, badge and actions snippets all remain without a repeated title.
- I did not re-read the others (PrDetail, AWS, MCP, Product order, Vault max). Nothing in the edited files suggested a regression.

### New findings
None. The Brokers header now has a sidebar toggle and a `content-toggle` chevron both in `leading`. This is slightly busy but within the budget, so I am not counting it.

### Remaining to reach 9.8
- Cut the Git repo header to five controls. For example, fold Fetch into the Pull menu, or move Fetch/Pull/Push into the pane toolbar.
- Convert the last plain `.btn` header controls (`Walkthroughs:217`, `PersonalAgentsPage:161`, `McpServers:273`) to `.small`. Alternatively, apply the same padding to plain `.btn` in `PageHeader.svelte:574`.
- Either give `AccessGroups` the resizer, or record it and the 200px Settings nav as documented fixed-width exceptions in layout.md.
- Switch Database to `LIST_PANE.default` (280). Otherwise document 300 as an explicit exception.
