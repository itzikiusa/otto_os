## Lens: IA, navigation and interaction consistency — iteration-5 re-score

**Score: 9.80/10.** Start 10.0, minus 2 minors still Open or Partial (0.2), which is 9.80. The two are the Browser v1/v2 toggle (Open, minor) and the Search/Filter placeholder wording (Partial, minor). No majors, blockers or nits remain. This is a read-only check of `/Users/itziklavon/claude_ade-design5`.

Judgement: none. I agree with the arithmetic.

### Iteration-4 Partial/Open items

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | Modules without ⌘K verbs (major) | **Fixed** | `registry.register(` now appears in 35 module files. All nine formerly uncovered modules now register verbs: Personal Agents, Kubernetes (`KubernetesPage`, `ClustersOverview`), AWS (`AwsPage`), MCP (`McpPage`), Usage (`UsagePage`), Browser (`BrowserView`), Rooms (`RoomLobby`), History (`HistoryPage`) and Connections (`DatabasePage`). |
| 2 | Selection not in the URL (major) | **Fixed** | `BrokersPage.svelte:83,96`, `ApiPage.svelte:173-206` and `WorkbenchPage.svelte:85,100` read `router.parts` and write back with `router.replace`, guarded by `router.module`. |
| 7 | ⌘K title grammar (minor) | **Fixed** | No titles start with `Guide:`, `Switch workspace:`, `Open repo:`, `Layout:`, `Scheme:` or `Theme:` any more. Appearance commands read "Use light color scheme" and "Use Warm theme" (`App.svelte:591-596`). |
| 8 | Ellipsis rule (minor) | **Fixed** | "Broadcast message to sessions…" (`App.svelte:575`) and "New skill evaluation…" (`SkillsEvalPage.svelte:114`). "Manage API environments" and "New assistant thread" stay without "…". They open or create without asking for input, so they are consistent with the rule. |
| 11 | Browser v1/v2 toggle and panel overlap (minor) | **Open** | The toggle buttons are still at `RightPanel.svelte:384-396`. |
| 12 | Search/filter placeholders (minor) | **Partial** | Every placeholder now ends in "…". `Settings`, `SkillsBrowser` and `MemoryTab` now say "Filter". Several client-side narrowing fields still say "Search": `MissionControlPage.svelte:323`, `TopicsTab.svelte:278`, `ResultsGrid.svelte:1448`, `DatabasePage.svelte:1569,1625`. content.md §5 (`:108`) has no search/filter rule. |
| 14 | BottomNav sheet hand-rolled (minor) | **Fixed** | `BottomNav.svelte:109` uses `<Drawer … title="More">`. |
| 15 | Overlapping modules (minor) | **Fixed** | The Canvas ⌘K entry is removed from `sidebar.ts`; line 431 documents why. layout.md §1.1 "What lives where" (`:58-76`) covers Vault, Workbench, Notes, Canvas, Rooms and Skills Lab. |
| 16 | Rail duplicates Help/Settings (nit) | **Fixed** | `Rail.svelte` no longer has Help/Settings entries in the account menu. |
| 18 | Doc drift (nit) | **Fixed** | "Five sections" is gone and layout.md:137 now says "accent at 14%". |
| New | "Sign out" in the Tools group (nit) | **Fixed** | `App.svelte:607` uses `group: 'Account'`. |

### Spot-checks of items previously Fixed (no regressions)

- Rail Back button: `Rail.svelte:121`.
- KEYMAP ⌘S row: `keys.ts:570`.
- `SESSION_PANEL` constant: `rightTabs.ts:8`.
- Pane/Session/Navigate/View/Tools/Account grouping: `App.svelte:575-607`.

### New findings

None found. The new URL sync follows the same `router.module` guard as the pages fixed earlier.

### Remaining to reach 9.8

This lens is at the 9.8 target (9.80).
- To finish: remove the Browser v1/v2 toggle at `RightPanel.svelte:384-396` once v2 is the default.
- To finish: pick one wording for narrowing fields ("Filter …") and document the rule in content.md §5. That would also change the five "Search" narrowing fields above.
