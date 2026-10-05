Documented exceptions in layout.md §342-344 and a code comment. I'm counting these as fixed under the rubric's exception route.

## Lens: Page layout & toolbar consistency — iteration-6 re-score

**Score: 10.00/10** (10.00 − 0 = 10.00). All four iteration-5 items are Fixed.

Judgement: the arithmetic gives 10.00. The two exception items (Database 300 px default and the fixed-width Settings and AccessGroups panes) are closed by documentation rather than by code changes. A stricter reviewer could keep them at 0.1 each (9.80). I treat them as resolved because the guideline now states the exception and the reason.

| # | Iteration-5 item | Status | Evidence |
|---|---|---|---|
| 4 | Database default 300 vs 280 (minor, Partial) | Fixed (documented exception) | `layout.md:344` lists the 300 px default (`SIDE_DEFAULT`) as a resizable exception. `DatabasePage.svelte:824-828` carries a matching comment. `:1066` still uses `PaneDivider`. |
| 5 | Fixed-width AccessGroups and Settings nav (minor, Partial) | Fixed (documented exception) | `layout.md:342` covers the Settings nav (200 px) and `:343` covers AccessGroups (`minmax(180px, 240px)`). Both match the code (`Settings.svelte:323`, `AccessGroups.svelte:492`). |
| 8 | Plain `.btn` in header actions (minor, Partial) | Fixed | `Walkthroughs.svelte:217`, `PersonalAgentsPage.svelte:161` and `McpServers.svelte:273` are now `btn small primary`. `PageHeader.svelte:574-577` also pins height, padding and font size for plain `.btn`, so it cannot drift. The remaining plain `.btn primary` hits (`VaultPage:533`, `McpServers:478`) are in modals, not headers. |
| 10 | Git header over five controls (minor, Open) | Fixed | `GitToolbar.svelte:253` renders Fetch only when `!inHeader`. In the header the controls are the Pull button plus its caret (`:262-285`, with "Fetch and pull options" in the menu), Push (`:288`), Branch & stash (`:312`, `data-keep`) and the ⋯ button. That is five controls, and the branch chip is not a control. |

Spot-checks of earlier Fixed items: Mission Control (`PaneDivider`), Brokers and Database header sidebar toggles, PrDetail, Product order, and `initialSelection` on Sqs and Brokers showed no regressions in the earlier grep results. I did not re-read them in this pass. I re-read only the Git, Database, Settings and button-size code above.

### New findings
None.

### Remaining to reach 9.8
- Nothing blocks 9.8. The lens is at or above the target on the fixed rubric.
- Optional hardening: add a ui-guards ratchet for header `.btn` without `.small`, and one for `PageHeader icon=`, so the converted pages stay converted.
