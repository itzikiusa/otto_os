## Lens: State design — iteration-5 re-score

**Score: 9.77/10** (10.0 − 0.1 × 2 minor − 0.03 × 1 nit = 9.77)

Judgement: finding 5 is only half-fixed, so I held it at its original minor severity as the rule says. The remaining plain-text loaders are low-impact: none of them replace a loaded list. I would call the real score about 9.8.

| # | Item (from the iteration-4 re-score) | Status | Evidence |
|---|---|---|---|
| 5 | Plain-text "Loading…" lines instead of `LoadState`/`Skeleton` | Partial | The four sites I named are fixed: `mcp/ToolsTab.svelte` has no "Loading" match, and `AttachProductStory.svelte`, `PodHttpPanel.svelte` and `StoryLinkCard.svelte` no longer appear in the regex output. The same `>Loading…<` regex still matches about 70 lines. Examples: `product/OverviewTab.svelte:1156,1330,1451,1759,2000`, `product/PublishDialog.svelte:235,270,294,318,372`, `database/QueryBuilder.svelte:885,998`, `vault/RecoveryView.svelte:122,136`, `product/TestCasesTab.svelte:1052`, `agents/history/HistoryPage.svelte:479`, `insights/InsightsPage.svelte:643`, `skills-lab/SkillsBrowser.svelte:351`. The AWS views are acceptable (status line plus `Skeleton`). The new guard `bare-loading` (`ui/scripts/ui-guards.mjs:254,498`) only catches a bare "Loading…" with no noun, so these named loaders pass it. |
| 11 | Git graph background refresh failures silent | Fixed | `git/GraphView.svelte:278-293` counts quiet-read failures. After two misses it sets `refsStaleError`. `GraphView.svelte:3610-3613` renders `LoadState` with Retry. The remaining `.catch(() => {})` hits (1118, 1552, 1855, 1921) are post-action `refreshAfter()` calls, which feed that counter. |
| 14b | Vault backlinks failure shown as empty | Open | `vault/vault.svelte.ts:800-804` still sets `next = []` on failure. `vault/StructuredNote.svelte:110` then shows "No backlinks yet". |
| New (iter-4) | AttachProductStory hand-rolled states | Fixed | `AttachProductStory.svelte` no longer shows up in the loading regex. I did not re-read the error path. |

Spot-checked as still fixed, with no regression:
- `Couldn't` count is 0 in `ui/src/*.{svelte,ts}`.
- `LinkedCanvases.svelte` still uses `LoadState`.
- `swarm.svelte.ts` still uses `Promise.allSettled`.
- `TopicsTab.svelte` still has `createBusy`.

### New findings
- None found.

### Remaining to reach 9.8
- Convert the remaining named plain-text loaders to `LoadState` or `Skeleton`. Start with the multi-state ones: `OverviewTab.svelte`, `PublishDialog.svelte`, `QueryBuilder.svelte`, `RecoveryView.svelte`, `TestCasesTab.svelte`, `HistoryPage.svelte`. Extend the `bare-loading` guard in `ui-guards.mjs` to flag `<p>/<div>Loading …</…>` text-only loaders.
- Track a backlinks error in `vault.svelte.ts:800-804`. Have `StructuredNote.svelte:110` show "Couldn’t load backlinks" with Retry instead of "No backlinks yet".
