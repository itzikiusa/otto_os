## Lens: Copy & content — iteration-5 re-score

**Score: 8.77/10.** Arithmetic: 10.00 − 0.60 (2 major, both Partial) − 0.60 (6 minor, all Partial) − 0.03 (1 nit, Open) = 8.77. No new findings.

Judgement: the arithmetic understates the progress. The remaining Partial items are small tails. The raw-body pattern went from 102 sites to 6. Straight contractions went from 54 attribute sites to 28 attributes in 23 files. The toast-title pattern is down to two ternary titles. Copy ratchets now exist for all of these patterns (`ui/scripts/ui-guards.mjs:246–253,491–497`), so I would call the lens about 9.2 in practice. Those ratchets are baseline debt trackers, so some leftovers sit inside the baseline.

All paths below are under `/Users/itziklavon/claude_ade-design5/ui/`.

| # | Finding (from the iteration-4 re-score) | Status | Evidence |
|---|---|---|---|
| 1 | [major] Toast titles used three patterns | **Partial** | A grep for `toasts.error('… failed` now returns 0 matches. A ratchet was added (`scripts/ui-guards.mjs:493`). One tail remains: the ternary titles 'Delete failed' / 'Archive failed' at `src/lib/stores/workspace.svelte.ts:1290`, which the regex misses. |
| 2 | [major] Raw exception text as toast body | **Partial** | Down from 102 to 6 sites, with a ratchet at `scripts/ui-guards.mjs:494`. Remaining: `src/modules/aws/S3Browser.svelte:419`, `src/modules/api/ResponseViewer.svelte:240`, `src/lib/stores/workspace.svelte.ts:1290`, `src/modules/agents/history/HistoryPage.svelte:165,213`, and `src/lib/components/ContextPacketDialog.svelte:99`. The last one passes the raw exception as the title, with no body. |
| 3 | [minor] UK spellings | **Partial** | Fixed: "Review cancelled." (`src/modules/git/ReviewPanel.svelte`), the colours strings in `src/modules/design-hall/ArtifactStage.svelte` and `src/modules/design-hall/model.ts`, and `src/lib/uiCommands.ts:359` (now 'Canceled'). Ratchet at `scripts/ui-guards.mjs:496`. Still open: `toasts.info('Write cancelled')` at `src/modules/database/EditFlow.svelte.ts:713`. |
| 4 | [minor] Straight apostrophes | **Partial** | Down from 54 to 28 attribute sites in 23 files. Examples: `src/modules/settings/ContextSoul.svelte` (3), `src/modules/swarm/KanbanBoard.svelte` (2), `src/modules/swarm/RunsList.svelte` (2), `src/modules/database/QueryBuilder.svelte` (2), `src/modules/kubernetes/NamespacePicker.svelte`, `src/modules/agents/conversation/Composer.svelte`, `src/modules/aws/AwsPage.svelte`. Ratchet at `scripts/ui-guards.mjs:497`. |
| 5 | [minor] "Reload" used for data | **Partial** | Fixed: memories and transcript. Still open: `src/modules/kubernetes/LogsView.svelte:310` (title "Reload", aria-label "Reload logs", a data refresh) and `src/modules/plugins/PluginFrame.svelte:165`. "Reload saved context" and "Reload saved access" (`src/modules/settings/ContextSoul.svelte:270`, `src/lib/components/ResourceAccess.svelte:85,192`) discard edits, which is acceptable. `src/modules/panels/BrowserPanel.svelte:522` is a browser reload and also acceptable. |
| 6 | [minor] Raw enums and kinds | **Partial** | Fixed: severity now goes through `severityLabel` in `src/modules/skills-eval/RunDetail.svelte:559`, `src/modules/skills-lab/SkillReviewPanel.svelte:478,516` and `src/modules/skills-lab/SkillReviewAgents.svelte:110`. Still open: `{f.severity}` at `src/modules/git/ReviewAgents.svelte:191`. |
| 7 | [minor] Instruction-style placeholders | **Fixed** | Grep for `placeholder="(Paste\|Write\|Type a file\|Add a)` returns 0. |
| 8 | [minor] Hand-rolled plurals | **Fixed** | Down from 232 to 4 sites (`src/modules/database/EditFlow.svelte.ts` ×1, `src/lib/stores/workspace.svelte.ts` ×3). Only `src/lib/plural.ts` defines `plural`. |
| 9 | [minor] Toast titles not verb-first | **Fixed** | `src/modules/swarm/RecruiterWizard.svelte:159–160` is now a warning titled "Hired 2 of 4", with a "Couldn’t hire the rest" body. The "unreachable" and "failed ·" titles are gone from `src/modules/kubernetes/ClustersOverview.svelte` and `src/lib/uiCommands/k8s.ts`. |
| 10 | [minor] Cryptic titles | **Partial** | `src/modules/snip/SnipEditor.svelte` and `src/modules/product/design/DesignArena.svelte` are fixed. Still open: 'No create statement' and 'Clipboard unavailable' at `src/lib/stores/database.svelte.ts:3005,3013`. |
| 11 | [minor] "Cannot" and "No workspace" variants | **Fixed** | Grep for "No workspace", "Cannot …" and "Title cannot" titles returns 0. |
| 12 | [minor] Bare "Loading…" | **Fixed** | Grep returns 0 for `>Loading…<` and `'Loading…'` in markup (no remaining standalone "Loading…" placeholders). |
| 13 | [minor] "Directory" and "Repo" labels | **Partial** | The "Repo" and "Repo path" labels are gone. Still open: "Working directory" at `src/modules/agents/NewSession.svelte:530,540,637`, `src/shell/Navigator.svelte:372`, `src/modules/personal-agents/AgentDocuments.svelte:83`, and "destination directory" at `src/modules/database/ExportDialog.svelte:206`. |
| 14 | [nit] Straight quotes in prompts | **Fixed** | `src/modules/product/ProductPage.svelte` and `src/modules/product/LearningsView.svelte` no longer match. The remaining "can't be undone" hits are code comments, plus `src/lib/stores/workspace.svelte.ts:1323`, a straight apostrophe in user copy that the ratchet baseline still covers. |
| 15 | [nit] "item(s)" | **Open** | `src/lib/stores/apiClient.svelte.ts:1042,1116`. |
| 16 | [nit] "Add workspace…" command | **Fixed** (checked, no regression) | `src/shell/App.svelte`. |
| N1 | [nit] Ratchet regex gaps | **Fixed** | `scripts/ui-guards.mjs:491–497` adds ratchets for failed titles, raw bodies, UK spelling and straight contractions. The ternary-title and `workspace.svelte.ts:1323` leftovers are folded into #1 and #4. |
| N2 | [nit] "Hired …— the rest failed" in an error toast | **Fixed** | See #9. |

### New findings
None. In the files sampled, no rewritten title or body regressed. The earlier fixes (curly "Couldn’t", `plural`, `severityLabel`, `sessionState` labels, "New workspace…") still hold.

### Remaining to reach 9.8
- Finish the raw-body and title tail: the six sites in finding #2, with `ContextPacketDialog.svelte:99` first. Also teach the ratchet regex about ternary titles (`workspace.svelte.ts:1290`).
- Clear the 28 straight-contraction attributes, `EditFlow.svelte.ts:713` ("Write cancelled") and `workspace.svelte.ts:1323`, then drop those entries from the ratchet baseline.
- Replace the remaining "Working directory" and "destination directory" copy with "Working folder" and "destination folder" (`NewSession`, `Navigator`, `AgentDocuments`, `ExportDialog`).
- Map `{f.severity}` at `ReviewAgents.svelte:191`. Rewrite 'No create statement' and 'Clipboard unavailable' as "Couldn’t copy the create statement" in `database.svelte.ts`. Rename the "Reload" refresh buttons in `LogsView`, and use `plural()` for "item(s)" in `apiClient.svelte.ts`.
