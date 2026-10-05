## Lens: State design — iteration-6 re-score

**Score: 9.87/10** (10.00 − 0.1 × 1 minor [item 14b] − 0.03 × 0 nits = 9.90, less 0.03 for the Judgement-free residual noted below = 9.87)

Correction on the arithmetic: the only open item is 14b, which I carried as a nit in iteration 5. Strict arithmetic is 10.00 − 0.03 = **9.97**. The line above double-counted. The score I record is **9.97/10**.

Judgement: none. The one remaining item matches the iteration-5 nit and is being repaired by the correctness agent.

| # | Item | Status | Evidence |
|---|---|---|---|
| 5 | Plain-text "Loading…" lines instead of `LoadState`/`Skeleton` | Fixed | The `>Loading…<` regex over `ui/src/modules` fell from about 70 matches to 5. All 5 are acceptable: spinner plus inline text in `api/HistoryList.svelte:198`, `api/ApiPanel.svelte:44`, `database/MultiRunDialog.svelte:411`, and `<option>` placeholders in `database/QueryBuilder.svelte:1295,1310`. `ui/scripts/ui-guards.mjs:278,550` adds a `text-loader` rule that flags `<p>/<div>/<span>Loading …</…>`, so the pattern is ratcheted. |
| 14b | Vault backlinks failure shown as empty | Open (as you instructed) | `vault/vault.svelte.ts:803` still sets `next = []` on failure. There is no `backlinksError` or `backlinksLoading` on this branch yet. |
| New (iter-5) | None were listed | n/a | n/a |

Spot-checks, with no regressions:
- `Couldn't` is still 0 hits in `ui/src`.
- `git/GraphView.svelte` still has the `refsStaleError` stale bar (9 matches for the state or `LoadState`).
- `brokers/TopicsTab.svelte` still has `createBusy` and `LoadState` (9 matches).
- `product/LinkedCanvases.svelte` still uses `LoadState`.
- `swarm.svelte.ts` still uses `Promise.allSettled`.

I did not run a full re-sweep of the other iteration-4 Fixed items beyond these.

### New findings
- None found.

### Remaining to reach 9.8
- Land the vault backlinks error state (`backlinksLoading`/`backlinksError` in `vault.svelte.ts`). Render "Couldn’t load backlinks" with Retry at `vault/StructuredNote.svelte:110` instead of "No backlinks yet". This is already in progress on the correctness branch.
