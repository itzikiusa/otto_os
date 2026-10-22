# Quality iteration 4 — lens: ui-performance

Reviewer: ui-performance lens. Read-only (grep/rg/sed/git only). No cargo, npm, build, e2e or test
runs.

- Worktree: `fix/quality-loop-20261021` @ 6cff08270.
- Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
  `ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: start at 10.0. For each confirmed open finding, subtract 1.0 for a blocker, 0.3 for a major,
0.1 for a minor and 0.03 for a nit.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 1 | 0.10 |
| nit | 0 | 0.00 |
| **Total** | 2 | **0.40** |

**Score: 10.0 − 0.40 = 9.60 / 10**

Both iteration-3 findings are fixed. The two new findings are in the Workbench editor. No earlier pass
covered it, and it is one of the most-changed UI areas in the last three weeks (`WorkbenchPage.svelte`
+855 lines, `workbench.svelte.ts` +618 lines).

## Previous-finding status (iteration 3)

| id | Iter-3 severity | Status | Evidence |
|---|---|---|---|
| ui-performance-3-01 | minor | **Fixed** | `kubernetes/monitor/MonitorCluster.svelte:370-389`: the listener effect reads `win` through `untrack(() => win)` (`:378`), so it depends only on the host and tbody elements. The re-measure effect is keyed on `sliceKey = $derived(win ? \`${win.start}:${win.end}\` : 'all')` (`:394`), and reads `win` untracked (`:399`). |
| ui-performance-3-02 | minor | **Fixed** | `aws/SqsView.svelte:365` passes `onrefresh={() => load({ awaitAttrs: true })}`. `load()` awaits the fan-out on auto ticks (`:104-105`). The fan-out goes through `aws.fanOutSqsAttrs(...)`, which fetches with `fetchSqsAttrs` (no store write) and commits in batches through `commitSqsAttrs` (`lib/stores/aws.svelte.ts:315-325`). |

## Areas covered this iteration

- **Re-verification** of both iteration-3 findings (table above).
- **Diffs since iteration 3:**
  - `eec25c214`: the Terminal grid-sync extraction to `termGridSync.ts`. It is a pure move, and every
    listener added in it is removed. Also the shell split (`installShell.svelte.ts`,
    `paletteSources.svelte.ts`): each source is its own effect, and the registry map stays plain with a
    raw `sources` signal. Also the workflows inspector split (`IdPicker`, `RepoPicker` and `SwarmTask`
    call `ensure` under `untrack`), the git store PR-list removal, the `workspace.svelte.ts`
    unknown-chunk keep, and the SQS store fan-out.
  - `5e1473454`: the `Object.hasOwn` event route.
  - `eb295dcc2`: the GraphView "landed in develop / main" mark. It is a per-row array that is already
    in the refs payload, with no new effect.
- **Workbench** (new this pass): `WorkbenchPage.svelte` (all effects and deriveds, the per-keystroke
  path), `workbench.svelte.ts` (`setBuffer`, the debounced backup and autosave, `isDirty`),
  `lib/detect.ts`, `lib/placeholders.ts`, `PlaceholdersPanel`, `Preview` (250 ms debounce),
  `SendToButton`, and the `HistoryPanel`/`FileList`/`EditorTabs` lists. On the `CodeEditor.svelte`
  side: the path/content effect, `swapState`, `createState` and the change emitter (synchronous
  below 256 KB).
- **AWS** (iteration-3 gap): `LogsView` (raw 5,000-event ring, `TableWindow`, the adaptive tail
  poller's capture and restart, Insights backoff, the debounced group prefix). Also `Ec2View`,
  `EksView`, `RdsView` and `AthenaView`. These have no pollers of their own: they refresh through
  ViewToolbar, and `Ec2View`'s load effect is keyed on `region` only.
- **Database:**
  - `MultiRunDialog`: `$state.raw` job, `?since=` delta poll, `onDestroy` stop.
  - `DatabaseChanges`: a 5 s hidden-paused poll.
  - The `DatabasePage` effects: the drag-reveal macrotask, the tab-strip scroll rAF, and the
    workspace load chain, whose loaders key their in-flight dedupe on workspace and epoch.
  - The `database.svelte.ts` debounced tab persistence.
- **Iteration-3 "not covered" items:**
  - `canvas/ConversationPanel`, `insights/ReportDetail`, `mcp/*` and `vault/RightPanel`/`FileTree`
    have no effects or timers to speak of.
  - `skills-eval/RunDetail`: the generation-guarded poll with a compare-before-assign. Its WS nudge
    is emitted only on completion (`crates/otto-server/src/skill_eval.rs:1216`).
  - `panels/BrowserPanel`: the rAF-coalesced bounds sync, with a 2 s visible-only drift check.
  - `mcp/ApprovalsTab` and `McpPage` use `liveQuery`.
  - `vault/NoteView`: render cache and off-thread render, image hydration outside the render, an
    embed worker pool of 2, diagram rendering.
  - `WorkflowsPage` live-run merge: `applyLiveNode` in place, with the `detail_version` skip.
- **Other:**
  - `shell/RightPanel` (notes debounce, folder stat per pick).
  - `agents/NewSession` (`recentDirs` derived, bounded at 12).
  - `settings/Logs` (5k-line ring, 150 ms filter debounce).
  - `lib/stores/apiClient.svelte.ts` (debounced tab persistence).
  - `lib/events.svelte.ts` dispatch.
- **Repo-wide audits repeated:** all 18 `setInterval` sites (unchanged since iteration 3), and
  `$derived` expressions that split or parse a whole text buffer (8 sites; only the Workbench ones
  run per keystroke on an unbounded buffer).

## Areas NOT covered yet (for the next iteration)

- `ui/src/modules/home/classrooms/*` (`scene.ts`, `model.ts`): heavy recent churn, but it looks
  like part of the school/classrooms rewrite that is excluded. Confirm the scope before reviewing it.
- `lib/components/Terminal.svelte`: the WebGL atlas, renderer swap and `termPark` re-adopt cost (owned
  by the dedicated terminal review).
- The `WorkflowsPage.svelte` inspector forms on very large runs (hundreds of nodes), apart from the
  live merge.
- `git/GraphView.svelte` (4.9k lines): only its refs-resync and background-reload throttle, plus the
  new landed mark, were re-read this pass.
- AWS `Ec2View`/`RdsView` with thousands of instances under "All regions" plus auto-refresh. Their
  tables are not windowed. This was not measured, so it is not reported.
- `product/*` beyond iteration 2's OverviewTab, PlanTab and AnalysisTab: `product/jira/*` has no
  effects. `design-hall/*` beyond `site/*`.
- Runtime measurement (heap over time, long tasks during agent bursts) needs a running app, which the
  read-only rule excludes.

## Findings

### ui-performance-4-01 — major — The Workbench editor's `path` follows the auto-detected language, so typing flips the CodeEditor to another EditorState: a full state rebuild, the cursor jumps to 0, and undo history is lost

- **Files:**
  - `ui/src/modules/workbench/WorkbenchPage.svelte:57, 62, 665-672`.
  - `ui/src/lib/components/CodeEditor.svelte:1046-1062` (path effect), `:948-975` (`swapState`),
    `:725-826` (`createState`), `:395-410` (change emitter).
  - `ui/src/modules/workbench/workbench.svelte.ts:640-643` (new docs are `Untitled N` with
    `language: 'auto'`).
- **Evidence:**
  - `lang = $derived(doc ? effectiveLanguage(doc.language, doc.name, active?.buffer ?? '') : 'txt')`
    (`:57`). For an `auto` doc whose name has no known extension (every new file:
    `nextUntitledName` → `"Untitled N"`, `language: 'auto'`), this re-sniffs the live buffer.
  - `editorPath = $derived(doc ? \`workbench/${doc.id}.${cmExtFor(lang) || 'txt'}\` : '')` (`:62`).
    The path carries the language, and CodeEditor picks the language pack from the path extension:
    `cmLangFor` → `langExtOf` → `extOf(path)` (`CodeEditor.svelte:343, 468-474`).
  - Below 256 KB the CodeEditor emits on every edit synchronously (`changeEmitter.ts:62-66`,
    `LAZY_CHANGE_MIN = 256 * 1024`). `onEdit` → `workbench.setBuffer` → `lang` re-derives. When the
    sniff result changes, `editorPath` changes.
  - The page passes `keepStates` (`:671`), so the CodeEditor path effect takes the `swapState` branch
    (`:1056-1061`). `swapState` parks the current EditorState under the old path and, with no parked
    state for the new path, runs `view.setState(createState(toPath, toContent))` (`:972-975`).
    `createState` builds a fresh `EditorState.create({ doc, extensions })` (`:822-825`). The work is
    a full re-split of the doc and a new extension set, and the language pack is loaded and parsed
    again. The new state starts with the selection at 0 and an empty history. Flipping back restores
    the parked state with a whole-document replace (`:1001-1006`).
  - Concrete flips while typing into a fresh file:
    - Typing `{`: closeBrackets makes the text `{}`, which is valid JSON, so `txt` becomes `json`.
      The path goes from `….txt` to `….json`, and the cursor lands before the `{`.
    - Typing `"` next gives `{""}`. `JSON.parse` throws, every other sniff misses, and `detect.ts`
      returns `'txt'`, so the path flips back.
    - Typing `# H` turns `txt` into `md`, which makes the path `….md`.
  - Every flip also flips `wrap={lang === 'md' || lang === 'txt'}` (`:672`), which costs a second
    compartment reconfigure.
  - Net per flip: an EditorState teardown and rebuild plus a language re-parse on the keystroke path.
    The user also sees it: the caret jumps to the start of the document mid-word and ⌘Z has nothing
    to undo.
  - It is untested: the Workbench e2e specs either name the file with an extension
    (`desktop-workbench.spec.ts:89`, `scratch.json`) or paste a whole value with one
    `insertText`.
- **Fix:**
  - Keep the editor path stable per document (`workbench/${doc.id}`, no language suffix), and pass
    the language as the existing `language` prop: `language={cmExtFor(lang)}`.
  - In `CodeEditor.svelte`, add a live language reconfigure effect, modelled on the `sqlDialect` one
    (`:1116-1122`). It reads `language`; if it differs from the applied one, it dispatches
    `langCompartment.reconfigure(cmLangFor(path, language) ?? [])` and calls
    `ensureLang(path)`. This leaves no `setState`, so the selection and history survive.
  - Make auto-detection sticky while typing, for example by re-sniffing only on doc open, paste, and
    after ~400 ms idle (the same debounce as validation). The language then cannot flip on every
    transient invalid state of a JSON or Markdown buffer.
  - Add an e2e test that types `{"a": 1}` and `# Heading` character by character into a new
    Untitled file and asserts the text and caret position.

### ui-performance-4-02 — minor — Workbench language sniffing scans the whole buffer twice per keystroke (and the Placeholders panel re-parses it a third time)

- **Files:** `ui/src/modules/workbench/WorkbenchPage.svelte:57-59`;
  `ui/src/modules/workbench/lib/detect.ts:79-86, 115-170`;
  `ui/src/modules/workbench/PlaceholdersPanel.svelte:17`;
  `ui/src/modules/workbench/lib/placeholders.ts:47-75`.
- **Evidence:**
  - Two independent deriveds run the same sniff on every buffer change:
    - `lang = $derived(... effectiveLanguage(doc.language, doc.name, buffer))`. For `auto`, this is
      `detectLanguage(name, content)` (`detect.ts:172-175`).
    - `detected = $derived(doc ? detectLanguage(doc.name, active?.buffer ?? '') : 'txt')` (`:58`).
      It is used only for the "Auto (JSON)" option label (`:561`), and it runs even when the doc has
      an explicit language.
  - Below 256 KB the CodeEditor emits on every keystroke (see 4-01), so both run per keystroke.
  - For an extension-less name (every new `Untitled N`), `detectLanguage` does all of the following
    over the full text:
    - `content.replace(/^﻿/, '')` and `text.trim()`.
    - `firstLine(text)`, which does `content.split('\n')` on the WHOLE document just to find the
      first non-blank line (`:79-86`).
    - A full `JSON.parse(t)` whenever the text starts with `{`/`[` and ends with `}`/`]` (`:132-138`).
    - Another `text.split('\n')` (`:152`).
    - Three per-line regex `filter` passes (`d2Edges`, `yamlish`, `nonBlank`; `:154, 164-165`).
    - Several whole-text `/m` regex scans (`:155, 158, 163, 167`).
  - On a 100-250 KB pasted log or JSON scratch file, that is two full splits, two JSON parses or
    regex sweeps and the line filters, all doubled, on the input path. The Preview and validation
    are debounced (`Preview.svelte:43`, `WorkbenchPage.svelte:181`), but the sniff is not.
  - With the Placeholders panel open, `found = $derived(detectPlaceholders(content, lang))` adds a
    third full pass per keystroke. For SQL docs that pass is `sqlHits` (`placeholders.ts:47-75`): a
    permissive regex sweep, then the quote- and comment-aware `substituteVars` parser over the whole
    script, then a char-by-char walk of the marked copy.
- **Fix:**
  - Compute the sniff once: `const detected = $derived.by(...)`, then
    `lang = explicit language ? doc.language : detected`. Better, compute it only from a debounced or
    sticky snapshot of the buffer (see 4-01): for example a `sniffText` state updated on open, paste,
    or after 400 ms idle.
  - In `detect.ts`, bound the content sniff to a prefix (for example the first 8-16 KB or first ~200
    lines). Replace `firstLine`'s full `split` with an `indexOf('\n')` walk. Only attempt the
    `JSON.parse` confirmation on the prefix-bounded case or when idle.
  - Debounce the PlaceholdersPanel input (it already only matters for display), for example a 200 ms
    debounced `content` like `Preview.svelte`.
