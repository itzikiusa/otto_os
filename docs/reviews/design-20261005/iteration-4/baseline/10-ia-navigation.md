## Lens: IA, navigation and interaction consistency

**Score: 8.0/10.** Start 10.0, minus 2 majors (0.6), 13 minors (1.3) and 4 nits (0.12), which is 7.98. The sidebar registry is now a single source of truth, but ⌘K verbs, URL-addressable selection and the cheat sheet have not reached the same standard.

**Credited from earlier passes (verified):**
- `lib/sidebar.ts` feeds the Rail, Navigator, BottomNav, ⌘K "Go to" and the ⌥Space bar through `goToEntries`. The Brokers, Canvas and Help routes sit in `EXTRA_GOTO`, so nothing is hand-listed (`sidebar.ts:432`, `FloatingBar.svelte:144`).
- `navBadge` is applied on all three nav surfaces (`Rail.svelte:143`, `Navigator.svelte:1080`, `BottomNav.svelte:99`).
- The active-row language (accent tint, 3px bar, accent glyph) is consistent between Rail and Navigator.
- `aria-current` is set on all three nav surfaces.
- Back/Forward, Esc-to-clear and the quick filter on workspaces are well built.
- `installKeyMap` handles ⌘-chord editor ownership and the `⌃` stand-in carefully.

### Findings

**[major] Most modules register no ⌘K verbs**
- Evidence: `registry.register(` is called from only 11 module files: `run-with-otto/RunLauncher.svelte:187`, `skills-eval/SkillsEvalPage.svelte:112`, `assistant/AssistantPage.svelte:98`, `proof/ProofPage.svelte:687`, `design-hall/DesignHallPage.svelte:131` and `ArtifactView.svelte:656`, `api/ApiPage.svelte:247`, `home/HomePage.svelte:158`, `skills-lab/SkillsBrowser.svelte:295`, `insights/InsightsPage.svelte:513`, `database/ConnectionComparison.svelte:49`.
- Not covered: Workflows, Swarm, Goal Loops, Scheduled Tasks, Personal Agents, Mission Control, Product, Vault, Workbench, Kubernetes, AWS, MCP, Usage, Browser, Rooms, History and Connections. Git registers only "Open repo".
- Rule: patterns.md §9, "Register your module's main verbs".
- Fix: add a "New …" and one or two page verbs per module, starting with the list/detail pages' primary actions (new workflow, new swarm, new scheduled task, new note, new loop).

**[major] Selection is not URL-addressable on most list/detail pages**
- Evidence: these pages have no `router.parts` or `router.replace` selection and use only `lastSelection` (a per-device memory, not the URL): Proof, Workflows, Swarm, Product, Scheduled Tasks, Goal Loops, Run with Otto, Brokers, API, Workbench.
- Compare Git, History, Mission Control, Insights, Skills Lab, Personal Agents, MCP and AWS, which do put selection in the route (`MissionControlPage.svelte:57,71`, `HistoryPage`, `GitPage`).
- Notifications compensate with in-memory page ports instead of a URL: `notifications.svelte.ts:506-523` calls `router.go('workflows')` and then `page.open(...)`. The result is not shareable, and a reload loses it.
- Rule: patterns.md §8, "A selection is part of the URL when it should survive reload or be shareable".
- Fix: add `#/workflows/<id>`, `#/proof/<id>`, `#/swarm/<id>`, `#/scheduled-tasks/<id>`, `#/loops/<id>` and `#/product/<id>`, and have the ports call `router.replace`.

**[minor] The collapsed Rail has no Back/Forward**
- `Rail.svelte:109-183` has only the expand button, the bell and the modules.
- The Navigator header has the buttons (`Navigator.svelte:745-764`). `NavButtons` is mounted only in the compact top bar (`App.svelte:1138`).
- The default collapsed desktop state therefore has no visible history controls, although ⌘⇧←/→ works.
- The Rail also does not show the current workspace or let you switch it. Only ⌘K and the Navigator do.
- Fix: put Back/Forward in the Rail, or in the PageHeader leading slot, when the Rail is collapsed.

**[minor] Selected-row tint drifts away from `--accent-soft` (14%)**
- `color-mix(in srgb, var(--accent) N%, transparent)` is used with N = 12, 14, 16, 18 or 20 on selected rows in:
  - `vault/FileTree.svelte:410` (18%)
  - `aws/SqsView.svelte:568`, `aws/Ec2View.svelte:350`, `aws/S3Browser.svelte:865`, `aws/RdsView.svelte:333` (12%)
  - `kubernetes/ResourceTable.svelte:210`, `kubernetes/ClusterWorkspace.svelte:699` (14%)
  - `brokers/TopicDetail.svelte:1164` (16%)
  - `brokers/SchemaVersionsPanel.svelte:253` (20%)
  - `api/EnvSelector.svelte:248` (12%)
  - `agents/NewSession.svelte:742` (10%)
- Several other selected states use `--surface-2` or `--surface` instead of a tint: `vault/VaultPage.svelte:726`, `kubernetes/NamespacePicker.svelte:295`, `api/ApiPage.svelte:554`.
- Rule: layout.md §4.1, "selected row in `--accent-soft`".
- Fix: replace these with `var(--accent-soft)`. Extend the ui-guards ratchet to flag `color-mix(... var(--accent) N%` inside `.sel`, `.selected` and `.active` rules.

**[minor] The `?` cheat sheet (KEYMAP) is incomplete**
- Chords that exist in code but not in `KEYMAP` (`keys.ts:451-537`):
  - ⌘S in the Vault note (`vault/NoteView.svelte:345`)
  - ⌘E in the Vault note (`vault/NoteView.svelte:349`)
  - ⌘S in Skills Lab (`skills-lab/SkillEditor.svelte:176`)
  - ⌘S in Design Hall (`design-hall/ArtifactView.svelte:670`) and Brand Kit (`design-hall/brand/BrandEditor.svelte:401`)
  - ⌘F in the Git graph search (`git/GraphSearchBar.svelte:51`)
- KEYMAP lists ⌘S only for the API client (`keys.ts:494`). `ArtifactView` shows ⌘S in ⌘K (`:658`) but not in the cheat sheet.
- Rule: patterns.md §9, KEYMAP is "the only source for the `?` cheat sheet".
- Fix: add Vault, Skills Lab and Design Hall groups, or one "Save (⌘S)" row that names the pages.

**[minor] The Help guide's shortcut table has drifted from KEYMAP**
- `help/sections/keyboard-shortcuts.md` (lines 36-146) has no row for:
  - ⌘\ (side-by-side pane) or ⌥-click
  - ⌃Tab or ⌃⇧Tab
  - ⌘O or ⌘N (Vault)
  - ⌘B (Database)
  - ⌘R (live browser)
  - ⌥↑/⌥↓ (Favorites)
- It does list ⌘⇧N, ⌘Q and ⌘M. The two tables are hand-maintained.
- Fix: generate the Help table from `KEYMAP`, or add a unit test that diffs them.

**[minor] ⌘K title grammar is mixed**
- Colon prefixes: `Home: add widget…` (`HomePage.svelte:160`), `Switch workspace: X` (`App.svelte:730`), `Focus session: X`, `Open repo: X`, `Connect: X`, `Settings: X`, `Guide: X`, `Scheme: X`, `Theme: X`, `Layout: grid` (`Splits.svelte:86-90`).
- Verb-first titles are used elsewhere.
- "Home:" duplicates the `group: 'Home'` field.
- Navigation uses both "Go to Git" and "Open Settings" / "Open Help" / "Open Brand Kit" / "Open golden tasks" (`App.svelte:582`, `sidebar.ts:455`). The command id is `core.go-settings` but the title says "Open".
- "Keyboard shortcuts" has no verb (`App.svelte:596`).
- Fix: "Go to X" for pages and sections. "Open X" only for panels and overlays. Drop module prefixes when the group already says it. Use "Show keyboard shortcuts".

**[minor] The "…" rule is applied inconsistently to commands**
- patterns.md §9 says a trailing "…" means the command asks for more input.
- Violations:
  - `Broadcast message to sessions` (`App.svelte:572`)
  - `Manage API environments` (`ApiPage.svelte:251`)
  - `New skill evaluation` (`SkillsEvalPage.svelte:113`)
  - `New assistant thread` (`AssistantPage.svelte:99`; check whether it prompts)
  - `Assemble proof pack…` (`ProofPage.svelte:691`; check whether it prompts)
- Fix: audit each command's `run` and make the ellipsis follow the behavior.

**[minor] ⌘K group names scatter related verbs**
- Pane and split verbs are spread across `Sessions` (Split vertically/horizontally, `App.svelte:577-578`), `View` (all `split.*`, `App.svelte:910-944`) and `Layout` (`Splits.svelte:71-90`).
- The right-panel openers (Notes, Git) are in `View`, "Open Settings" is in `Navigate`, and "Take a screenshot (snip)" is in `Tools`.
- Module groups use the module's Title Case name (`Skills Lab`, `Design Hall`, `API`, `Proof`).
- Fix: define one group vocabulary (for example Session, Pane, View, Navigate, Tools, plus module names) and document it in patterns.md §9.

**[minor] The right panel has four names**
- `App.svelte:1151-1152` has `title="Activity panel"` with `aria-label="Toggle right panel"`.
- The `Drawer` label is "Activity" (`:1231`).
- The `<aside>` is "Session panel" (`RightPanel.svelte:443`).
- ⌘K, KEYMAP and the Help guide say "right panel".
- Only Notes and Git can be opened from ⌘K (`App.svelte:594-595`), but `RightTab` has nine tabs (`ui.svelte.ts:18-27`).
- Rule: components.md, where an icon-only button's `aria-label` and `title` are the same text.
- Fix: pick one name. Set `title` equal to `aria-label`. Register an "Open <tab> panel" command for every tab.

**[minor] The right panel duplicates top-level modules and exposes an internal version toggle**
- It has API, Browser and Canvas tabs, while Browser, API and Design Hall (Whiteboard) are also sidebar modules.
- Browser carries a user-visible v1/v2 switch (`RightPanel.svelte:395-408`) that `ui.svelte.ts:28-31` calls "Transitional".
- Fix: finish the v2 convergence and remove the toggle. State in the guidelines which tabs are session-scoped and which are mirrors.

**[minor] Search and filter placeholders follow no convention**
- "Search" versus "Filter" is arbitrary:
  - Client-side narrowing is called "Search" in `MissionControlPage.svelte:343`, `brokers/TopicsTab.svelte:272`, `database/ResultsGrid.svelte:1447` and `git/PrList.svelte:177`.
  - It is called "Filter" in `Navigator.svelte:943`, `aws/AwsPage.svelte:172` and `database/DatabasePage.svelte:1452`.
- Trailing "…" is on about 35 placeholders and missing on about 12:
  - `help/Walkthroughs.svelte:246`
  - `assistant/MemoryTab.svelte:256`
  - `api/CollectionsTree.svelte:287`
  - `api/HistoryList.svelte:160`
  - `skills-lab/SkillsBrowser.svelte:359`
  - `settings/Settings.svelte:251`
  - `settings/SkillsLibrary.svelte:273`
  - `design-hall/LearnedPage.svelte:399`
  - `design-hall/ReferencesPanel.svelte:146`
  - `design-hall/site/LeftPanel.svelte:249`
  - `agents/conversation/ConversationView.svelte:753`
- Other quirks: `kubernetes/ClusterWorkspace.svelte:591` uses "Filter  ( / )" with a double space and is the only field with a `/` focus key. The Navigator's "Search all sessions…" also filters.
- content.md §5 has no placeholder rule.
- Fix: add one rule. Use "Search <things>…" for server or fulltext search and "Filter <things>…" for narrowing a visible list. Always end with "…" (or never). Apply `/` to focus the list filter everywhere or nowhere.

**[minor] The ⌥Space bar omits plugin "Go to" commands**
- `FloatingBar.svelte:141` calls `availableModules(..., [])`, so plugin modules reachable in the app have no "Go to" there.
- The bar also lacks Settings sections, guides and Favorites commands. The `core.go-settings` entry is hand-added at `:152`.
- Fix: load `plugins.list` in the window, or document it as intentionally reduced.

**[minor] BottomNav selection and sheet differ from the other nav surfaces**
- The primary tab's active state is only a text colour (`BottomNav.svelte:205`), with no tint or bar.
- Active sheet items use `color-mix(... 12%, var(--surface))` (`:315-318`), not `--accent-soft`.
- The More sheet is hand-rolled (`:122-176`: its own backdrop, `dialogFocus`, `pushModal`) instead of `Drawer`/`Modal`. The "More" button has no `title`.
- Fix: share one active style via a small class or token, and build the sheet on `Drawer`.

**[minor] Overlapping modules with no stated boundary**
- Workbench (`sidebar.ts:120`) shares the keywords "mermaid d2 json markdown notes" with Design Hall (`:117`) and Vault (`:113`), and the right panel adds a Notes tab. There are three "scratch notes" homes.
- Canvas is still a separate ⌘K destination ("Canvas", `sidebar.ts:434`) as well as Design Hall's Whiteboard.
- `modules/product/design/scene3d/*` coexists with `modules/design-hall/studio3d/*`.
- Rooms exists as a module (`rooms/`) and as `personal-agents/RoomsView.svelte`.
- Skills Lab has route id `skills-eval` but folders `skills-lab` and `skills-eval`, and a Settings → Skills library.
- Fix: write a one-line "what lives where" table in layout.md §2 and remove the duplicate entry points, starting with Canvas in ⌘K and Product's design studio.

**[nit] Rail duplicates Help and Settings**
- They appear both as bottom buttons (`Rail.svelte:153-172`) and in the account menu (`:101-102`). The Navigator shows them once, in its footer.

**[nit] Navigator toast titles skipped the "Couldn't" sweep**
- `Navigator.svelte:364` ("Rename failed") and `:380` ("Change failed").
- Rule: content.md:90 requires "Couldn't …".

**[nit] Doc and token drift in the sidebar guidance**
- layout.md:115 says the active tint is 16%, but `--accent-soft` is 14% (`tokens.css:93`).
- layout.md:90 says "Five sections is the budget", but `SIDEBAR_GROUPS` has six, five plus runtime Plugins.
- The Navigator count chip says `title="working sessions"` in lowercase (`Navigator.svelte:884`).
- The Navigator Help row has no `title` while the Rail's does (`Navigator.svelte:1015-1023`).

**[nit] PageHeader icon use is inconsistent**
- `workbench/WorkbenchPage.svelte:369` and the Rooms pages (`rooms/RoomRecapsPage.svelte:18`, `rooms/RoomPage.svelte:110`) pass `icon`.
- layout.md says most pages omit it.

### What would get this lens to 9.8
- Give every sidebar module its main ⌘K verbs, with consistent title grammar, "…" rules and one group vocabulary.
- Put the selected item in the URL for Proof, Workflows, Swarm, Scheduled Tasks, Loops, Product and Run with Otto. Make notification deep links route through it.
- Generate the `?` sheet and the Help shortcut table from one `KEYMAP`, including page-scoped chords (⌘S, ⌘E, ⌘\, ⌃Tab).
- Replace the ad-hoc selected-row `color-mix` percentages with `--accent-soft`, and add a ratchet rule for it.
- Add Back/Forward, workspace context and plugin parity to the collapsed Rail and ⌥Space bar, and give BottomNav the same selection language.
- Document search/filter placeholder wording and the "what lives where" boundaries for overlapping modules, then remove duplicates (Canvas in ⌘K, the right-panel API/Browser/Canvas tabs, Product's design studio).

Files reviewed (all under `/Users/itziklavon/claude_ade-design/`):
- `ui/src/lib/sidebar.ts`
- `ui/src/lib/keys.ts`
- `ui/src/shell/App.svelte`
- `ui/src/shell/Rail.svelte`
- `ui/src/shell/Navigator.svelte`
- `ui/src/shell/BottomNav.svelte`
- `ui/src/lib/components/FloatingBar.svelte`
- `ui/src/lib/lastSelection.ts`
- `ui/src/lib/stores/notifications.svelte.ts`
- `docs/design/guidelines/layout.md`
- `docs/design/guidelines/patterns.md`
