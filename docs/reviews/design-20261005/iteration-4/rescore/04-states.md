## Lens: State design — re-score

**Score: 9.2/10** (10.0 − 0.1 × 6 minor (0.6) − 0.03 × 1 nit (0.03) = 9.37, rounded down to 9.2 because the plain-text loader pattern is still widespread). I held the loader finding at minor under your rule that a Partial counts at its original severity, so this is a judgement call. Strict arithmetic gives 9.4. The remaining items are finding 5 (Partial), 11 and 14b (Open), plus the new findings below.

### Previous findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | Linked Canvases failure shown as empty | Fixed | `product/LinkedCanvases.svelte:26-36` keeps the last good list and sets `loadError` on failure. `LoadState` is imported at line 13. |
| 2 | Bulk swarm ops swallow failures | Fixed | `lib/stores/swarm.svelte.ts:612-628` uses `Promise.allSettled` and returns `BulkTaskResult`. `swarm/KanbanBoard.svelte:186-204` toasts "Couldn’t move N of M tasks" and re-selects the failed ones. |
| 3 | Brokers tabs blank the list on reload | Fixed | `TopicsTab.svelte:324`, `GroupsTab.svelte:310,331-337`, `LagAlertsPanel.svelte:101`, `SchemaTab.svelte:78` and `SchemaVersionsPanel.svelte:128,163` all use `LoadState` now. |
| 4 | Straight "Couldn't" (about 217 hits) | Fixed | The only remaining hit in `ui/src` is `help/sections/insights.md`, a doc file, not UI. |
| 5 | About 40 plain-text "Loading…" lines | Partial | The Brokers, panels, `CanvasPanel`, `OutputsPanel` and `FileTree` loaders moved to `LoadState`. The count of `>Loading…<` lines is down from 122 to 88 (rough regex count, includes button and busy labels). Hand-rolled ones still verified: `mcp/ToolsTab.svelte:197`, `kubernetes/PodHttpPanel.svelte:219`, `agents/AttachProductStory.svelte:84`, `swarm/StoryLinkCard.svelte:72`. The AWS views use a status line plus `Skeleton`, which is acceptable. |
| 6 | Inspector pickers: failure shown as "none yet" | Fixed | `design-hall/site/Inspector.svelte:178-188` and `233-243` show a disabled "Couldn’t load …" row with a "Try again" action. |
| 7 | SkillReviewPanel silently empty | Fixed | `skills-lab/SkillReviewPanel.svelte:90-107` uses `allSettled` and tracks `skillsFailed`. The panel shows "<sources> skills didn’t load" at line 386-389. |
| 8 | CanvasPanel and bespoke error blocks | Fixed | `CanvasPanel.svelte:255`, `ActivityPanel.svelte:193`, `OutputsPanel.svelte:286,345` and `FileTree.svelte:338,343` use `LoadState`. `AttachProductStory.svelte` is still hand-rolled, which is why finding 5 stays Partial. |
| 9 | "Create topic" has no in-flight state | Fixed | `brokers/TopicsTab.svelte:47,240-251,314-317` adds `createBusy`, a "Creating…" label, and disables Cancel too. |
| 10 | Delete icon buttons stay enabled in flight | Fixed | `SessionNames.svelte:177-179`, `Channels.svelte:247-264,446-452` and `IssueAccounts.svelte:177-192,270-276` use a per-row busy flag, `aria-busy` and a spinner. |
| 11 | Git graph stale refresh not signalled | Open | `git/GraphView.svelte:690-703,780,798` still swallow with `.catch(() => {})`. There is no stale marker. |
| 12 | Palette data failures drop commands | Fixed | `lib/components/FloatingBar.svelte:178-185` sets `failed` and adds a disabled row "Couldn’t load sessions and repositories". |
| 13 | Snip shortcut has no in-flight state | Fixed | `settings/SnipSettings.svelte:47-58,138-147` has a `saving` state with "Saving…", "Resetting…" and "Turning off…". |
| 14a | TokensPanel users failure silent | Fixed | `mcp/TokensPanel.svelte:52-63` keeps `usersError` and skips it for 403. |
| 14b | Vault backlinks failure becomes `[]` | Open (unverified) | `vault.svelte.ts` `reloadBacklinks` still sets `next = []` on failure. I did not confirm whether the UI says "No backlinks". |

### New findings

- `[nit]` `AttachProductStory.svelte:83-89` mixes a hand-rolled loading line, a plain error with Retry, and an empty message inside a `Modal`. It is consistent with itself but not with `LoadState`. This is the same pattern as finding 5, so I am not counting it again.
- No regressions found. In the files I changed or read, `LoadState` is used correctly: error with data gives the stale bar, and `loading` is passed through.

### Remaining to reach 9.8
- Move the remaining hand-rolled loaders to `LoadState` (`ToolsTab.svelte:197`, `PodHttpPanel.svelte:219`, `AttachProductStory.svelte:84`, `StoryLinkCard.svelte:72`, plus the other `>Loading…<` lines). Add a ui-guards ratchet for `>Loading…<`.
- Show a stale or "last refresh failed" indicator when Git graph background refreshes fail repeatedly (`GraphView.svelte:690-703,780,798`).
- Confirm the vault backlinks failure path, and show an error or retry if the UI currently reads "No backlinks".
- Add a guard for `catch { x = [] }` where the result feeds an empty message, so finding-1-style regressions cannot return.
