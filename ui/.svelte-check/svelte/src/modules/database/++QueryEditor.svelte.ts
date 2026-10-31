///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import { focusOnMount } from '../../lib/focusOnMount';
// SQL / Redis / Mongo editor. Wraps the shared CodeEditor with a server-backed
// completion source (debounced /db/completion). Cmd/Ctrl+Enter runs; toolbar
// has Run / Save / Explain-with-agent. Results render in the ResultsGrid below.
import { onDestroy, tick, untrack } from 'svelte';
import { plural } from '../../lib/plural';
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';
import { isInertAt } from './completion-gate';
import type { EditorState } from '@codemirror/state';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import { loadCmLang } from '../../lib/components/cm-langs';
import { sqlDialectForKind } from '../../lib/sql-dialects';
import ResultsGrid from './ResultsGrid.svelte';
import PlanView from './PlanView.svelte';
import VarsPrompt from './VarsPrompt.svelte';
import MultiRunDialog from './MultiRunDialog.svelte';
import { dbHandoff, prefillVarType } from './handoff.svelte';
import { databaseAccessChild } from '../../lib/access-options';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Icon from '../../lib/components/Icon.svelte';
import {
    database,
    effectiveViewMode,
    viewModeReason,
    ROW_LIMIT_ALL,
    TAB_HISTORY_PREFIX,
    type QueryTab,
  } from '../../lib/stores/database.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { clipHistory } from '../../lib/stores/clipHistory.svelte';
import { unpersistEditorState } from '../../lib/editor-history';
import { parkedEditCount } from '../../lib/database/grid-tab-state';
import { confirmer } from '../../lib/confirm.svelte';
import type { DbCompletionKind } from '../../lib/api/types';
import {
    statementAtCursor,
    analyzeStatement,
    extractVars,
    substituteVars,
    renderVar,
    defaultVarSpec,
    looksLikeMongoshScript,
    type SplitMode,
    type VarSpec,
  } from '../../lib/database/sql-util';
import { splitModeFor } from '../../lib/database/sql-dialect';
import { loadErrorText } from '../../lib/loadError';
import { api } from '../../lib/api/client';
import type { MongoshInfo } from '../../lib/api/types';
import { paneResizer } from '../../lib/paneResizer';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const tab = $derived(database.tab);

  // The result view the grid renders, resolved here (not in ResultsGrid, which
  // is also mounted without a tab): the tab's own pick → the auto-Vertical
  // column threshold → the connection's remembered pick → the engine default.
  const viewInputs = $derived({
    tabPick: tab.viewMode ?? null,
    connPick: database.connView,
    columnCount: tab.result?.columns.length ?? 0,
    // Per engine: MongoDB auto-verticals wide results by default, the SQL
    // engines keep the grid unless the user opted in (Settings → Appearance).
    autoVerticalCols: ui.dbAutoVerticalFor(database.capabilities?.engine),
    engine: database.capabilities?.engine ?? null,
  });
  const viewMode = $derived(effectiveViewMode(viewInputs));
  const viewReason = $derived(viewModeReason(viewInputs));

  // Default row-cap options (applied when a statement has no explicit LIMIT).
  const ROW_LIMIT_OPTS: { label: string; value: number }[] = [
    { label: '100', value: 100 },
    { label: '500', value: 500 },
    { label: '1,000', value: 1000 },
    { label: '5,000', value: 5000 },
    { label: '10,000', value: 10000 },
    { label: '50,000', value: 50000 },
    // Unbounded rows, still bounded by the daemon's 32 MB response budget.
    { label: 'All (≤ 32 MB)', value: ROW_LIMIT_ALL },
  ];

  // Editor language (drives syntax highlighting): a small Redis highlighter for
  // redis; JavaScript for Mongo — native queries are JS-like (`db.coll.find({…})`,
  // aggregate pipelines, BSON literals), so JS colourizes them properly (the
  // SELECT subset still reads fine as JS); SQL for mysql / clickhouse.
  const lang = $derived(
    database.queryLanguage === 'redis'
      ? 'redis'
      : database.queryLanguage === 'mongo'
        ? 'js'
        : 'sql',
  );
  // SQL / Redis packs are static in cm-langs.ts; the Mongo editor's JS pack and
  // the document editors' JSON pack load on demand. Warm both as soon as a
  // Mongo connection is active so the workbench doesn't open them plain.
  $effect(() => {
    if (lang !== 'js') return;
    for (const ext of ['js', 'json']) void loadCmLang(ext).catch(() => {});
  });
  // The connection's SQL dialect: it decides what the editor tokenizes as a
  // string or comment (MySQL `\'` escapes and `#` comments), which gates
  // completion inside literals and scopes the completion span to a statement.
  const sqlDialect = $derived(sqlDialectForKind(database.selectedConn?.kind));
  // Re-key the editor on the tab's STABLE uid + engine so it rebuilds cleanly
  // per query tab — and so its undo history (lib/editor-history.ts) keys on an
  // identity that survives remounts and reloads (`tab.id` is renumbered).
  const editorPath = $derived(`query-${tab.uid}.${lang}`);
  /** Shared-store history key for an editor path: `dbtab:<uid>`; masked tabs
   *  keep history in memory only (it holds every pasted string). */
  function historyKeyFor(p: string): { key: string; persist: boolean } | null {
    const m = /^query-(.+)\.[a-z]+$/.exec(p);
    if (!m) return null;
    const t = database.tabByUid(m[1]);
    if (!t) return null; // closed tab — nothing to keep
    return { key: TAB_HISTORY_PREFIX + t.uid, persist: !t.mask };
  }
  let codeEditor: ReturnType<typeof CodeEditor> | undefined = $state();
  let editHostEl: HTMLDivElement | undefined = $state();
  /** ⌥⌘V / toolbar: pick a copy made in Otto and insert it at the cursor. */
  async function openClipPicker(anchor?: Element | null): Promise<void> {
    await clipHistory.load();
    const entries = clipHistory.entries;
    const oneLine = (t: string): string => {
      const flat = t.replace(/\s+/g, ' ').trim();
      return flat.length > 80 ? `${flat.slice(0, 79)}…` : flat;
    };
    ctxMenu.showAt(
      anchor ?? editHostEl,
      entries.length === 0
        ? [{ label: clipHistory.enabled ? 'Nothing copied in Otto yet' : 'Clipboard history is off (Settings → Appearance)', disabled: true }]
        : [
            ...entries.map((e) => ({
              label: oneLine(e.text),
              icon: e.pinned ? 'pin' : 'copy',
              hint: e.source,
              title: e.text.length > 400 ? `${e.text.slice(0, 400)}…` : e.text,
              action: () => codeEditor?.insertText(e.text),
            })),
            { separator: true, pinned: true },
            { label: 'Clear unpinned', icon: 'trash', pinned: true, action: () => clipHistory.clear() },
          ],
      { filter: entries.length > 0, filterPlaceholder: 'Filter copies made in Otto…', maxVisible: 30 },
    );
  }
  // Statement separator + lexing: redis is one command per line; others use
  // `;`, lexed per engine (Postgres: dollar quotes, no `#` comments, `\` only
  // escapes in E'…') — `sql-dialect.ts`. Variables render per engine too.
  const splitMode = $derived<SplitMode>(
    database.queryLanguage === 'redis' ? 'line' : splitModeFor(database.selectedConn?.kind),
  );
  // Live selection + cursor (from CodeEditor) → run only the selected/current
  // statement instead of the whole buffer. Plain (non-reactive) on purpose:
  // only run() reads it, and `text` is a lazy getter — nothing re-renders or
  // copies the selection per cursor move. CodeEditor re-announces it whenever
  // it switches documents (tab switch), so it never describes another tab.
  let editorSel: { readonly text: string; cursor: number } = { text: '', cursor: 0 };

  // ── Buffer analysis (statement count, variables, mongosh-script) ─────────
  // Derived from a SETTLED copy of the buffer, not per keystroke: above
  // ANALYZE_SYNC_MAX chars it recomputes only after ANALYZE_IDLE_MS of quiet
  // (one allocation-free pass — `analyzeStatement`). It only drives the "Run
  // all N" button, the Variables bar and the script notice; Run / Run all /
  // Format re-derive from the ACTUAL statement when they execute.
  const ANALYZE_SYNC_MAX = 16_384;
  const ANALYZE_IDLE_MS = 150;
  interface BufferAnalysis {
    count: number;
    vars: string[];
    script: boolean;
  }
  let analysis = $state.raw<BufferAnalysis>({ count: 0, vars: [], script: false });
  let analyzedTabId = -1;
  $effect(() => {
    const text = tab.statement;
    const mode = splitMode;
    const mongo = database.queryLanguage === 'mongo';
    const id = tab.id;
    const apply = (): void => {
      const r = analyzeStatement(text, mode);
      const script = mongo && looksLikeMongoshScript(text);
      const prev = untrack(() => analysis);
      analyzedTabId = id;
      // Keep the identity when nothing changed so `{#each queryVars}` and the
      // toolbar don't re-render on every settle.
      if (r.count === prev.count && script === prev.script && sameList(r.vars, prev.vars)) return;
      analysis = { count: r.count, vars: r.vars, script };
    };
    // A tab switch (or a small buffer) is analysed at once — a new tab must
    // never show the previous tab's Variables bar.
    if (text.length <= ANALYZE_SYNC_MAX || id !== analyzedTabId) {
      apply();
      return;
    }
    const h = setTimeout(apply, ANALYZE_IDLE_MS);
    return () => clearTimeout(h);
  });
  function sameList(a: string[], b: string[]): boolean {
    return a.length === b.length && a.every((v, i) => v === b[i]);
  }
  /** Synchronous mongosh-script check for the run paths (the settled
   *  `isMongoshScript` may lag the buffer by ANALYZE_IDLE_MS). */
  function isScriptNow(): boolean {
    return database.queryLanguage === 'mongo' && looksLikeMongoshScript(tab.statement);
  }
  // Variables the current tab's statement references (:name / {name}).
  const queryVars = $derived(analysis.vars);
  let varsBarEl = $state<HTMLElement | null>(null);
  // Run-time prompt for query variables without a value (VarsPrompt): the
  // base statement is kept so the run resumes once the values are filled in.
  let varsPrompt = $state<{ names: string[]; base: string } | null>(null);
  // Opening a saved query that references placeholders pre-opens the prompt
  // (`openSavedQuery` lives in the store, so this hooks the tab's savedQueryId).
  let prevSavedId: string | undefined;
  $effect(() => {
    const id = tab.savedQueryId;
    if (id && id !== prevSavedId) {
      const names = extractVars(tab.statement, splitMode);
      const missing = names.filter((n) => !(tab.vars[n]?.value ?? '').trim());
      if (missing.length > 0) varsPrompt = { names: missing, base: tab.statement };
    }
    prevSavedId = id;
  });

  // Mongosh SCRIPT notice: when the buffer is real JavaScript (consts,
  // functions, control flow — mirrors the daemon's detection), Run executes it
  // through the actual `mongosh` CLI. Probe the daemon for the binary the
  // FIRST time a script is detected so a missing mongosh becomes an inline
  // install hint here, before the run — never a surprise run-time error.
  const isMongoshScript = $derived(analysis.script);
  let mongoshInfo = $state<MongoshInfo | null>(null);
  /** Why the probe itself failed (a timeout, a 403, an unreachable daemon) —
   *  kept apart from `mongoshInfo` so a failed CHECK never reads as "mongosh
   *  is not installed" and tells the person to install what they may have. */
  let mongoshProbeError = $state<string | null>(null);
  let mongoshProbed = false;
  async function probeMongosh(): Promise<void> {
    mongoshInfo = null; // back to "checking…" while the re-probe runs
    mongoshProbeError = null;
    try {
      mongoshInfo = await api.get<MongoshInfo>('/db/mongosh');
    } catch (e) {
      mongoshProbeError = loadErrorText(e);
    }
  }
  $effect(() => {
    if (!isMongoshScript || mongoshProbed) return;
    mongoshProbed = true;
    void probeMongosh();
  });

  // ── Tab labels ────────────────────────────────────────────────────────────
  // Derive a short, human label from a tab's SQL: prefer an explicit user name,
  // else the table after FROM/INTO/UPDATE, else a leading keyword snippet,
  // falling back to "Query N".
  function tabLabel(t: QueryTab, index: number): string {
    if (t.name && t.name !== 'Query') return t.name;
    // The head of the buffer is enough for a label — never regex-scan a pasted
    // 150 KB script on every keystroke.
    const sql = t.statement.slice(0, 2048).trim();
    if (!sql) return `Query ${index + 1}`;
    const from = sql.match(/\b(?:from|into|update|join)\s+`?([\w.$]+)`?/i);
    if (from) {
      const obj = from[1].split('.').pop() ?? from[1];
      const verb = sql.match(/^\s*(\w+)/)?.[1]?.toUpperCase() ?? '';
      return verb && verb !== 'SELECT' ? `${verb} ${obj}` : obj;
    }
    const verb = sql.match(/^\s*(\w+)/)?.[1];
    if (verb) return verb.length > 18 ? `${verb.slice(0, 18)}…` : verb;
    return `Query ${index + 1}`;
  }

  // Inline rename (double-click a chip).
  let renaming = $state<number | null>(null);
  let renameText = $state('');
  function startRename(i: number, t: QueryTab): void {
    renaming = i;
    renameText = t.name && t.name !== 'Query' ? t.name : tabLabel(t, i);
  }
  function commitRename(t: QueryTab): void {
    const v = renameText.trim();
    if (v) {
      t.name = v;
      // Persist the rename — the store's tab persistence is private, but
      // switchTab to the SAME index is a no-op that flushes it.
      database.switchTab(database.activeTab);
    }
    renaming = null;
  }

  // ── Reopen closed tab (⇧⌥⌘W) ─────────────────────────────────────────────
  // ⌥⌘W / ✕ discard an unsaved buffer instantly; keep the last few closed tab
  // payloads in memory so a mis-close is recoverable for the session.
  interface ClosedTabPayload {
    name: string;
    statement: string;
    vars: QueryTab['vars'];
    timeout_ms: number | null;
    mask: boolean;
    savedQueryId?: string;
    viewMode: QueryTab['viewMode'];
  }
  const MAX_CLOSED = 10;
  let closedTabs = $state<ClosedTabPayload[]>([]);
  /** Push the non-empty buffers among `tabs` onto the reopen stack (oldest out). */
  function rememberClosed(tabs: QueryTab[]): void {
    const worth = tabs
      .filter((t) => t.statement.trim())
      .map((t) => ({
        name: t.name,
        statement: t.statement,
        vars: { ...t.vars },
        timeout_ms: t.timeout_ms ?? null,
        mask: !!t.mask,
        savedQueryId: t.savedQueryId,
        viewMode: t.viewMode ?? null,
      }));
    if (worth.length) closedTabs = [...closedTabs, ...worth].slice(-MAX_CLOSED);
  }
  /** Un-applied grid edits a closing tab would lose: the live grid's for the
   *  active tab, the parked ones (grid-tab-state) for the others. */
  function unappliedEdits(closing: QueryTab[]): number {
    const active = database.tab;
    return closing.reduce(
      (n, t) => n + (t === active ? database.livePendingEdits : parkedEditCount(t.uid)),
      0,
    );
  }
  /** Ask before a close throws away un-applied cell edits. */
  async function okToDiscard(closing: QueryTab[]): Promise<boolean> {
    const n = unappliedEdits(closing);
    if (n === 0) return true;
    return confirmer.ask(
      `${plural(n, 'un-applied change')} in the results ${n === 1 ? 'is' : 'are'} not saved to the database and will be lost.`,
      { title: `Discard ${plural(n, 'change')}?`, confirmLabel: 'Discard', danger: true },
    );
  }
  async function closeTabAt(i: number): Promise<void> {
    const t = database.tabs[i];
    if (t && !t.pinned && !(await okToDiscard([t]))) return;
    if (database.tabs[i] !== t) return; // the strip changed while asking
    // A pinned tab doesn't close (the store toasts "Unpin to close") — so it
    // must not land on the reopen stack as if it had.
    if (t && !t.pinned) rememberClosed([t]);
    database.closeTab(i);
  }
  // Bulk closes from the tab strip menu: pinned tabs always survive.
  async function closeOthersAt(i: number): Promise<void> {
    const keep = database.tabs[i];
    const closing = database.tabs.filter((x, idx) => idx !== i && !x.pinned);
    if (!(await okToDiscard(closing))) return;
    const at = database.tabs.indexOf(keep);
    if (at < 0) return;
    rememberClosed(database.tabs.filter((x, idx) => idx !== at && !x.pinned));
    database.closeOtherTabs(at);
  }
  async function closeAll(): Promise<void> {
    if (!(await okToDiscard(database.tabs.filter((x) => !x.pinned)))) return;
    rememberClosed(database.tabs.filter((x) => !x.pinned));
    database.closeAllTabs();
  }
  function reopenClosedTab(): void {
    const payload = closedTabs[closedTabs.length - 1];
    if (!payload) return;
    closedTabs = closedTabs.slice(0, -1);
    database.newTab(payload.statement);
    const nt = database.tab;
    nt.name = payload.name;
    nt.vars = payload.vars;
    nt.timeout_ms = payload.timeout_ms;
    nt.mask = payload.mask;
    nt.savedQueryId = payload.savedQueryId;
    nt.viewMode = payload.viewMode;
    database.switchTab(database.activeTab); // same-index no-op → persists tabs
  }

  // Tab strip right-click: pin/unpin, rename, close others / close all. Pinned
  // tabs survive the bulk closes, and the labels say so whenever a pin exists.
  function tabMenu(e: MouseEvent, i: number, t: QueryTab): void {
    const anyPinned = database.tabs.some((x) => x.pinned);
    const keeps = anyPinned ? ' (keeps pinned tabs)' : '';
    const others = database.tabs.filter((x, idx) => idx !== i && !x.pinned).length;
    ctxMenu.show(e, [
      {
        label: t.pinned ? 'Unpin tab' : 'Pin tab',
        icon: 'pin',
        action: () => database.togglePinTab(i),
      },
      { label: 'Rename', icon: 'edit', action: () => startRename(i, t) },
      ...(database.supportsBuilder
        ? [
            {
              label: 'Open in Builder',
              icon: 'layers' as const,
              disabled: !t.statement.trim(),
              action: () => database.openInBuilder(t.statement),
            },
          ]
        : []),
      { separator: true },
      { label: `Close others${keeps}`, disabled: others === 0, action: () => void closeOthersAt(i) },
      { label: `Close all${keeps}`, action: () => void closeAll() },
    ]);
  }

  // Map server completion kinds → CodeMirror completion "type" (drives the icon).
  function cmType(kind: DbCompletionKind): string {
    switch (kind) {
      case 'keyword':
        return 'keyword';
      case 'function':
        return 'function';
      case 'table':
      case 'view':
      case 'collection':
        return 'class';
      case 'column':
      case 'field':
        return 'property';
      case 'database':
        return 'namespace';
      case 'command':
        return 'method';
      case 'operator':
        return 'operator';
      default:
        return 'variable';
    }
  }

  // Word boundary the completion replaces (identifiers, incl. dotted prefixes).
  const TOKEN_RE = /[\w$.]*$/;
  /** Quiet time after the last keystroke before a completion request goes out
   *  (on top of CodeMirror's own activate-on-typing delay). */
  const COMPLETION_DELAY_MS = 60;
  /** The completion request carries at most this much text around the cursor
   *  — the daemon only analyses the statement the cursor is in. */
  const CTX_BEFORE = 8192;
  const CTX_AFTER = 2048;
  /** True when the cursor sits in a literal/comment where completion is noise
   *  (e.g. editing INSERT … VALUES data) — see `completion-gate.ts`. */
  function inInertNode(state: EditorState, pos: number): boolean {
    return isInertAt(state, pos, database.queryLanguage === 'mongo');
  }

  /**
   * The span sent to the daemon: the top-level statement around the cursor
   * (lang-sql `Statement`, JS `ExpressionStatement`; the line for redis),
   * clamped to CTX_BEFORE/CTX_AFTER. Never the whole buffer — that shipped a
   * 150 KB script twice per completion request.
   */
  function completionSpan(state: EditorState, pos: number): { from: number; to: number } {
    const doc = state.doc;
    let from = Math.max(0, pos - CTX_BEFORE);
    let to = Math.min(doc.length, pos + CTX_AFTER);
    let stmtFrom = -1;
    if (splitMode === 'line') {
      const line = doc.lineAt(pos);
      stmtFrom = line.from;
      to = Math.min(to, line.to);
    } else {
      let n = syntaxTree(state).resolveInner(pos, -1);
      while (n.parent && n.parent.parent) n = n.parent;
      if (n.parent) {
        stmtFrom = n.from;
        to = Math.min(to, n.to);
      }
    }
    if (stmtFrom >= from) {
      from = stmtFrom;
    } else if (from > 0) {
      // Clamped inside a long statement: start at the next line so the window
      // doesn't open mid-token (bounded — never walk back to a far line start).
      const line = doc.lineAt(from);
      if (line.from !== from && line.to < pos) from = line.to + 1;
    }
    return { from, to };
  }

  // Server-driven completion source. Each query waits COMPLETION_DELAY_MS, then
  // asks the daemon once. It is NOT aborted by ordinary typing (no
  // `onDocChange`): CodeMirror keeps a pending query alive while the word grows
  // and then filters its answer through `validFor`, so a word typed during the
  // request costs ONE request instead of an abort + a fresh request per key.
  // (The daemon builds a cold schema snapshot in a detached task, so even a
  // dropped request's build completes and is cached.) CodeMirror still aborts
  // it — timer + fetch — on a cursor jump, Escape or a non-typing edit. A
  // `truncated` answer (the daemon capped the list for the typed word) has no
  // `validFor`, so the next keystroke re-asks with the longer word.
  let inflight: AbortController | null = null;
  // CodeMirror can destroy a pending completion without dispatching its context
  // abort. Cancel our debounce too, before it reads this editor's derived mode.
  let completionDisposed = false;
  onDestroy(() => {
    completionDisposed = true;
    inflight?.abort();
  });
  function completionSource(ctx: CompletionContext): Promise<CompletionResult | null> | null {
    if (completionDisposed) return null;
    const before = ctx.matchBefore(TOKEN_RE);
    const word = before?.text ?? '';
    // Only auto-open when there's a token or the user explicitly triggered.
    if (!ctx.explicit && word.length === 0) return null;
    // Numbers, string values and comments: nothing to complete while TYPING.
    // An explicit Ctrl-Space still completes there (a column name inside a
    // Mongo `$expr` string, a dynamic-SQL string, a mis-tokenized region).
    if (!ctx.explicit && (/^\d/.test(word) || inInertNode(ctx.state, ctx.pos))) return null;

    // Where the accepted completion is inserted, and what text the popup filters
    // against. A leading qualifier is NOT part of the identifier, so only the
    // segment after the last dot is replaced/matched (the qualifier stays, labels
    // are bare). This applies to:
    //   • SQL `alias.`/`db.`/`table.` qualifiers, and
    //   • Mongo's `db.` / `db.<coll>.` SELECTOR (collection / method completion).
    // But a Mongo embedded FIELD path (`addr.city`, inside `find({…})`) IS the
    // identifier and must be replaced/matched WHOLE — else the collection labels
    // never match the typed `db.` and the popup shows nothing. Distinguish the
    // two by the `db.` selector prefix.
    let from = before ? before.from : ctx.pos;
    if (before) {
      const stripQualifier = database.queryLanguage !== 'mongo' || before.text.startsWith('db.');
      if (stripQualifier) {
        const dot = before.text.lastIndexOf('.');
        if (dot >= 0) from = before.from + dot + 1;
      }
    }

    const { state, pos } = ctx;
    return new Promise((resolve) => {
      inflight?.abort();
      const ac = new AbortController();
      inflight = ac;
      const timer = setTimeout(async () => {
        if (completionDisposed || ac.signal.aborted) { resolve(null); return; }
        // Sliced HERE, after the quiet period — not per keystroke.
        const span = completionSpan(state, pos);
        const prefix = state.sliceDoc(span.from, pos);
        // Text after the cursor lets the server resolve the FROM table list even
        // when the cursor sits in the SELECT list before it.
        const suffix = state.sliceDoc(pos, span.to);
        const { items, truncated } = await database.complete(prefix, suffix, undefined, ac.signal);
        if (inflight === ac) inflight = null;
        if (ac.signal.aborted || items.length === 0) {
          resolve(null);
          return;
        }
        const options: Completion[] = items.map((it) => ({
          label: it.label,
          type: cmType(it.kind),
          detail: it.detail ?? undefined,
          apply: it.insert_text ?? undefined,
          // Index columns/fields and in-scope tables carry a higher score, so
          // they sort above plain columns / keywords among matching options.
          boost: it.score ?? undefined,
        }));
        resolve(truncated ? { from, options } : { from, options, validFor: TOKEN_RE });
      }, COMPLETION_DELAY_MS);
      ac.signal.addEventListener('abort', () => {
        clearTimeout(timer);
        if (inflight === ac) inflight = null;
        resolve(null);
      }, { once: true });
      ctx.addEventListener('abort', () => ac.abort());
    });
  }

  // Substitute query-level variables into `base` and execute it (transient — the
  // tab's text is never overwritten by the rendered SQL). Shared by Run
  // (selection / statement-at-cursor) and Run all (the whole buffer as a batch).
  function execBase(base: string): void {
    if (!canQuery) return;
    if (!base.trim()) {
      void database.runQuery();
      return;
    }
    // Query-level variables: every :name / {name} in the chosen statement needs
    // a value before it can run.
    const names = extractVars(base, splitMode);
    const missing = names.filter((n) => !(tab.vars[n]?.value ?? '').trim());
    if (missing.length > 0) {
      varsPrompt = { names: missing, base };
      return;
    }
    // Render each variable per its type/escape (string → quoted+escaped, number →
    // raw, raw → verbatim), then substitute.
    const rendered = Object.fromEntries(
      names.map((n) => [n, renderVar(tab.vars[n] ?? defaultVarSpec(), splitMode)]),
    );
    const finalSql = names.length > 0 ? substituteVars(base, rendered, splitMode) : base;
    void database.runQuery(finalSql, undefined, { transient: true });
  }

  const canQuery = $derived(!!database.selectedConnId && ['db_query','db_data','db_schema'].some(op=>resourceAccess.can('connection',database.selectedConnId!,op,'database','edit',databaseAccessChild(database.activeDb))));
  // Why Run is disabled — a disabled button must say why (the normal tooltip
  // describes what Run WOULD do, which reads as a dead button here).
  const noQueryReason = $derived(
    database.selectedConnId
      ? 'You have no query access to this connection (or this database) — ask its owner for access'
      : 'Pick a connection first',
  );
  $effect(()=>{if(database.selectedConnId)void resourceAccess.load('connection',database.selectedConnId,databaseAccessChild(database.activeDb));});
  function run(): void {
    if (!canQuery) return;
    // While a query is running, ⌘↵ (and the editor's submit) must NOT silently
    // abort-and-restart — stopping is an explicit act (the Stop button / Esc).
    if (tab.running) return;
    // A mongosh SCRIPT is INDIVISIBLE. `statementAtCursor` cuts on top-level
    // `;`, which for real JavaScript slices the file into fragments that share
    // no scope — running one alone either does nothing visible (the leading
    // `const …;`) or dies on an undefined variable from an earlier fragment.
    // With no explicit selection, Run therefore executes the whole buffer,
    // exactly what the script notice above the editor promises.
    if (isScriptNow() && !editorSel.text.trim()) {
      execBase(tab.statement);
      return;
    }
    // Run the selection if there is one, else the statement under the cursor.
    execBase(
      editorSel.text.trim()
        ? editorSel.text
        : statementAtCursor(tab.statement, editorSel.cursor, splitMode),
    );
  }

  // Run the WHOLE buffer as one multi-statement batch (the backend splits it and
  // returns one result set per statement — the grid shows a result switcher).
  const stmtCount = $derived(analysis.count);
  /** Whether the buffer holds anything but whitespace — stops at the first
   *  non-space char instead of trimming the whole buffer per render. */
  const hasStatement = $derived(/\S/.test(tab.statement));
  function runAll(): void {
    if (!canQuery) return;
    execBase(tab.statement);
  }

  // "Run on…": the selection (else the whole buffer) on several targets and/or
  // once per placeholder value — MultiRunDialog owns the whole flow.
  let multiRun = $state<{ statement: string } | null>(null);
  function openMultiRun(): void {
    if (!canQuery || !hasStatement) return;
    multiRun = { statement: editorSel.text.trim() ? editorSel.text : tab.statement };
  }

  // Workbench "Send to → Database — Run on…": once this connection can query,
  // open the parked script in a NEW tab with its placeholder values and show
  // the Run on… sheet (setup stage — nothing runs without the user).
  $effect(() => {
    if (!dbHandoff.pending || !canQuery) return;
    untrack(() => {
      const p = dbHandoff.take();
      if (!p) return;
      database.newTab(p.statement);
      for (const [name, value] of Object.entries(p.vars)) {
        if (value.trim()) database.setVar(name, { value, type: prefillVarType(value) });
      }
      multiRun = { statement: p.statement };
    });
  });

  // Draggable split between the editor and the results.
  //
  // The height is remembered PER TAB, not once for the whole view: a tab holding
  // a 40-line query and a tab holding `SELECT 1` want very different splits, and
  // one shared pixel value made every tab fight over it. Until a tab has
  // produced a result there is nothing to show below, so the editor takes the
  // pane and that large dead grey area simply doesn't exist.
  const perTabH = new Map<number, number>();
  let editorH = $state(240);
  let resizing = $state(false);
  // Height to restore when collapsing the maximize toggle.
  let prevEditorH = $state(0);

  /** Fraction of the pane the editor gets before a tab has any result. */
  const UNRUN_FRACTION = 0.85;

  function defaultEditorH(hasResult: boolean): number {
    const max = maxEditorH();
    if (!hasResult) return max;
    const vh = typeof window !== 'undefined' ? window.innerHeight : 1000;
    // Fit the statement: a one-line `SELECT * FROM t` (a table opened from the
    // tree) gets a compact editor and the grid gets the room; a long query
    // gets up to ~42% of the window.
    const lines = (tab?.statement ?? '').split('\n').length;
    const fit = 56 + Math.min(lines, 30) * 19;
    return Math.max(132, Math.min(max, Math.round(vh * 0.42), fit));
  }

  // Adopt this tab's remembered height whenever the active tab changes, and
  // shrink the editor off its "nothing to show yet" size the first time the tab
  // RUNS — not when its result lands. Shrinking on arrival moved the whole
  // results area up by a third of the window at the exact moment the rows
  // appeared (the "table jumps when I open it" bug); shrinking at Run time
  // lets the results frame (loading skeleton → rows) sit still.
  let lastTabId = -1;
  let lastHadResult = false;
  $effect(() => {
    const id = tab?.id ?? -1;
    // Explain also needs the output pane before this tab has run a query.
    const hasResult = !!(tab?.result || tab?.running || tab?.error || database.planOpen);
    if (id !== lastTabId) {
      lastTabId = id;
      lastHadResult = hasResult;
      editorH = perTabH.get(id) ?? loadEditorH(hasResult);
      return;
    }
    if (hasResult && !lastHadResult) {
      lastHadResult = true;
      // Only auto-shrink a tab the user has never sized by hand.
      if (!perTabH.has(id)) editorH = defaultEditorH(true);
    } else if (!hasResult && lastHadResult) {
      lastHadResult = false;
    }
  });

  function loadEditorH(hasResult: boolean): number {
    if (typeof localStorage === 'undefined') return defaultEditorH(hasResult);
    const v = Number(localStorage.getItem('db.editorH'));
    if (!hasResult) return defaultEditorH(false);
    return Number.isFinite(v) && v > 80 ? Math.min(v, maxEditorH()) : defaultEditorH(true);
  }
  // Tallest the editor may grow to — viewport-relative so it can take most of the
  // pane on big screens, while always reserving room for the results grid so it
  // can never be squeezed off-screen. (≥180 keeps it sane on short windows.)
  function maxEditorH(): number {
    const vh = typeof window !== 'undefined' ? window.innerHeight : 1000;
    return Math.max(180, vh - 260);
  }
  // Soft-wrap long lines in the editor. ON by default: unwrapped long lines
  // (a pasted INSERT dump) make every keystroke re-lay out every rendered line
  // — 20–41 ms/key in WebKit at 150 KB, vs ~3 ms wrapped. Per device, like the
  // editor height.
  const WRAP_KEY = 'db.editorWrap';
  let wrapLines = $state(loadWrap());
  function loadWrap(): boolean {
    try {
      return localStorage.getItem(WRAP_KEY) !== '0';
    } catch {
      return true;
    }
  }
  function setWrap(on: boolean): void {
    wrapLines = on;
    try {
      localStorage.setItem(WRAP_KEY, on ? '1' : '0');
    } catch {
      /* ignore */
    }
  }

  function persistEditorH(): void {
    // Per-tab in memory (the tab itself is session state), plus one global
    // fallback on disk so a fresh tab opens at a size you already liked.
    if (tab) perTabH.set(tab.id, editorH);
    try {
      localStorage.setItem('db.editorH', String(Math.round(editorH)));
    } catch {
      /* ignore */
    }
  }
  function startResize(e: PointerEvent): void {
    e.preventDefault();
    resizing = true;
    const startY = e.clientY;
    const startH = editorH;
    const onMove = (ev: PointerEvent): void => {
      editorH = Math.max(100, Math.min(maxEditorH(), startH + (ev.clientY - startY)));
    };
    const onUp = (): void => {
      resizing = false;
      persistEditorH();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    // A cancelled gesture (system gesture, focus loss) ends the drag too —
    // else pointermove stays attached and the pane follows the cursor.
    window.addEventListener('pointercancel', onUp);
  }
  function setEditorH(h: number): void {
    editorH = h;
    persistEditorH();
  }
  // Double-click the grip to expand the editor to (nearly) full height, and again
  // to restore the prior height — a quick way to focus on a long query.
  function toggleExpand(): void {
    const max = maxEditorH();
    if (editorH < max - 8) {
      prevEditorH = editorH;
      editorH = max;
    } else {
      editorH = prevEditorH > 80 ? prevEditorH : defaultEditorH(!!tab?.result);
    }
    persistEditorH();
  }
  /** ⇧⌘E — same toggle, reachable from the keyboard. */
  function toggleMaxEditor(): void {
    toggleExpand();
  }

  // ── Save query ──────────────────────────────────────────────────────────
  let saving = $state(false);
  let saveName = $state('');
  let savePending = $state(false);
  /** True when this tab was opened from a saved query that still exists — then
   *  the primary Save UPDATES it in place ("Save as new" forks a fresh one). */
  const savedLinked = $derived(
    !!tab?.savedQueryId && database.savedQueries.some((q) => q.id === tab.savedQueryId),
  );
  async function openSave(): Promise<void> {
    if (savePending) return;
    saveName = tab.name && tab.name !== 'Query' ? tab.name : '';
    saving = true;
    await tick();
  }
  /** Primary save: update the linked saved query in place, else create a new one
   *  (a name is required only for the create case). */
  async function confirmSave(): Promise<void> {
    if (savePending || (!savedLinked && !saveName.trim())) return;
    savePending = true;
    try {
      const saved = await database.saveActiveTab(saveName);
      if (saved) { saving = false; saveName = ''; }
    } finally { savePending = false; }
  }
  /** Always create a fresh saved query from the current statement (a name is
   *  required). Lets the user fork a saved query without overwriting it. */
  async function confirmSaveAsNew(): Promise<void> {
    const name = saveName.trim();
    if (savePending || !name) return;
    savePending = true;
    try {
      const saved = await database.saveQuery(name, tab.statement);
      if (saved) { saving = false; saveName = ''; }
    } finally { savePending = false; }
  }

  const canEdit = $derived(ws.myRole !== 'viewer');

  // ── DB Assistant entry points ────────────────────────────────────────────────
  // "Ask AI" (free-form) and "Ask in English" (draft a query) both OPEN the
  // embedded DB Assistant panel beside the editor — a real, file-backed agent that
  // runs read-only against the DB in its own live shell (see DbAssistantPanel).
  // This replaces the old inline NL→SQL drafter (which hung on a folder-trust
  // dialog and got no schema context).

  // ── Phone accordion ────────────────────────────────────────────────────────
  // On a phone the editor and the results are each collapsible, independently
  // scrolling blocks (tap a header to expand/minimise). Default: editor open,
  // results auto-open once a query has produced something. Inert on desktop —
  // the headers only render when isPhone.
  let editorOpen = $state(true);
  let resultsOpen = $state(true);
  const hasResult = $derived(!!tab.result || !!tab.error);
  // A first Explain is itself the output. The idle grid's empty-state sizing
  // must not compete with it; real rows/errors/loading still share the pane.
  const planOnly = $derived(database.planOpen && !!database.queryPlan && !tab.result && !tab.error && !tab.running);

  // ── Keyboard shortcuts (active only while the DB query view owns focus) ───────
  // Registered on window but gated to fire only when focus is inside this editor
  // (or nowhere), so they never hijack another module's inputs. The component is
  // mounted only on the Query tab, so the listener is naturally scoped to it.
  let rootEl = $state<HTMLElement | null>(null);
  // The global key map (lib/keys.ts) now matches modifiers exactly, so ⌥⌘/⇧⌘
  // augmented chords don't collide with the plain-⌘ session actions (⌘T/⌘W/⌘F).
  // ⌃Tab stays the app-global session cycler; query-tab nav uses ⌥⌘→/←.
  const SHORTCUTS: { keys: string; label: string }[] = [
    { keys: '⌘↵', label: 'Run' },
    { keys: '⇧⌘↵', label: 'Run all statements' },
    { keys: '⌘S', label: 'Save query' },
    { keys: '⇧⌘F', label: 'Format' },
    { keys: '⇧⌘V', label: 'Cycle results view (Grid → Vertical → JSON)' },
    { keys: 'Esc', label: 'Cancel running query' },
    { keys: '⌥⌘→ / ⌥⌘←', label: 'Next / previous query tab' },
    { keys: '⌥⌘T', label: 'New query tab' },
    { keys: '⌥⌘W', label: 'Close query tab' },
    { keys: '⇧⌥⌘W', label: 'Reopen closed tab' },
    { keys: '⌥⌘V', label: 'Paste from Otto clipboard history' },
  ];

  function switchRelative(dir: 1 | -1): void {
    const n = database.tabs.length;
    if (n < 2) return;
    const cur = database.activeTab;
    database.switchTab(dir === 1 ? (cur + 1) % n : (cur - 1 + n) % n);
  }

  function handleShortcut(e: KeyboardEvent): void {
    const cmd = e.metaKey || e.ctrlKey; // ⌘ on macOS, Ctrl elsewhere
    // Letter chords match on e.code (physical key) so macOS Option-key character
    // composition (⌥T → "†") doesn't defeat them.
    // Esc — cancel a running query (or close the shortcuts popover). When the
    // editor's completion popup is open, Esc belongs to IT (close the popup) —
    // let CodeMirror handle it instead of killing the running query.
    if (e.key === 'Escape') {
      const completionOpen = !!rootEl?.querySelector('.cm-tooltip-autocomplete');
      if (tab.running && !completionOpen) {
        e.preventDefault();
        database.abortQuery(undefined, { report: true });
      } else if (shortcutsOpen && !completionOpen) {
        shortcutsOpen = false;
      }
      return;
    }
    // ⇧⌘↵ — run the whole buffer as a batch (⌘↵ alone = editor's run-current).
    if (cmd && e.shiftKey && !e.altKey && e.key === 'Enter') {
      e.preventDefault();
      if (!tab.running && database.selectedConnId) runAll();
      return;
    }
    // ⌘S — save the query.
    if (cmd && !e.shiftKey && !e.altKey && e.code === 'KeyS') {
      e.preventDefault();
      if (canEdit && hasStatement) void openSave();
      return;
    }
    // ⇧⌘F — format (⌘F alone stays the app-global find). A mongosh SCRIPT is
    // real JavaScript — the structural formatter collapses its newlines, which
    // breaks ASI statement boundaries — so Format is gated off for scripts.
    if (cmd && e.shiftKey && !e.altKey && e.code === 'KeyF') {
      e.preventDefault();
      if (database.queryLanguage !== 'redis' && !isScriptNow() && hasStatement && !tab.running) {
        void database.formatStatement();
        editorSel = { text: '', cursor: 0 };
      }
      return;
    }
    // ⌘B — collapse/restore the schema sidebar (the biggest single win in
    // width when you are actually writing SQL).
    if (cmd && !e.shiftKey && !e.altKey && e.code === 'KeyB') {
      e.preventDefault();
      database.toggleSidebar();
      return;
    }
    // ⇧⌘E — maximize the editor over the results pane (and back).
    if (cmd && e.shiftKey && !e.altKey && e.code === 'KeyE') {
      e.preventDefault();
      toggleMaxEditor();
      return;
    }
    // ⇧⌘V — cycle the results view Grid → Vertical → JSON (stored as the tab's
    // explicit pick). Inside the SQL editor the chord stays the browser's
    // paste-and-match-style, so it's left alone there.
    if (cmd && e.shiftKey && !e.altKey && e.code === 'KeyV') {
      if ((e.target as HTMLElement)?.closest?.('.cm-editor')) return;
      e.preventDefault();
      database.cycleViewMode(viewMode);
      return;
    }
    // ⌥⌘V — paste from the clipboard ring (copies made in Otto) at the cursor.
    if (cmd && e.altKey && !e.shiftKey && e.code === 'KeyV') {
      e.preventDefault();
      void openClipPicker();
      return;
    }
    // ⌥⌘T new query tab / ⌥⌘W close query tab (⌘T/⌘W stay session actions).
    if (e.metaKey && e.altKey && e.code === 'KeyT') {
      e.preventDefault();
      database.newTab();
      return;
    }
    if (e.metaKey && e.altKey && !e.shiftKey && e.code === 'KeyW') {
      e.preventDefault();
      if (database.tabs.length > 1) void closeTabAt(database.activeTab);
      return;
    }
    // ⇧⌥⌘W — reopen the most recently closed query tab.
    if (e.metaKey && e.altKey && e.shiftKey && e.code === 'KeyW') {
      e.preventDefault();
      reopenClosedTab();
      return;
    }
    // ⌥⌘→ / ⌥⌘← — switch query tabs (⌃Tab is the app-global session cycler).
    if (e.metaKey && e.altKey && e.key === 'ArrowRight') {
      e.preventDefault();
      switchRelative(1);
      return;
    }
    if (e.metaKey && e.altKey && e.key === 'ArrowLeft') {
      e.preventDefault();
      switchRelative(-1);
    }
  }

  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const ae = document.activeElement;
      // Only when the DB query view owns focus (or nothing else does) — never
      // steal keys destined for another pane's inputs.
      if (rootEl && ae && ae !== document.body && !rootEl.contains(ae)) return;
      handleShortcut(e);
      // When we handled it, stop the event so shell-level bindings (e.g. Ctrl+Tab)
      // don't ALSO fire. Capture phase + stopPropagation makes the DB view win
      // while it's focused.
      if (e.defaultPrevented) e.stopPropagation();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  // ── Shortcuts popover (⌨) ─────────────────────────────────────────────────────
  let shortcutsOpen = $state(false);
  let kbdWrapEl = $state<HTMLElement | null>(null);
  let kbdPopEl = $state<HTMLElement | null>(null);
  $effect(() => {
    if (!shortcutsOpen) return;
    const onDown = (e: PointerEvent): void => {
      if (kbdWrapEl && !kbdWrapEl.contains(e.target as Node)) shortcutsOpen = false;
    };
    // Esc closes it from anywhere (the editor's own Esc handler only sees keys
    // typed in the editor) and hands focus back to the ⌨ button.
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      shortcutsOpen = false;
      kbdWrapEl?.querySelector<HTMLElement>('button')?.focus();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
    };
  });
  // Clamp the popover into the viewport: never off the right/left edge, and
  // its height capped to the room below its top so the list scrolls instead
  // of running off the bottom (AGENTS.md "Floating UI must survive long content").
  $effect(() => {
    const el = kbdPopEl;
    if (!shortcutsOpen || !el) return;
    el.style.transform = '';
    const vw = window.innerWidth;
    el.style.maxWidth = `${Math.max(160, vw - 16)}px`;
    el.style.maxHeight = `${Math.max(120, window.innerHeight - el.getBoundingClientRect().top - 8)}px`;
    const r = el.getBoundingClientRect();
    let dx = 0;
    if (r.right > vw - 8) dx = vw - 8 - r.right;
    if (r.left + dx < 8) dx = 8 - r.left;
    if (dx !== 0) el.style.transform = `translateX(${dx}px)`;
  });
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {  "class":`query-editor`,});rootEl = $$_div0;
   { svelteHTML.createElement("div", {     "class":`qe-tabs scroll-thin`,"role":`tablist`,"aria-label":`Query tabs`,});
        for(let t of __sveltets_2_ensureArray(database.tabs)){let i = 1;t.id;
      
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                "class":`qe-tab`,...__sveltets_2_empty({"data-db-tab-id":t.id}),"role":`presentation`,"onclick":() => database.switchTab(i),"ondblclick":() => startRename(i, t),"oncontextmenu":(e) => tabMenu(e, i, t),});i === database.activeTab;!!t.agent;
        if(renaming === i){
           {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {                 "dir":`ltr`,"aria-label":`Rename query tab`,"class":`qe-tab-rename mono`,"bind:value":renameText,"onclick":(e) => e.stopPropagation(),"ondblclick":(e) => e.stopPropagation(),"onblur":() => commitRename(t),"onkeydown":(e) => {
              e.stopPropagation();
              if (e.key === 'Enter') commitRename(t);
              else if (e.key === 'Escape') renaming = null;
            },});/*Ωignore_startΩ*/() => renameText = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
        }else{
           { svelteHTML.createElement("button", {             "class":`qe-tab-main`,"role":`tab`,"tabindex":i === database.activeTab ? 0 : -1,"aria-selected":i === database.activeTab,"aria-keyshortcuts":database.tabs.length > 1 && !t.pinned ? 'Delete Backspace' : undefined,"onkeydown":(e) => {
              if (e.metaKey || e.ctrlKey || e.altKey) return;
              // Delete / Backspace closes the focused tab (as TabBar does);
              // a pinned or last tab has no close, so the keys do nothing.
              if (e.key === 'Delete' || e.key === 'Backspace') {
                if (database.tabs.length > 1 && !t.pinned) {
                  e.preventDefault();
                  const bar = e.currentTarget.closest('[role="tablist"]');
                  void closeTabAt(i).then(() =>
                    queueMicrotask(() => bar?.querySelectorAll<HTMLElement>('[role="tab"]')[database.activeTab]?.focus()),
                  );
                }
                return;
              }
              let next = i;
              const forward = getComputedStyle(e.currentTarget).direction === 'rtl' ? 'ArrowLeft' : 'ArrowRight';
              if (e.key === forward) next = (i + 1) % database.tabs.length;
              else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') next = (i - 1 + database.tabs.length) % database.tabs.length;
              else if (e.key === 'Home') next = 0;
              else if (e.key === 'End') next = database.tabs.length - 1;
              if (next !== i) {
                e.preventDefault();
                database.switchTab(next);
                const bar = e.currentTarget.closest('[role="tablist"]');
                queueMicrotask(() => bar?.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus());
              }
            },});
            if(t.pinned){
               { svelteHTML.createElement("span", {   "class":`qe-tab-pin`,"title":`Pinned`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`pin`,"size":12,}});} }
            }
            if(t.agent){
              
               { svelteHTML.createElement("span", {     "class":`qe-tab-agent`,"title":`Opened by ${t.agent.label}`,"aria-label":`Opened by ${t.agent.label}`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}
               }
            }
             { svelteHTML.createElement("span", { "class":`qe-tab-label`,});tabLabel(t, i); }
            if(t.running){ { svelteHTML.createElement("span", {   "class":`qe-tab-dot running`,"title":`Running`,}); }
            } else if (t.error){ { svelteHTML.createElement("span", {   "class":`qe-tab-dot error`,"title":`Error`,}); }}
           }
          if(database.tabs.length > 1 && !t.pinned){
             { svelteHTML.createElement("button", {             "class":`qe-tab-close`,"title":`Close tab`,"aria-label":`Close ${tabLabel(t, i)}`,"tabindex":i === database.activeTab ? 0 : -1,"onclick":(e) => {
                e.stopPropagation();
                void closeTabAt(i);
              },"ondblclick":(e) => e.stopPropagation(),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
          }
        }
       }}
    }
     { svelteHTML.createElement("button", {       "class":`qe-tab-new`,"title":`New query tab`,"aria-label":`New query tab`,"onclick":() => database.newTab(),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
     }
   }

   { svelteHTML.createElement("div", { "class":`qe-toolbar`,});
     { svelteHTML.createElement("div", { "class":`qe-actions`,});
    if(tab.running){
       { svelteHTML.createElement("button", {     "class":`btn small danger qe-stop`,"onclick":() => database.abortQuery(undefined, { report: true }),"title":`Stop the running query`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}
        
       }
    }else{
       { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":run,"disabled":!canQuery,"title":!canQuery
          ? noQueryReason
          : isMongoshScript
          ? 'Run the WHOLE script through mongosh — a script is indivisible, so Run never sends a single ;-delimited fragment (⌘↵)'
          : 'Run the selection, else the statement under the cursor (⌘↵)',});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}
        
         { svelteHTML.createElement("span", { "class":`kbd`,});  }
       }
      if(stmtCount > 1 && !isMongoshScript){
         { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":runAll,"disabled":!canQuery,"title":!canQuery ? noQueryReason : `Run all ${stmtCount} statements as one batch — one result set per statement (⇧⌘↵)`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}
           
         }
      }
       { svelteHTML.createElement("button", {             "class":`btn small ghost`,"onclick":openMultiRun,"disabled":!canQuery || !hasStatement,"title":!canQuery
          ? noQueryReason
          : 'Run on… — the same script on several connections / databases, or once per parameter value (preview every final statement first)',"aria-label":`Run on multiple targets`,...__sveltets_2_empty({"data-testid":`multi-run-open`}),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":12,}});} { svelteHTML.createElement("span", { "class":`btn-label`,});  }
       }
    }
     { svelteHTML.createElement("span", {   "class":`qe-sep`,"aria-hidden":`true`,}); }
    if(canEdit){
       { svelteHTML.createElement("button", {         "class":`btn small ghost`,"onclick":openSave,"disabled":!hasStatement,"title":savedLinked ? 'Update the saved query (or Save as new)' : 'Save this query',});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}savedLinked ? 'Update' : 'Save';
       }
    }
    if(database.capabilities?.explain !== false){
       { svelteHTML.createElement("button", {              "class":`btn small ghost`,"aria-expanded":database.planOpen,"onclick":() => void database.explainPlan(),"disabled":!hasStatement || tab.running,"title":`Show the query plan (EXPLAIN) — a normalized tree with cost warnings`,"aria-label":`Explain`,});database.planOpen;
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});} { svelteHTML.createElement("span", { "class":`btn-label`,});  }
       }
    }
     { svelteHTML.createElement("button", {              "class":`btn small ghost`,"aria-expanded":database.assistOpen && database.assistMode === 'ask',"onclick":() => database.openAssist('ask'),"disabled":!canQuery || !auth.can('agents','edit'),"title":!canQuery || !auth.can('agents','edit') ? 'Needs agent access and query permission on this connection' : 'Ask the DB Assistant about this connection — it opens beside the editor',"aria-label":`Ask Otto`,});database.assistOpen && database.assistMode === 'ask';
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});} { svelteHTML.createElement("span", { "class":`btn-label`,});  }
     }
     { svelteHTML.createElement("button", {              "class":`btn small ghost`,"aria-expanded":database.assistOpen && database.assistMode === 'nl',"onclick":() => database.openAssist('nl'),"disabled":!canQuery || !auth.can('agents','edit'),"title":!canQuery || !auth.can('agents','edit') ? 'Needs agent access and query permission on this connection' : 'Describe what you want in plain English — the DB Assistant drafts a query you can insert or run',"aria-label":`Ask in English`,});database.assistOpen && database.assistMode === 'nl';
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`comment`,"size":12,}});} { svelteHTML.createElement("span", { "class":`btn-label`,});   }
     }
    if(database.queryLanguage !== 'redis'){
       { svelteHTML.createElement("button", {           "class":`btn small ghost`,"onclick":() => {
          void database.formatStatement();
          editorSel = { text: '', cursor: 0 };
        },"disabled":!hasStatement || tab.running || isMongoshScript,"title":isMongoshScript
          ? 'Format is disabled for mongosh scripts — reflowing real JavaScript breaks its statement boundaries'
          : 'Format / beautify the SQL',"aria-label":`Format`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`format`,"size":12,}});} { svelteHTML.createElement("span", { "class":`btn-label`,});  }
       }
    }
     { const $$_div3 = svelteHTML.createElement("div", {  "class":`qe-kbd`,});kbdWrapEl = $$_div3;
       { svelteHTML.createElement("button", {              "class":`icon-btn`,"onclick":() => (shortcutsOpen = !shortcutsOpen),"title":`Keyboard shortcuts`,"aria-label":`Keyboard shortcuts`,"aria-expanded":shortcutsOpen,"aria-haspopup":`dialog`,});shortcutsOpen; { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`command`,"size":13,}});} }
      if(shortcutsOpen){
        
         { const $$_div4 = svelteHTML.createElement("div", {      "class":`qe-kbd-pop`,"role":`dialog`,"aria-labelledby":`qe-kbd-title`,});kbdPopEl = $$_div4;
           { svelteHTML.createElement("h3", {   "class":`qe-kbd-title`,"id":`qe-kbd-title`,});  }
             for(let s of __sveltets_2_ensureArray(SHORTCUTS)){s.label;
             { svelteHTML.createElement("div", { "class":`qe-kbd-row`,});
               { svelteHTML.createElement("span", { "class":`qe-kbd-label`,});s.label; }
               { svelteHTML.createElement("kbd", { "class":`qe-kbd-keys`,});s.keys; }
             }
          }
         }
      }
     }
     }
     { svelteHTML.createElement("div", { "class":`qe-settings`,});
    if(database.capabilities?.sql && database.databaseNames.length > 0){
       { svelteHTML.createElement("label", {   "class":`qe-db`,"title":`Active database — queries run scoped to it, so you don’t need a db. prefix`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":12,}});}
         { svelteHTML.createElement("select", {       "class":`input`,"value":database.activeDb ?? '',"onchange":(e) => database.setActiveDb((e.currentTarget as HTMLSelectElement).value || null),});
           { svelteHTML.createElement("option", {"value":"",});   }
             for(let db of __sveltets_2_ensureArray(database.databaseNames)){db;
             { svelteHTML.createElement("option", { "value":db,});db; }
          }
         }
       }
    } else if (database.isRedis && database.keyspaces.length > 0){
       { svelteHTML.createElement("label", {   "class":`qe-db`,"title":`Active Redis database — commands (GET, HGETALL, …) run against this DB`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":12,}});}
         { svelteHTML.createElement("select", {       "class":`input`,"value":database.activeDb ?? database.keyspaces[0]?.id ?? '',"onchange":(e) => database.setActiveDb((e.currentTarget as HTMLSelectElement).value || null),});
             for(let ks of __sveltets_2_ensureArray(database.keyspaces)){ks.id;
             { svelteHTML.createElement("option", { "value":ks.id,});ks.label; }
          }
         }
       }
    }
     { svelteHTML.createElement("label", {   "class":`qe-limit`,"title":`Default row cap — an explicit LIMIT in your query overrides this`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("select", {       "class":`input`,"value":database.rowLimit,"onchange":(e) => database.setRowLimit(Number((e.currentTarget as HTMLSelectElement).value)),});
           for(let o of __sveltets_2_ensureArray(ROW_LIMIT_OPTS)){o.value;
           { svelteHTML.createElement("option", { "value":o.value,});o.label; }
        }
       }
     }
     { svelteHTML.createElement("label", {   "class":`qe-timeout`,"title":`Per-statement timeout (ms) — the engine stops the statement past it; 0 or blank = no limit`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {               "class":`input qe-timeout-input`,"type":`number`,"min":`0`,"step":`1000`,"placeholder":`ms`,"value":tab.timeout_ms ?? '',"oninput":(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          database.tab.timeout_ms = v > 0 ? v : null;
        },});}
     }
     { svelteHTML.createElement("label", {      "class":`qe-mask`,"title":`Mask PII/prod — server redacts sensitive values (emails, tokens, keys) before returning results`,});tab.mask;
       { svelteHTML.createElement("input", {         "type":`checkbox`,"class":`sr-only`,"checked":tab.mask,"onchange":(e) => {
          const on = (e.currentTarget as HTMLInputElement).checked;
          database.tab.mask = on;
          // A masked tab's undo history (every pasted string) must not stay on disk.
          if (on) unpersistEditorState(TAB_HISTORY_PREFIX + database.tab.uid);
        },});}
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}
      if(tab.mask){ { svelteHTML.createElement("strong", {});  }}else{ { svelteHTML.createElement("span", {});  }}
     }
     { svelteHTML.createElement("label", {      "class":`qe-mask qe-wrap`,"title":wrapLines
        ? 'Long lines wrap (fast with big pasted scripts) — turn off to scroll long lines horizontally'
        : 'Long lines scroll horizontally — wrapping keeps typing fast in big pasted scripts',});wrapLines;
       { svelteHTML.createElement("input", {         "type":`checkbox`,"class":`sr-only`,"checked":wrapLines,"onchange":(e) => setWrap((e.currentTarget as HTMLInputElement).checked),});}
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":12,}});}
       { svelteHTML.createElement("span", {});  }
     }
     { svelteHTML.createElement("button", {             "class":`qe-mask qe-clip`,"type":`button`,"onclick":(e) => void openClipPicker(e.currentTarget),"aria-label":`Paste from clipboard history`,"title":`Paste from clipboard history — copies made in Otto (⌥⌘V)`,...__sveltets_2_empty({"data-testid":`qe-clip-history`}),});
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}
     }
     { svelteHTML.createElement("span", {   "class":`qe-lang mono`,"title":`Query language`,});database.queryLanguage; }
     }
   }

  if(queryVars.length > 0){
     { const $$_div1 = svelteHTML.createElement("div", {  "class":`qe-vars`,});varsBarEl = $$_div1;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}
       { svelteHTML.createElement("span", { "class":`qe-vars-label`,});  }
         for(let name of __sveltets_2_ensureArray(queryVars)){name;
        const spec = tab.vars[name] ?? defaultVarSpec();
         { svelteHTML.createElement("div", { "class":`qe-var`,});
           { svelteHTML.createElement("span", { "class":`qe-var-name mono`,});name; }
           { svelteHTML.createElement("input", {                "dir":`auto`,"aria-label":`Value for ${name}`,"class":`input qe-var-input`,"value":spec.value,"placeholder":spec.type === 'number' ? '123' : 'value',"spellcheck":`false`,"oninput":(e) =>
              database.setVar(name, { value: (e.currentTarget as HTMLInputElement).value }),"onkeydown":(e) => {
              if (e.key === 'Enter') run();
            },});}
           { svelteHTML.createElement("select", {         "class":`input qe-var-type`,"value":spec.type,"title":`How to substitute this variable: string (quoted), number (raw), or raw (verbatim)`,"onchange":(e) =>
              database.setVar(name, {
                type: (e.currentTarget as HTMLSelectElement).value as VarSpec['type'],
              }),});
             { svelteHTML.createElement("option", { "value":`string`,});  }
             { svelteHTML.createElement("option", { "value":`number`,});  }
             { svelteHTML.createElement("option", { "value":`raw`,});  }
           }
          if(spec.type === 'string'){
             { svelteHTML.createElement("label", {   "class":`qe-var-esc`,"title":`Escape quotes inside the value`,});
               { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":spec.escape,"onchange":(e) =>
                  database.setVar(name, { escape: (e.currentTarget as HTMLInputElement).checked }),});}
              
             }
          }
         }
      }
     }
  }

  if(isMongoshScript){
     { svelteHTML.createElement("div", {    "class":`qe-script`,...__sveltets_2_empty({"data-testid":`mongosh-script-bar`}),});mongoshInfo?.available === false || mongoshProbeError !== null;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
       { svelteHTML.createElement("span", {});
                   
         { svelteHTML.createElement("code", { "class":`mono`,});  }        
                
       }
      if(mongoshProbeError !== null){
         { svelteHTML.createElement("span", { "class":`qe-script-state warn`,});    mongoshProbeError; }
      } else if (mongoshInfo === null){
         { svelteHTML.createElement("span", { "class":`qe-script-state dim`,});   }
      } else if (mongoshInfo.available){
         { svelteHTML.createElement("span", {   "class":`qe-script-state ok`,"title":`The daemon found the mongosh CLI on its PATH`,});
           mongoshInfo.version ?? 'mongosh found';
         }
      }else{
         { svelteHTML.createElement("span", { "class":`qe-script-state warn`,});
                    { svelteHTML.createElement("code", { "class":`mono`,});   }  
         }
      }
      if(mongoshProbeError !== null || (mongoshInfo !== null && !mongoshInfo.available)){
         { svelteHTML.createElement("button", {       "class":`qe-script-retry`,"onclick":() => void probeMongosh(),"title":`Probe the daemon for the mongosh CLI again`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} 
         }
      }
     }
  }

  if(saving){
     { svelteHTML.createElement("div", { "class":`save-bar`,});
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {             "dir":`auto`,"aria-label":`Query name`,"class":`input grow`,"placeholder":savedLinked ? 'Name (blank = keep current)' : 'Query name',"bind:value":saveName,"onkeydown":(e) => {
          if (e.key === 'Enter') confirmSave();
          else if (e.key === 'Escape') saving = false;
        },});/*Ωignore_startΩ*/() => saveName = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
       { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":confirmSave,"disabled":savePending || (!savedLinked && !saveName.trim()),});
        savePending ? 'Saving…' : savedLinked ? 'Update' : 'Save';
       }
      if(savedLinked){
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":confirmSaveAsNew,"disabled":savePending || !saveName.trim(),"title":`Create a new saved query instead of updating`,});
            
         }
      }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (saving = false),});  }
     }
  }

  if(viewport.isPhone){
     { svelteHTML.createElement("button", {     "class":`qe-acc-head`,"onclick":() => (editorOpen = !editorOpen),"aria-expanded":editorOpen,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":editorOpen ? 'chevronDown' : 'chevronRight',"size":14,}});}
       { svelteHTML.createElement("span", { "class":`qe-acc-title`,});  }
     }
  }
   { const $$_div1 = svelteHTML.createElement("div", {     "class":`qe-edit`,"style":`height: ${editorH}px`,});viewport.isPhone && !editorOpen;editHostEl = $$_div1;
     { const $$_rotidEedoC2C = __sveltets_2_ensureComponent(CodeEditor); const $$_rotidEedoC2 = new $$_rotidEedoC2C({ target: __sveltets_2_any(), props: {                                     "path":editorPath,"content":tab.statement,"root":ws.current?.root_path ?? '',"language":lang,"readOnly":false,"minimal":true,"findOwner":true,"wrap":wrapLines,"keepStates":true,"historyKey":historyKeyFor,"oncopy":(text) => {
        if (!tab.mask) clipHistory.record(text, 'Editor');
      },sqlDialect,"placeholder":lang === 'redis' ? 'Write a command — ⌘↵ to run' : 'Write a query — ⌘↵ to run',"completionSource":database.selectedConnId ? completionSource : null,"autoTriggerGate":(s, pos) => !inInertNode(s, pos),"onchange":(v) => database.setStatement(v),"onsubmit":run,"onselect":(s) => (editorSel = s),}});codeEditor = $$_rotidEedoC2;}
   }

  
   {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: editorH, min: 100, max: maxEditorH(), orientation: 'horizontal', onChange: setEditorH, onReset: toggleExpand, text: (v) => `Editor ${Math.round(v)} pixels tall` })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                  "class":`qe-splitter`,"role":`separator`,"aria-orientation":`horizontal`,"aria-label":`Resize editor and results`,"title":`Drag or use ↑/↓ to resize · double-click or Enter to expand`,"onpointerdown":startResize,"ondblclick":toggleExpand,});resizing; { svelteHTML.createElement("span", { "class":`qe-grip`,}); } }}

  if(viewport.isPhone){
     { svelteHTML.createElement("button", {     "class":`qe-acc-head`,"onclick":() => (resultsOpen = !resultsOpen),"aria-expanded":resultsOpen,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":resultsOpen ? 'chevronDown' : 'chevronRight',"size":14,}});}
       { svelteHTML.createElement("span", { "class":`qe-acc-title`,});  }
      if(hasResult && tab.result){ { svelteHTML.createElement("span", { "class":`qe-acc-count`,});plural(tab.result.stats.row_count, 'row'); }}
      if(tab.error){ { svelteHTML.createElement("span", { "class":`qe-acc-count err`,});  }}
     }
  }
   { svelteHTML.createElement("div", {  "class":`qe-results`,});viewport.isPhone && !resultsOpen;
    if(database.planOpen && database.queryPlan){
       { svelteHTML.createElement("div", {  "class":`qe-plan`,});planOnly;
         { const $$_weiVnalP3C = __sveltets_2_ensureComponent(PlanView); new $$_weiVnalP3C({ target: __sveltets_2_any(), props: {    "plan":database.queryPlan,"onclose":() => database.closePlan(),}});}
       }
    }
    
    if(!planOnly){
       { const $$_dirGstluseR2C = __sveltets_2_ensureComponent(ResultsGrid); new $$_dirGstluseR2C({ target: __sveltets_2_any(), props: {                         "result":tab.result,"error":tab.error,"statement":(tab.error ? tab.err_statement : tab.ran_statement) ?? undefined,"connectionId":database.selectedConnId,"ranNode":tab.ran_node,"running":tab.running,"startedAt":tab.pending?.startedAt ?? null,"tabKey":tab.uid,"offset":tab.offset,viewMode,viewReason,"tabPick":tab.viewMode ?? null,"onviewmode":(m) => database.setViewMode(m),}});}
    }
   }
 }

if(multiRun){
   { const $$_golaiDnuRitluM0C = __sveltets_2_ensureComponent(MultiRunDialog); new $$_golaiDnuRitluM0C({ target: __sveltets_2_any(), props: {    "statement":multiRun.statement,"onclose":() => (multiRun = null),}});}
}

if(varsPrompt){
   { const $$_tpmorPsraV0C = __sveltets_2_ensureComponent(VarsPrompt); new $$_tpmorPsraV0C({ target: __sveltets_2_any(), props: {         "names":varsPrompt.names,"vars":tab.vars,"onsubmit":(vals) => {
      for (const [n, spec] of Object.entries(vals)) database.setVar(n, spec);
      const base = varsPrompt!.base;
      varsPrompt = null;
      execBase(base);
    },"oncancel":() => (varsPrompt = null),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const QueryEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type QueryEditor__SvelteComponent_ = ReturnType<typeof QueryEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default QueryEditor__SvelteComponent_;