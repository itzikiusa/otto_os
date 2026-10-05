## Lens: Page layout & toolbar consistency — re-score

**Score: 9.4/10** (10.0 − 6 minors × 0.1 = 9.4). Every major is fixed. Six minors remain: one open, five partial. No blockers, majors or nits remain, and I found no regressions.

| # | Previous finding | Status | Evidence |
|---|---|---|---|
| 1 | Workflows has no `PageBody` and a hand-rolled resizer (major) | Fixed | `WorkflowsPage.svelte:1894` has `<PageBody fill padded={false}>` and `:1976` has `PaneDivider` with `LIST_PANE`. The `paneResizer` uses left at `:2242` and `:3328` are the run-input dock and context panel, not the list pane. |
| 2 | PrDetail has a second title and tab row and no `PageBody` (major) | Fixed | `PrDetail.svelte:451-456` puts the PR title in the header, the tabs in `tabsPlacement="below"`, and #N and state in the badge. `PageBody` wraps the body at `:482`. |
| 3 | Skills Evaluator second tab row (major) | Fixed | `SkillsEvalPage.svelte` no longer has a `se-subhead` element in the markup. The Evaluator tabs are now in the header at `SkillsLabPage.svelte:114`, with `tabsPlacement` set to below. |
| 4 | Hand-rolled resizers bypass `PaneDivider` (minor) | Partial | Fixed: ApiPage now uses `PaneDivider` at `:322`. Still open: `MissionControlPage.svelte:435-436` has both `onpointerdown={startDetailResize}` and `use:paneResizer`. `DatabasePage.svelte:1052` still overrides to `min=220` and `defaultWidth=300`. |
| 5 | Fixed-width list panes with no resizer (minor) | Partial | Fixed: Assistant `:200`, Walkthroughs `:288`, ContextLibrary `:412`, Workbench `:458` and RoomsView `:235` now use `PaneDivider`. Still fixed: `aws/LogsView.svelte:743` (260px), `settings/AccessGroups.svelte:492` (180–240) and the `settings/Settings.svelte:323` nav (200px). |
| 6 | Assistant shows the thread list on every tab (minor) | Fixed | `AssistantPage.svelte:125` now has `showList = tab === 'chat' && …`. |
| 7 | Vault pane maximum above `LIST_PANE.max` (minor) | Fixed | `VaultPage.svelte:40-41` uses `LIST_PANE.max`. |
| 8 | Header buttons mix `.btn` and `.btn.small` (minor) | Open | Non-small primaries remain: `VaultPage:303`, `AwsPage:174`, `CanvasPage:194`, `DatabasePage:938`, `PersonalAgentsPage:147`, `ProductPage:675`, `McpServers:272` and `Walkthroughs:217`. |
| 9 | Product: Import after the primary (minor) | Fixed | `ProductPage.svelte:670` (Import) now comes before the primary at `:675`. |
| 10 | Git header over the five-control budget (minor) | Open | `GitToolbar.svelte:253-350` still has Fetch, Pull and its caret, Push and Branch & stash, plus the ⋯ button. |
| 11 | `icon=` on Workbench and Rooms headers (minor) | Fixed | The grep on workbench and rooms finds no `<PageHeader … icon=`. |
| 12 | "Pick a …" panes (minor) | Partial | Fixed: the SkillsBrowser pane is gone. Still present: `aws/SqsView.svelte:358` ("Pick a queue") and `brokers/BrokersPage.svelte:498` ("Pick a cluster"). |
| 13 | Inconsistent list-pane collapse affordance (minor) | Open | Brokers still uses accordion chevrons (`BrokersPage:320,358`). Database keeps its in-pane chevron and SCHEMA rail. Skills Lab and Canvas still use the header `sidebar` icon. |
| 14 | MCP gutter and clearance (minor) | Fixed | `McpPage.svelte:150` has `PageBody padded={section === 'otto'}`. `OttoServerHome.svelte:538` now says the gutter comes from `PageBody`. |
| 15 | AWS error state outside `PageBody`, and header filter (minor) | Fixed | `AwsPage.svelte:188-191` wraps the error in `PageBody`. The header "Filter accounts" input is gone. |
| 16 | Git landing duplicates "Add repository" (minor) | Fixed | `GitPage.svelte` has no header `btn primary` left. The only match is `:612`, further down the file. The `+` tab at `:332` remains. |
| 17 | Brokers `titleContent` duplicates the title (nit) | Fixed | `BrokersPage.svelte` has no `titleContent`. |
| 18 | Redundant `width="full"` on Usage (nit) | Fixed | `UsagePage.svelte` has no `width="full"`. |
| 19 | Providers header primary was a maintenance action (nit) | Fixed | The only primary left is the form submit at `Providers.svelte:716`. |

### New findings
None. I did not read `PluginFrame.svelte`'s layout beyond confirming it still has a bare `<PageHeader title={name}>` at line 160 with no `PageBody` grep match. It is not scored because I did not verify it renders a body.

### Remaining to reach 9.8
- Normalize header button size: make `PageHeader` actions use `.small` padding for plain `.btn` too, or convert the eight non-small primaries listed in row 8.
- Remove the duplicate drag handler in Mission Control (`:435`). Decide whether Database's 300/220 override stays and document it, and give the Logs/AccessGroups/Settings list panes the same resizer or a documented exception.
- Cut the Git repo header to five controls by folding Fetch into the Pull menu or moving Fetch/Pull/Push into the pane toolbar.
- Pick one list-pane collapse control (header `sidebar` icon) and use it on Brokers and Database.
- Auto-select a first item in `SqsView` and `BrokersPage` so the "Pick a queue" and "Pick a cluster" panes never show when items exist.
