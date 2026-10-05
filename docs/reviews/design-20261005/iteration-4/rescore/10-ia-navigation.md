## Lens: IA, navigation and interaction consistency — re-score

**Score: 8.7/10.** Start 10.0, minus 2 majors still Partial (0.6), minus 6 minors still Open or Partial (0.6), minus 3 nits (0.09), which is 1.29, so 8.71. This is a read-only check of HEAD b751ca6f.

### Previous findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | Most modules register no ⌘K verbs (major) | **Partial** | Now registered: Workflows (`WorkflowsPage.svelte:940`), Swarm (`:201`), Loops (`LoopsPage.svelte:80`), Scheduled Tasks (`:71`), Mission Control (`:240`), Product (`:340`), Vault (`VaultPage.svelte:135`), Workbench (`:283`), Git (`GitPage.svelte:62`), Canvas. Still none: Personal Agents, Kubernetes, AWS, MCP, Usage, Browser, Rooms, History and Connections. |
| 2 | Selection not URL-addressable (major) | **Partial** | Fixed for Proof (`ProofPage.svelte:122,142`), Workflows (`:389,404`), Swarm (`:208,228`), Product (`:309,334`), Scheduled Tasks (`:45,65`), Loops (`LoopsPage.svelte:59,74`) and Run with Otto (`:67,85`). Brokers still only calls `rememberSelection` (`BrokersPage.svelte:98`). API and Workbench still have no route selection. |
| 3 | Rail has no Back/Forward | **Fixed** | `Rail.svelte:123-130` |
| 4 | Selected-row tint drift | **Fixed** | Selected rows now use `var(--accent-soft)`: `SqsView:569`, `S3Browser:866`, `RdsView:337`, `Ec2View:354`, `TopicDetail:1170`, `ResourceTable:210`, `ClusterWorkspace:699`, `FileTree:410`, `EnvSelector:249`, `NewSession:742`. The remaining `color-mix` hits in `FileTree`, `SqsView`, `RdsView`, `NewSession` and `TopicDetail` are other states, not the selected-row rules. |
| 5 | KEYMAP incomplete | **Fixed** | `keys.ts:570-574` adds ⌘S, ⌘E, ⌘O/⌘N and ⌘B. |
| 6 | Help guide drift | **Fixed** | `keyboard-shortcuts.md:43-45,148-154` |
| 7 | ⌘K title grammar mixed | **Partial** | Fixed: "Go to Settings" (`App.svelte:583`), "Show keyboard shortcuts" (`:604`), no "Home:" prefix. Colon prefixes remain: `Guide:` (`:692`), `Switch workspace:` (`:738`), `Open repo:` (`:788`), `Layout:` (`Splits.svelte:86-90`), `Scheme:`, `Theme:`. |
| 8 | Ellipsis rule inconsistent | **Open** | `App.svelte:573`, `ApiPage.svelte:245`, `SkillsEvalPage.svelte:113` and `AssistantPage.svelte:101` are unchanged. |
| 9 | Group names scatter verbs | **Fixed** | Groups are now Session / Pane / Tools / Navigate / View (`App.svelte:570-594`, `:919-953`). |
| 10 | Right panel has four names; ⌘K opens only 2 of 9 tabs | **Fixed** | One `SESSION_PANEL` constant (`rightTabs.ts:8`) is used for the title, aria-label, Drawer label and KEYMAP (`App.svelte:587,1160,1241`). `Open ${t.label} panel` is registered per tab (`:598`). |
| 11 | Right panel duplicates modules; v1/v2 toggle | **Open** | The toggle is still at `RightPanel.svelte:385-396`. |
| 12 | Search/filter placeholders | **Partial** | Fixed: `HistoryList.svelte:162`, `CollectionsTree.svelte:288`. Still missing "…" or still mixing Search and Filter: `Walkthroughs:250`, `SkillsBrowser:373`, `Settings:251`, `SkillsLibrary:273`, `ReferencesPanel:146`, `LearnedPage:399`, `LeftPanel:249`, `MemoryTab:263`, `ClusterWorkspace:591`. content.md §5 still has no rule. |
| 13 | ⌥Space bar omits plugin Go-to | **Fixed** | `FloatingBar.svelte:146-149` passes `pluginEntries`. |
| 14 | BottomNav selection and sheet | **Partial** | Fixed: active tint and bar (`BottomNav.svelte:208-230`), the sheet uses `--accent-soft` (`:333-336`), and the More button has a `title` (`:113`). The sheet is still hand-rolled (`:125`). |
| 15 | Overlapping modules | **Open** | `sidebar.ts:434` still has the Canvas ⌘K destination. No boundary table was added to layout.md. |
| 16 | Rail duplicates Help/Settings (nit) | **Open** | `Rail.svelte:101-102` and `:183` |
| 17 | Navigator toast titles (nit) | **Fixed** | The "Rename failed" and "Change failed" strings are gone. |
| 18 | Doc/token drift and Help `title` (nit) | **Partial** | The Navigator Help row now has a `title` (`Navigator.svelte:1023`). layout.md:110 ("Five sections") and :135 ("16%") are unchanged. |
| 19 | PageHeader `icon` inconsistency (nit) | **Fixed** | No `<PageHeader icon=…>` remains. |

### New findings

- **[nit] "Sign out" now sits in the Tools group** (`App.svelte:605`). The old Account group was folded into Tools. Suggest a Navigate/Account group or the View group.
- No regressions found. The route-sync code in the pages above guards with `router.module === …` before `router.replace`.

### Remaining to reach 9.8

- Add ⌘K verbs to the nine modules still without them: Personal Agents, Kubernetes, AWS, MCP, Usage, Browser, Rooms, History and Connections.
- Make Brokers, API and Workbench selections addressable (`#/brokers/<id>` and so on).
- Finish the title grammar (a single `Guide`/`Layout`/`Scheme`/`Theme` style, or verb-first) and apply the "…" rule to Broadcast, Manage API environments, New skill evaluation and New assistant thread. Add a placeholder rule to content.md and apply it to the placeholders still missing it.
- Drop the Browser v1/v2 toggle and the Canvas ⌘K duplicate, and add a "what lives where" table to layout.md. Move the BottomNav More sheet onto `Drawer`.
- Correct layout.md ("Five sections", "16%") and move "Sign out" out of Tools.
