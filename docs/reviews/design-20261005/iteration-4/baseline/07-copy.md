## Lens: Copy & content

**Score: 8.2/10.** Arithmetic: 10.0 − 0.6 (2 major × 0.3) − 1.1 (11 minor × 0.1) − 0.09 (3 nit × 0.03) = 8.2. Three error-title patterns, raw exception text in toast bodies and a half-applied apostrophe and spelling sweep keep it from 9.8.

**Credited from earlier passes and checked:**
- Sentence case holds. Only module and proper-noun titles ("Open Settings", "Open Brand Kit") are capitalised.
- No `...` characters appear in UI copy. The only hit is the `go test ./...` placeholder at `ui/src/modules/skills-eval/MatrixView.svelte:355`, which is correct.
- `toastError` (`ui/src/lib/toastError.ts`) exists and has about 298 call sites. Its title is "Couldn’t <verb>" and its body is the human-readable cause.
- The `plural` helper exists (`ui/src/lib/plural.ts`).
- `humanize()` (`ui/src/modules/run-with-otto/runStatus.ts:47`) and `displayState` (`ui/src/modules/vault/DocsAgentsView.svelte:179`) map enums to words.
- Most loading text names its object ("Loading trash…", "Loading files…").

### Findings

**[major] Error toast titles use three competing patterns**
- "Couldn’t …": the swept files, for example `ui/src/modules/vault/vault.svelte.ts:410`.
- "Could not …": 87 occurrences across 28 files. Examples: `ui/src/modules/git/FindingActions.svelte:89,109,134,166,207`; `ui/src/modules/product/OverviewTab.svelte` (13 sites, e.g. `:211,481,572`); `ui/src/modules/product/QuestionsTab.svelte`, `ui/src/modules/product/ProductPage.svelte:82,120,195`.
- Noun-first "X failed" or "Copy failed": about 190 sites. Examples: `ui/src/lib/stores/database.svelte.ts:1114,3302,4339`; `ui/src/lib/stores/apiClient.svelte.ts:945–1745`; `ui/src/modules/canvas/SceneList.svelte:70,86,112,128`; `ui/src/modules/aws/SqsView.svelte:154–273`; `ui/src/modules/aws/S3Browser.svelte:327–497`; `ui/src/modules/brokers/BrokersPage.svelte:109–280`; `ui/src/modules/database/DatabasePage.svelte:431–703`; `ui/src/modules/git/GitToolbar.svelte:59,140,176,200`; `ui/src/lib/stores/workspace.svelte.ts:1511,1575,1585,1633`. Also the bare "Copy failed" at `ui/src/modules/canvas/MermaidCanvas.svelte:266`, `ui/src/modules/canvas/D2Canvas.svelte:263`, `ui/src/modules/database/ResultsGrid.svelte:1132,1161` and about 20 other places.
- Several of these titles name no object: "Change failed" (`ui/src/shell/Navigator.svelte:381`), "Add failed" (`ui/src/modules/git/GitPage.svelte:284`), "Branch failed" (`GitToolbar.svelte:140`), "Delete failed" (`workspace.svelte.ts:1511,1585`), "Move failed" (`ui/src/modules/product/ProductPage.svelte:182`).
- Rule: content.md §4 gives the title as "what failed, starting with the verb the user tried". The previous pass swept only the vault, git and settings files.
- Fix: route these through `toastError('Couldn’t <verb> the <object>', e)`. Add a guard in `ui/scripts/ui-guards.mjs` that rejects `toasts.error('… failed'` and `'Could not`.

**[major] Raw exception text is the toast body at about 285 sites**
- Examples: `ui/src/modules/vault/RefineDrawer.svelte:150,171,195`; `ui/src/shell/Navigator.svelte:364,381,398`; `ui/src/modules/browser/TabStrip.svelte:18`; `ui/src/modules/snip/SnipEditor.svelte:281,334,338`; `ui/src/modules/aws/InstallPanel.svelte:21`; `ui/src/modules/product/OverviewTab.svelte:481,572,617,766,806,847,1075` (20 sites); `ui/src/modules/vault/vault.svelte.ts:823` (`String(e)`).
- Rule: content.md §4 says "never the raw exception". `String(e)` renders "Error: …" or "ApiError: …". Status-code mapping (403, 404, 409, network) is skipped.
- Fix: replace `e instanceof Error ? e.message : String(e)` with `toastError(title, e)`, which calls `loadErrorText`. Find the remaining sites with `grep -rE "toasts\.(error|warn)\(.*(instanceof Error|String\()" ui/src`.

**[minor] UK spellings are still in visible copy**
- "Review cancelled": `ui/src/modules/vault/DocsAgentsView.svelte:172`.
- "The review was cancelled": `ui/src/modules/workflows/RunAgents.svelte:221`, directly beside the label "Canceled".
- `toasts.info('Run cancelled')`: `ui/src/modules/workflows/WorkflowsPage.svelte:871`.
- "Export cancelled": `ui/src/modules/database/ExportDialog.svelte:176`.
- "Write cancelled": `ui/src/modules/database/EditFlow.svelte.ts:711`.
- "Placement cancelled.": `ui/src/modules/rooms/RoomAnnotations.svelte:36`.
- "Screen sharing was cancelled": `ui/src/modules/rooms/room-media.ts:248`.
- "analyse": `ui/src/modules/product/ProductPage.svelte:579` and `ui/src/modules/product/DiscoveryTab.svelte:195`.
- "colours" in a command keyword: `ui/src/modules/design-hall/DesignHallPage.svelte:135`.
- Rule: US English only. The earlier sweep left these behind, and `SftpBrowser` and `AutomationEditor` already say "Canceled".
- Fix: change to "canceled" and "analyze". Add `cancelled|colour|analyse` as a copy ratchet that excludes comments and wire values.

**[minor] Straight apostrophes in prose after a curly-quote sweep**
- "Couldn't": 217 straight against 555 curly. Examples: `ui/src/modules/mission-control/MissionControlPage.svelte:375` ("Couldn't load Mission Control"); `ui/src/modules/home/boxes/UsageBox.svelte:115`, `InsightsBox.svelte:88`, `MissionControlBox.svelte:99`; `ui/src/modules/agents/conversation/ConversationView.svelte:794`; `ui/src/modules/database/StructureView.svelte:669`; the 15 sites in `ui/src/modules/proof/ProofPage.svelte`; `ui/src/modules/swarm/SwarmPage.svelte` (11); `ui/src/modules/settings/Notifications.svelte:123,181,192`.
- Other contractions render straight: `ui/src/modules/help/Walkthroughs.svelte:309`, `ui/src/modules/loops/LoopsPage.svelte:89`, `ui/src/modules/kubernetes/NamespacePicker.svelte:170`, `ui/src/modules/agents/TiledView.svelte:390`, `ui/src/modules/swarm/AgentGraph.svelte:294`, `ui/src/modules/workflows/WorkflowsPage.svelte:1704`.
- Rule: content.md §3, "curly quotes in prose". The same product shows both forms side by side, for example `RepoView.svelte:422` ("aren’t") against `FindingsBoard`.
- Fix: bulk-replace `([A-Za-z])'(t|s|re|ll|ve)\b` with the curly form in visible strings only. Then extend the guard to flag a straight apostrophe inside `title=`, `body=`, `placeholder=` and `toasts.*`.

**[minor] "Reload" is used where the action is "Refresh"**
- `ui/src/modules/share/SharePage.svelte:411` (the button reads "Reload" next to a refresh icon).
- `ui/src/modules/personal-agents/AgentMemoryInspector.svelte:99` (`aria-label="Reload memories"` but `title="Reload"`, so the tooltip does not equal the aria-label).
- `ui/src/modules/aws/AthenaView.svelte:386` ("Reload catalog").
- `ui/src/modules/agents/conversation/ConversationView.svelte:703` ("Reload transcript").
- `ui/src/modules/personal-agents/AgentDocuments.svelte:106` ("Reload saved version").
- `ui/src/modules/plugins/PluginFrame.svelte:165` ("Reload {name}").
- Rule: the Vocabulary table reserves "Reload" for the UI itself (⌘⇧R and the chunk-failed screen). Browser "Reload page" in `BrowserView` and `BrowserPanel` is correct.
- Fix: use "Refresh …". Keep `AgentDocuments.svelte:106` only if it truly discards edits, and then say "Discard changes and reload".

**[minor] Enum values and internal kinds still reach the screen**
- `ui/src/modules/workbench/send/SessionPickerModal.svelte:56` shows `{s.provider} · {s.status}` raw.
- `ui/src/modules/vault/RecoveryView.svelte:125` shows `{entry.kind}`.
- `ui/src/modules/git/FindingsBoard.svelte:147–149` `statusLabel` only strips underscores, so chips show "in progress" in lower case. It is used at `:198,234,294`.
- `ui/src/modules/git/FindingsBoard.svelte:150–152` `transitionLabel` prints raw `from → to` values.
- Raw severity strings: `ui/src/modules/skills-lab/SkillReviewPanel.svelte:452,490`, `ui/src/modules/skills-lab/SkillReviewAgents.svelte:105`, `ui/src/modules/skills-eval/RunDetail.svelte:551`.
- Raw `{kind}`: `ui/src/modules/assistant/cards/ActionCard.svelte:26`, `ui/src/modules/canvas/Inspector.svelte:53`, `ui/src/modules/insights/CapabilitiesPage.svelte:164`, `ui/src/modules/vault/LiveContext.svelte:291,336`.
- Raw `a.requested_by_kind` in parentheses: `ui/src/modules/mcp/ApprovalsTab.svelte:206`.
- Raw Jira status in a "Move to {to_status}" label: `ui/src/modules/product/OverviewTab.svelte:515,538`. These are Jira's own names, so they are probably fine.
- Rule: content.md §6, "mapped to words, not just de-underscored".
- Fix: add a per-domain mapper or use `humanize()`. For session status, reuse the shared session-state labels.

**[minor] Placeholders are instructions, not examples**
- "Paste log / command output / note": `ui/src/modules/proof/ProofPage.svelte:997`.
- "Paste the PR description / claims": `ProofPage.svelte:1150`.
- "Write your story or paste notes here…", "Paste conversation or notes here…", "Write the description in Markdown…", "Add a comment…": `ui/src/modules/product/OverviewTab.svelte:1412,1493,1546,1606`.
- "Write answer or additional context…": `ui/src/modules/product/QuestionsTab.svelte:485`.
- "Type a file name": `ui/src/modules/workbench/QuickOpen.svelte:81`.
- "Add a note": `ui/src/modules/browser/NotesRail.svelte:69`.
- Rule: content.md §5, "Placeholders are examples… not instructions".
- Fix: use example text ("Login fails after the SSO redirect…") and keep the verb in the label. "Type a command…" and "Type or speak…" are accepted prompt patterns.

**[minor] Plurals are hand-rolled at about 246 sites, and `plural()` exists twice**
- Pattern: `${n} thing{n !== 1 ? 's' : ''}`. Examples: `ui/src/modules/product/OverviewTab.svelte:1272–1276`; `ui/src/modules/git/DiffViewer.svelte` (7); `ui/src/modules/database/MultiRunDialog.svelte` (8); `ui/src/modules/git/FocusView.svelte:352,356`; `ui/src/modules/settings/SkillsLibrary.svelte:209`.
- The helper is duplicated at `ui/src/lib/plural.ts:3` (which supports an irregular plural) and `ui/src/lib/orchestrate.ts:321` (which does not).
- Fix: delete the copy in `orchestrate.ts`, re-export from `ui/src/lib/plural.ts`, and migrate the call sites. Use `plural(n, 'analysis', 'analyses')` for `OverviewTab.svelte:1273`.

**[minor] Toast titles are not verb-first or have no object**
- `ui/src/modules/agents/MissionControl.svelte:161`: `e.message` fallback "Failed to save view".
- `ui/src/modules/panels/BrowserPanel.svelte:460`: "Failed to send" with the body "Could not inject message into the agent session."
- `ui/src/modules/brokers/ReplayPanel.svelte:72`: the title is just "Replay".
- `ui/src/modules/kubernetes/ClustersOverview.svelte:42` and `ui/src/lib/uiCommands/k8s.ts:318`: "{name}: unreachable" and "{verb} failed · {name}".
- `ui/src/modules/swarm/RecruiterWizard.svelte:157`: an error toast titled "Hired 2 of 4 — the rest failed".
- `ui/src/modules/mcp/PolicyForm.svelte:53` and `ui/src/modules/mcp/ServerForm.svelte:92–100`: validation messages shown as error toasts. content.md §4 says invalid fields get inline errors.
- Fix: use "Couldn’t save the view", "Couldn’t send to the agent", "Couldn’t replay the messages" and similar. Move field validation inline.

**[minor] Bare or cryptic titles that leak internals**
- "Couldn’t register the close guard": `ui/src/modules/snip/SnipEditor.svelte:338`.
- "Scene edit rejected by the validator": `ui/src/modules/product/design/DesignArena.svelte:498`.
- "Secure secrets failed": `ui/src/lib/stores/apiClient.svelte.ts:966`.
- "Transition failed": `ui/src/modules/product/OverviewTab.svelte:557`.
- "Clipboard unavailable", "No create statement" and "DB assistant failed" (`ui/src/lib/stores/database.svelte.ts:3012,3004,1114`). "DB assistant" is not the vocabulary "Ask Otto".
- "Install finished but engine is not available": `ui/src/lib/api/usage.svelte.ts:670`.
- Fix: rewrite each as what the user tried plus a next step.

**[minor] "Cannot" and "No workspace" variants for the same state**
- "No workspace" at `ui/src/modules/aws/AwsPage.svelte:105`, `ui/src/modules/kubernetes/ClusterWorkspace.svelte:264` and `ui/src/modules/database/DatabasePage.svelte:658`.
- "No workspace selected" at `ui/src/lib/components/BroadcastModal.svelte:61`, `ui/src/shell/Palette.svelte:333`, `ui/src/shell/App.svelte:550`, `ui/src/modules/settings/Providers.svelte:163` and `ui/src/lib/stores/apiClient.svelte.ts:1346`.
- "Cannot open" at `ui/src/modules/git/GraphView.svelte:1647`, "Cannot start {x}" at `ui/src/modules/agents/NewSession.svelte:303` and "Title cannot be empty" at `ui/src/modules/product/OverviewTab.svelte:787`, against the voice "Couldn’t".
- Also "No agent session" (`ui/src/modules/panels/BrowserPanel.svelte:440`, `ui/src/lib/components/CodeEditor.svelte:435`) against "No active session" (`ui/src/modules/settings/SelfImprovement.svelte:257`).
- Fix: one shared string such as "Select a workspace first" (a toast helper or constant). Align the "Cannot …" titles.

**[minor] Bare "Loading…" with no object**
- `ui/src/modules/kubernetes/PodHttpPanel.svelte:219`
- `ui/src/modules/git/FindingsBoard.svelte:289`
- `ui/src/modules/design-hall/LearnedCard.svelte:82`
- `ui/src/modules/design-hall/CompareModal.svelte:136` (the `<option>` text)
- `ui/src/shell/Navigator.svelte:1466`
- `ui/src/modules/share/SharePage.svelte:363` (the title fallback)
- Rule: content.md §5, "Loading text names the thing".
- Fix: "Loading response…", "Loading finding details…", "Loading what Otto learned…", "Loading versions…", "Loading archived sessions…", "Loading session…".

**[minor] "Directory" and "Repo" in labels where the vocabulary says Folder and Repository**
- "Working directory": `ui/src/modules/workflows/WorkflowsPage.svelte:2262`, `ui/src/shell/Navigator.svelte:369`, `ui/src/modules/proof/ProofPage.svelte:349,359`, `ui/src/modules/personal-agents/AgentDocuments.svelte:83`.
- "destination directory": `ui/src/modules/database/ExportDialog.svelte:204`.
- "Worktree directory is gone": `ui/src/modules/git/GraphView.svelte:1647`.
- "Otto’s bin directory": `ui/src/modules/kubernetes/InstallPanel.svelte:34`.
- "Repo": `ui/src/modules/run-with-otto/RunLauncher.svelte:284`.
- "Repo ID (…)": `WorkflowsPage.svelte:2270,2718,3014`. This label asks the user to type an id, which breaks "label, never the id".
- "Repo path": `ui/src/modules/swarm/SwarmPage.svelte:753`.
- Rule: Vocabulary table (Folder, Repository).
- Fix: use "Working folder" and "Repository". Replace the Repo ID text fields with a repository picker.

**[nit] Straight quotes around names in prompts**
- `ui/src/modules/product/ProductPage.svelte:105` ("under "${epic.title}"") and `:455`.
- `ui/src/modules/product/LearningsView.svelte:228` mixes curly quotes with "can't".

**[nit] Count notation "item(s)"**
- `ui/src/lib/stores/apiClient.svelte.ts:1042` renders "{failed} item(s) of “…” weren’t imported".
- Fix: use `plural()`.

**[nit] The ⌘K command "Add workspace…" has the id `core.new-workspace`**
- `ui/src/shell/App.svelte:579`. It opens a creator, so §2 says "New workspace…".
- Fix: rename the command and the matching button label.

### What would get this lens to 9.8
- Make one error-title convention real. Everything goes through `toastError('Couldn’t <verb> the <object>', e)`, with a guard that bans `' failed'` and `'Could not` titles. Bodies come from `loadErrorText` only, so no `e.message` or `String(e)` reaches a toast.
- Finish the sweeps with ratchets in `ui/scripts/ui-guards.mjs`: UK spellings (cancelled, analyse, colour) in strings, straight apostrophes inside `title`, `body`, `placeholder` and `toasts.*`, and `Reload` outside the browser and chunk-reload cases.
- Delete the duplicate `plural()` in `ui/src/lib/orchestrate.ts` and migrate the hand-rolled `n !== 1 ? 's'` sites.
- Add per-domain label mappers for session status, finding status and severity, trash kind and approval requester, so no raw enum or kind is printed.
- Rewrite instruction-style placeholders as examples. Replace bare "Loading…" with object-named text and the "directory" and "Repo ID" labels with folder and repository pickers.
- Move field validation out of error toasts into inline messages.
