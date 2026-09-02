<script lang="ts">
  // SQL / Redis / Mongo editor. Wraps the shared CodeEditor with a server-backed
  // completion source (debounced /db/completion). Cmd/Ctrl+Enter runs; toolbar
  // has Run / Save / Explain-with-agent. Results render in the ResultsGrid below.
  import { tick } from 'svelte';
  import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import ResultsGrid from './ResultsGrid.svelte';
  import PlanView from './PlanView.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { database, ROW_LIMIT_ALL, type QueryTab } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { DbCompletionKind } from '../../lib/api/types';
  import {
    statementAtCursor,
    countStatements,
    extractVars,
    substituteVars,
    renderVar,
    defaultVarSpec,
    looksLikeMongoshScript,
    type SplitMode,
    type VarSpec,
  } from './sql-util';
  import { api } from '../../lib/api/client';
  import type { MongoshInfo } from '../../lib/api/types';

  const tab = $derived(database.tab);

  // Default row-cap options (applied when a statement has no explicit LIMIT).
  const ROW_LIMIT_OPTS: { label: string; value: number }[] = [
    { label: '100', value: 100 },
    { label: '500', value: 500 },
    { label: '1,000', value: 1000 },
    { label: '5,000', value: 5000 },
    { label: '10,000', value: 10000 },
    { label: '50,000', value: 50000 },
    { label: 'All', value: ROW_LIMIT_ALL },
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
  // Re-key the editor on tab id + engine so it rebuilds cleanly per query tab.
  const editorPath = $derived(`query-${tab.id}.${lang}`);
  // Statement separator: redis is one command per line; others use `;`.
  const splitMode = $derived<SplitMode>(database.queryLanguage === 'redis' ? 'line' : 'sql');
  // Live selection + cursor (from CodeEditor) → run only the selected/current
  // statement instead of the whole buffer.
  let editorSel = $state<{ text: string; cursor: number }>({ text: '', cursor: 0 });
  // Variables the current tab's statement references (:name / {name}).
  const queryVars = $derived(extractVars(tab.statement, splitMode));
  let varsBarEl = $state<HTMLElement | null>(null);

  // Mongosh SCRIPT notice: when the buffer is real JavaScript (consts,
  // functions, control flow — mirrors the daemon's detection), Run executes it
  // through the actual `mongosh` CLI. Probe the daemon for the binary the
  // FIRST time a script is detected so a missing mongosh becomes an inline
  // install hint here, before the run — never a surprise run-time error.
  const isMongoshScript = $derived(
    database.queryLanguage === 'mongo' && looksLikeMongoshScript(tab.statement),
  );
  let mongoshInfo = $state<MongoshInfo | null>(null);
  let mongoshProbed = false;
  async function probeMongosh(): Promise<void> {
    mongoshInfo = null; // back to "checking…" while the re-probe runs
    try {
      mongoshInfo = await api.get<MongoshInfo>('/db/mongosh');
    } catch {
      mongoshInfo = { available: false };
    }
  }
  $effect(() => {
    if (!isMongoshScript || mongoshProbed) return;
    mongoshProbed = true;
    void probeMongosh();
  });

  // Reset the tracked selection/cursor when switching query tabs, so a stale
  // selection from another tab can never run against the newly-active one.
  $effect(() => {
    void tab.id;
    editorSel = { text: '', cursor: 0 };
  });

  // ── Tab labels ────────────────────────────────────────────────────────────
  // Derive a short, human label from a tab's SQL: prefer an explicit user name,
  // else the table after FROM/INTO/UPDATE, else a leading keyword snippet,
  // falling back to "Query N".
  function tabLabel(t: QueryTab, index: number): string {
    if (t.name && t.name !== 'Query') return t.name;
    const sql = t.statement.trim();
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
  }
  const MAX_CLOSED = 10;
  let closedTabs = $state<ClosedTabPayload[]>([]);
  function closeTabAt(i: number): void {
    const t = database.tabs[i];
    // Only a non-empty buffer is worth remembering.
    if (t && t.statement.trim()) {
      closedTabs = [
        ...closedTabs.slice(-(MAX_CLOSED - 1)),
        {
          name: t.name,
          statement: t.statement,
          vars: { ...t.vars },
          timeout_ms: t.timeout_ms ?? null,
          mask: !!t.mask,
          savedQueryId: t.savedQueryId,
        },
      ];
    }
    database.closeTab(i);
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
    database.switchTab(database.activeTab); // same-index no-op → persists tabs
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

  // Server-driven completion source. Debounced via a shared in-flight promise so
  // fast typing collapses to the latest prefix. Failures degrade to no results.
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  function completionSource(ctx: CompletionContext): Promise<CompletionResult | null> {
    const before = ctx.matchBefore(TOKEN_RE);
    const word = before?.text ?? '';
    // Only auto-open when there's a token or the user explicitly triggered.
    if (!ctx.explicit && word.length === 0) return Promise.resolve(null);

    const prefix = ctx.state.sliceDoc(0, ctx.pos);
    // Text after the cursor lets the server resolve the FROM table list even
    // when the cursor sits in the SELECT list before it.
    const suffix = ctx.state.sliceDoc(ctx.pos);

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

    return new Promise((resolve) => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(async () => {
        const items = await database.complete(prefix, suffix);
        if (items.length === 0) {
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
        resolve({ from, options, validFor: TOKEN_RE });
      }, 120);
    });
  }

  // Substitute query-level variables into `base` and execute it (transient — the
  // tab's text is never overwritten by the rendered SQL). Shared by Run
  // (selection / statement-at-cursor) and Run all (the whole buffer as a batch).
  function execBase(base: string): void {
    if (!base.trim()) {
      void database.runQuery();
      return;
    }
    // Query-level variables: every :name / {name} in the chosen statement needs
    // a value before it can run.
    const names = extractVars(base, splitMode);
    const missing = names.filter((n) => !(tab.vars[n]?.value ?? '').trim());
    if (missing.length > 0) {
      toasts.error(
        'Missing variable value',
        `Set a value for ${missing.map((n) => ':' + n).join(', ')}`,
      );
      void tick().then(() => {
        const inputs = varsBarEl ? Array.from(varsBarEl.querySelectorAll('input')) : [];
        (inputs.find((i) => !i.value.trim()) ?? inputs[0])?.focus();
      });
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

  function run(): void {
    // While a query is running, ⌘↵ (and the editor's submit) must NOT silently
    // abort-and-restart — stopping is an explicit act (the Stop button / Esc).
    if (tab.running) return;
    // A mongosh SCRIPT is INDIVISIBLE. `statementAtCursor` cuts on top-level
    // `;`, which for real JavaScript slices the file into fragments that share
    // no scope — running one alone either does nothing visible (the leading
    // `const …;`) or dies on an undefined variable from an earlier fragment.
    // With no explicit selection, Run therefore executes the whole buffer,
    // exactly what the script notice above the editor promises.
    if (isMongoshScript && !editorSel.text.trim()) {
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
  const stmtCount = $derived(countStatements(tab.statement, splitMode));
  function runAll(): void {
    execBase(tab.statement);
  }

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
    return Math.max(180, Math.min(max, Math.round(vh * 0.42)));
  }

  // Adopt this tab's remembered height whenever the active tab changes, and
  // shrink the editor off its "nothing to show yet" size the first time the tab
  // produces a result.
  let lastTabId = -1;
  let lastHadResult = false;
  $effect(() => {
    const id = tab?.id ?? -1;
    const hasResult = !!tab?.result;
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
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
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
  /** True when this tab was opened from a saved query that still exists — then
   *  the primary Save UPDATES it in place ("Save as new" forks a fresh one). */
  const savedLinked = $derived(
    !!tab?.savedQueryId && database.savedQueries.some((q) => q.id === tab.savedQueryId),
  );
  async function openSave(): Promise<void> {
    saveName = tab.name && tab.name !== 'Query' ? tab.name : '';
    saving = true;
    await tick();
  }
  /** Primary save: update the linked saved query in place, else create a new one
   *  (a name is required only for the create case). */
  async function confirmSave(): Promise<void> {
    if (!savedLinked && !saveName.trim()) return;
    const saved = await database.saveActiveTab(saveName);
    if (saved) {
      saving = false;
      saveName = '';
    }
  }
  /** Always create a fresh saved query from the current statement (a name is
   *  required). Lets the user fork a saved query without overwriting it. */
  async function confirmSaveAsNew(): Promise<void> {
    const name = saveName.trim();
    if (!name) return;
    const saved = await database.saveQuery(name, tab.statement);
    if (saved) {
      saving = false;
      saveName = '';
    }
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
    { keys: 'Esc', label: 'Cancel running query' },
    { keys: '⌥⌘→ / ⌥⌘←', label: 'Next / previous query tab' },
    { keys: '⌥⌘T', label: 'New query tab' },
    { keys: '⌥⌘W', label: 'Close query tab' },
    { keys: '⇧⌥⌘W', label: 'Reopen closed tab' },
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
        database.abortQuery();
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
      if (canEdit && tab.statement.trim()) void openSave();
      return;
    }
    // ⇧⌘F — format (⌘F alone stays the app-global find). A mongosh SCRIPT is
    // real JavaScript — the structural formatter collapses its newlines, which
    // breaks ASI statement boundaries — so Format is gated off for scripts.
    if (cmd && e.shiftKey && !e.altKey && e.code === 'KeyF') {
      e.preventDefault();
      if (database.queryLanguage !== 'redis' && !isMongoshScript && tab.statement.trim() && !tab.running) {
        database.formatStatement();
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
    // ⌥⌘T new query tab / ⌥⌘W close query tab (⌘T/⌘W stay session actions).
    if (e.metaKey && e.altKey && e.code === 'KeyT') {
      e.preventDefault();
      database.newTab();
      return;
    }
    if (e.metaKey && e.altKey && !e.shiftKey && e.code === 'KeyW') {
      e.preventDefault();
      if (database.tabs.length > 1) closeTabAt(database.activeTab);
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
    window.addEventListener('pointerdown', onDown, true);
    return () => window.removeEventListener('pointerdown', onDown, true);
  });
  // Clamp the popover into the viewport (never off the right/left edge); the
  // CSS max-height + overflow cap the vertical side.
  $effect(() => {
    const el = kbdPopEl;
    if (!shortcutsOpen || !el) return;
    el.style.transform = '';
    const r = el.getBoundingClientRect();
    const vw = window.innerWidth;
    let dx = 0;
    if (r.right > vw - 8) dx = vw - 8 - r.right;
    if (r.left + dx < 8) dx = 8 - r.left;
    if (dx !== 0) el.style.transform = `translateX(${dx}px)`;
  });
</script>

<div class="query-editor" bind:this={rootEl}>
  <div class="qe-tabs" role="tablist" aria-label="Query tabs">
    {#each database.tabs as t, i (t.id)}
      <div
        class="qe-tab"
        class:active={i === database.activeTab}
        role="tab"
        tabindex="0"
        aria-selected={i === database.activeTab}
        onclick={() => database.switchTab(i)}
        ondblclick={() => startRename(i, t)}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            database.switchTab(i);
          }
        }}
      >
        {#if renaming === i}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="qe-tab-rename mono"
            bind:value={renameText}
            autofocus
            onclick={(e) => e.stopPropagation()}
            onblur={() => commitRename(t)}
            onkeydown={(e) => {
              e.stopPropagation();
              if (e.key === 'Enter') commitRename(t);
              else if (e.key === 'Escape') renaming = null;
            }}
          />
        {:else}
          <span class="qe-tab-label">{tabLabel(t, i)}</span>
          {#if t.running}<span class="qe-tab-dot running" title="Running"></span>
          {:else if t.error}<span class="qe-tab-dot error" title="Error"></span>{/if}
          {#if database.tabs.length > 1}
            <button
              class="qe-tab-close"
              title="Close tab"
              aria-label="Close tab"
              onclick={(e) => {
                e.stopPropagation();
                closeTabAt(i);
              }}
            >
              <Icon name="x" size={10} />
            </button>
          {/if}
        {/if}
      </div>
    {/each}
    <button class="qe-tab-new" title="New query tab" aria-label="New query tab" onclick={() => database.newTab()}>
      <Icon name="plus" size={12} />
    </button>
  </div>

  <div class="qe-toolbar">
    {#if tab.running}
      <button class="btn small stop" onclick={() => database.abortQuery()} title="Stop the running query">
        <Icon name="x" size={12} />
        Stop
      </button>
    {:else}
      <button
        class="btn small primary"
        onclick={run}
        disabled={!database.selectedConnId}
        title={isMongoshScript
          ? 'Run the WHOLE script through mongosh — a script is indivisible, so Run never sends a single ;-delimited fragment (⌘↵)'
          : 'Run the selection, else the statement under the cursor (⌘↵)'}
      >
        <Icon name="play" size={12} />
        Run
        <span class="kbd">⌘↵</span>
      </button>
      {#if stmtCount > 1 && !isMongoshScript}
        <button
          class="btn small"
          onclick={runAll}
          disabled={!database.selectedConnId}
          title="Run all {stmtCount} statements as one batch — one result set per statement (⇧⌘↵)"
        >
          <Icon name="play" size={12} />
          Run all
        </button>
      {/if}
    {/if}
    {#if canEdit}
      <button
        class="btn small"
        onclick={openSave}
        disabled={!tab.statement.trim()}
        title={savedLinked ? 'Update the saved query (or Save as new)' : 'Save this query'}
      >
        <Icon name="check" size={11} />{savedLinked ? 'Update' : 'Save'}
      </button>
    {/if}
    {#if database.capabilities?.explain !== false}
      <button
        class="btn small ghost"
        class:on={database.planOpen}
        onclick={() => void database.explainPlan()}
        disabled={!tab.statement.trim() || tab.running}
        title="Show the query plan (EXPLAIN) — a normalized tree with cost warnings"
      >
        <Icon name="zap" size={11} /><span class="btn-label">Explain</span>
      </button>
    {/if}
    <button
      class="btn small ghost"
      class:on={database.assistOpen && database.assistMode === 'ask'}
      onclick={() => database.openAssist('ask')}
      disabled={!database.selectedConnId}
      title="Ask an agent anything about this database — opens the DB Assistant beside the editor"
    >
      <Icon name="comment" size={11} /><span class="btn-label">Ask AI</span>
    </button>
    <button
      class="btn small ghost"
      class:on={database.assistOpen && database.assistMode === 'nl'}
      onclick={() => database.openAssist('nl')}
      disabled={!database.selectedConnId}
      title="Describe what you want in plain English — the DB Assistant agent drafts a query you can insert or run"
    >
      <Icon name="comment" size={11} /><span class="btn-label">Ask in English</span>
    </button>
    {#if database.queryLanguage !== 'redis'}
      <button
        class="btn small ghost"
        onclick={() => {
          database.formatStatement();
          editorSel = { text: '', cursor: 0 };
        }}
        disabled={!tab.statement.trim() || tab.running || isMongoshScript}
        title={isMongoshScript
          ? 'Format is disabled for mongosh scripts — reflowing real JavaScript breaks its statement boundaries'
          : 'Format / beautify the SQL'}
      >
        <Icon name="command" size={11} /><span class="btn-label">Format</span>
      </button>
    {/if}
    <div class="qe-kbd" bind:this={kbdWrapEl}>
      <button
        class="btn small ghost"
        class:on={shortcutsOpen}
        onclick={() => (shortcutsOpen = !shortcutsOpen)}
        title="Keyboard shortcuts"
        aria-label="Keyboard shortcuts"
        aria-expanded={shortcutsOpen}
      >⌨</button>
      {#if shortcutsOpen}
        <div class="qe-kbd-pop" role="menu" bind:this={kbdPopEl}>
          <div class="qe-kbd-title">Keyboard shortcuts</div>
          {#each SHORTCUTS as s (s.label)}
            <div class="qe-kbd-row">
              <span class="qe-kbd-label">{s.label}</span>
              <kbd class="qe-kbd-keys">{s.keys}</kbd>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    <span class="grow"></span>
    {#if database.capabilities?.sql && database.databaseNames.length > 0}
      <label class="qe-db" title="Active database — queries run scoped to it, so you don't need a db. prefix">
        <Icon name="db" size={11} />
        <select
          class="input"
          value={database.activeDb ?? ''}
          onchange={(e) => database.setActiveDb((e.currentTarget as HTMLSelectElement).value || null)}
        >
          <option value="">No active DB</option>
          {#each database.databaseNames as db (db)}
            <option value={db}>{db}</option>
          {/each}
        </select>
      </label>
    {:else if database.isRedis && database.keyspaces.length > 0}
      <label class="qe-db" title="Active Redis database — commands (GET, HGETALL, …) run against this DB">
        <Icon name="db" size={11} />
        <select
          class="input"
          value={database.activeDb ?? database.keyspaces[0]?.id ?? ''}
          onchange={(e) => database.setActiveDb((e.currentTarget as HTMLSelectElement).value || null)}
        >
          {#each database.keyspaces as ks (ks.id)}
            <option value={ks.id}>{ks.label}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="qe-limit" title="Default row cap — an explicit LIMIT in your query overrides this">
      <span>Limit</span>
      <select
        class="input"
        value={database.rowLimit}
        onchange={(e) => database.setRowLimit(Number((e.currentTarget as HTMLSelectElement).value))}
      >
        {#each ROW_LIMIT_OPTS as o (o.value)}
          <option value={o.value}>{o.label}</option>
        {/each}
      </select>
    </label>
    <label class="qe-timeout" title="Per-statement timeout (ms) — 0 or blank = no limit; MySQL only">
      <span>Timeout</span>
      <input
        class="input qe-timeout-input"
        type="number"
        min="0"
        step="1000"
        placeholder="ms"
        value={tab.timeout_ms ?? ''}
        oninput={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          database.tab.timeout_ms = v > 0 ? v : null;
        }}
      />
    </label>
    <label
      class="qe-mask"
      class:active={tab.mask}
      title="Mask PII/prod — server redacts sensitive values (emails, tokens, keys) before returning results"
    >
      <input
        type="checkbox"
        class="sr-only"
        checked={tab.mask}
        onchange={(e) => { database.tab.mask = (e.currentTarget as HTMLInputElement).checked; }}
      />
      <Icon name="lock" size={11} />
      {#if tab.mask}<span class="qe-masked-badge">Masked</span>{:else}<span>Mask</span>{/if}
    </label>
    <span class="qe-lang mono">{database.queryLanguage}</span>
  </div>

  {#if queryVars.length > 0}
    <div class="qe-vars" bind:this={varsBarEl}>
      <Icon name="tag" size={11} />
      <span class="qe-vars-label">Variables</span>
      {#each queryVars as name (name)}
        {@const spec = tab.vars[name] ?? defaultVarSpec()}
        <div class="qe-var">
          <span class="qe-var-name mono">{name}</span>
          <input
            class="input qe-var-input"
            value={spec.value}
            placeholder={spec.type === 'number' ? '123' : 'value'}
            spellcheck="false"
            oninput={(e) =>
              database.setVar(name, { value: (e.currentTarget as HTMLInputElement).value })}
            onkeydown={(e) => {
              if (e.key === 'Enter') run();
            }}
          />
          <select
            class="input qe-var-type"
            value={spec.type}
            title="How to substitute this variable: string (quoted), number (raw), or raw (verbatim)"
            onchange={(e) =>
              database.setVar(name, {
                type: (e.currentTarget as HTMLSelectElement).value as VarSpec['type'],
              })}
          >
            <option value="string">string</option>
            <option value="number">number</option>
            <option value="raw">raw</option>
          </select>
          {#if spec.type === 'string'}
            <label class="qe-var-esc" title="Escape quotes inside the value">
              <input
                type="checkbox"
                checked={spec.escape}
                onchange={(e) =>
                  database.setVar(name, { escape: (e.currentTarget as HTMLInputElement).checked })}
              />
              esc
            </label>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  {#if isMongoshScript}
    <div class="qe-script" class:missing={mongoshInfo?.available === false} data-testid="mongosh-script-bar">
      <Icon name="zap" size={11} />
      <span>
        mongosh script detected — Run executes the WHOLE file through the real
        <code class="mono">mongosh</code> CLI against this connection (counts as a write).
        Select a region first to run only that part.
      </span>
      {#if mongoshInfo === null}
        <span class="qe-script-state dim">checking for mongosh…</span>
      {:else if mongoshInfo.available}
        <span class="qe-script-state ok" title="The daemon found the mongosh CLI on its PATH">
          ✓ {mongoshInfo.version ?? 'mongosh found'}
        </span>
      {:else}
        <span class="qe-script-state warn">
          mongosh is not installed on the daemon's PATH — <code class="mono">brew install mongosh</code>, then retry.
        </span>
        <button
          class="qe-script-retry"
          onclick={() => void probeMongosh()}
          title="Probe the daemon for the mongosh CLI again"
        >
          <Icon name="refresh" size={10} /> Retry
        </button>
      {/if}
    </div>
  {/if}

  {#if saving}
    <div class="save-bar">
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input grow"
        placeholder={savedLinked ? 'Name (blank = keep current)' : 'Query name'}
        bind:value={saveName}
        autofocus
        onkeydown={(e) => {
          if (e.key === 'Enter') confirmSave();
          else if (e.key === 'Escape') saving = false;
        }}
      />
      <button class="btn small primary" onclick={confirmSave} disabled={!savedLinked && !saveName.trim()}>
        {savedLinked ? 'Update' : 'Save'}
      </button>
      {#if savedLinked}
        <button class="btn small" onclick={confirmSaveAsNew} disabled={!saveName.trim()} title="Create a new saved query instead of updating">
          Save as new
        </button>
      {/if}
      <button class="btn small" onclick={() => (saving = false)}>Cancel</button>
    </div>
  {/if}

  {#if viewport.isPhone}
    <button class="qe-acc-head" onclick={() => (editorOpen = !editorOpen)} aria-expanded={editorOpen}>
      <Icon name={editorOpen ? 'chevronDown' : 'chevronRight'} size={14} />
      <span class="qe-acc-title">Editor</span>
    </button>
  {/if}
  <div class="qe-edit" class:qe-collapsed={viewport.isPhone && !editorOpen} style="height: {editorH}px">
    <CodeEditor
      path={editorPath}
      content={tab.statement}
      root={ws.current?.root_path ?? ''}
      language={lang}
      readOnly={false}
      minimal={true}
      findOwner={true}
      completionSource={database.selectedConnId ? completionSource : null}
      onchange={(v) => database.setStatement(v)}
      onsubmit={run}
      onselect={(s) => (editorSel = s)}
    />
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="qe-splitter"
    class:resizing
    role="separator"
    aria-orientation="horizontal"
    aria-label="Drag to resize editor and results"
    title="Drag to resize · double-click to expand"
    onpointerdown={startResize}
    ondblclick={toggleExpand}
  ><span class="qe-grip"></span></div>

  {#if viewport.isPhone}
    <button class="qe-acc-head" onclick={() => (resultsOpen = !resultsOpen)} aria-expanded={resultsOpen}>
      <Icon name={resultsOpen ? 'chevronDown' : 'chevronRight'} size={14} />
      <span class="qe-acc-title">Results</span>
      {#if hasResult && tab.result}<span class="qe-acc-count">{tab.result.stats.row_count} rows</span>{/if}
      {#if tab.error}<span class="qe-acc-count err">error</span>{/if}
    </button>
  {/if}
  <div class="qe-results" class:qe-collapsed={viewport.isPhone && !resultsOpen}>
    {#if database.planOpen && database.queryPlan}
      <div class="qe-plan">
        <PlanView plan={database.queryPlan} onclose={() => database.closePlan()} />
      </div>
    {/if}
    <!-- `statement` is the one that PRODUCED these rows, not the live buffer: the
         grid describes what is on screen (see QueryTab.ran_statement). -->
    <ResultsGrid
      result={tab.result}
      error={tab.error}
      statement={tab.ran_statement ?? tab.statement}
      connectionId={database.selectedConnId}
      running={tab.running}
      offset={tab.offset}
    />
  </div>
</div>

<style>
  .query-editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    /* Shrink with the (narrow tablet) main pane rather than forcing intrinsic
       width, so the wrapping toolbar/tab strips stay inside the viewport. */
    min-width: 0;
  }
  .qe-tabs {
    display: flex;
    align-items: stretch;
    gap: 3px;
    margin-bottom: 8px;
    overflow-x: auto;
    scrollbar-width: thin;
    padding-bottom: 1px;
    border-bottom: 1px solid var(--border);
  }
  .qe-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    max-width: 220px;
    padding: 0 4px 0 11px;
    border: 1px solid transparent;
    border-bottom: none;
    border-top-left-radius: var(--radius-s);
    border-top-right-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
    flex: 0 0 auto;
    transition: background 0.12s, color 0.12s;
  }
  .qe-tab:hover {
    background: color-mix(in srgb, var(--text-dim) 7%, transparent);
    color: var(--text);
  }
  .qe-tab.active {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--text);
    font-weight: 600;
    /* sit on top of the strip's bottom border */
    margin-bottom: -1px;
    padding-bottom: 1px;
  }
  .qe-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .qe-tab-rename {
    height: 18px;
    width: 130px;
    padding: 0 4px;
    font-size: 11.5px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
  }
  .qe-tab-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
  .qe-tab-dot.running {
    background: var(--accent);
    animation: qe-pulse 1s ease-in-out infinite;
  }
  .qe-tab-dot.error {
    background: var(--status-exited);
  }
  @keyframes qe-pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  .qe-tab-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 17px;
    height: 17px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex: 0 0 auto;
    opacity: 0.6;
  }
  .qe-tab:hover .qe-tab-close,
  .qe-tab.active .qe-tab-close {
    opacity: 1;
  }
  .qe-tab-close:hover {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
  .qe-tab-new {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex: 0 0 auto;
  }
  .qe-tab-new:hover {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--accent);
  }
  /* Secondary actions shed their labels when the pane is tight, so the row never
     wraps into a second bar; `title` still names each one. */
  @media (max-width: 1500px) {
    .btn-label {
      display: none;
    }
  }
  .qe-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 0 6px;
  }
  /* Query-level variables bar — shown only when the statement references
     :name / {name}. One labelled input per variable, values remembered per tab. */
  .qe-vars {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 0 0 8px;
    color: var(--text-dim);
  }
  /* Mongosh-script notice: what Run will do + whether the CLI exists. */
  .qe-script {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 8px;
    padding: 5px 10px;
    font-size: 11.5px;
    color: var(--text);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border-radius: var(--radius-s);
  }
  .qe-script.missing {
    border-color: color-mix(in srgb, var(--status-warn) 50%, transparent);
    background: color-mix(in srgb, var(--status-warn) 10%, transparent);
  }
  .qe-script-state {
    font-size: 11px;
  }
  .qe-script-state.ok {
    color: var(--status-working);
    font-weight: 600;
  }
  .qe-script-state.warn {
    color: var(--status-warn);
    font-weight: 600;
  }
  .qe-script-state.dim {
    color: var(--text-dim);
  }
  .qe-script-retry {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11px;
    cursor: pointer;
  }
  .qe-script-retry:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .qe-vars-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .qe-var {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 2px 6px;
  }
  .qe-var-name {
    font-size: 11.5px;
    color: var(--accent);
  }
  .qe-var-name::before {
    content: ':';
    opacity: 0.6;
  }
  .qe-var-input {
    height: 22px;
    width: 120px;
    font-size: 12px;
  }
  .qe-var-type {
    height: 22px;
    font-size: 11px;
    padding: 0 2px;
  }
  .qe-var-esc {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.03em;
    cursor: pointer;
  }
  /* Toggle highlight for a toolbar button (Ask AI / Ask in English) when its DB
     Assistant panel is open in that mode. */
  .btn.small.ghost.on {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  /* ⌨ shortcuts popover */
  .qe-kbd {
    position: relative;
    display: inline-flex;
  }
  .qe-kbd-pop {
    position: absolute;
    top: calc(100% + 6px);
    inset-inline-start: 0;
    z-index: 30;
    min-width: 220px;
    /* Never off-screen: cap to the viewport and scroll inside (the JS clamp
       handles the horizontal side). */
    max-width: calc(100vw - 16px);
    max-height: min(60vh, calc(100vh - 120px));
    overflow-y: auto;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .qe-kbd-title {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 2px 4px 4px;
  }
  .qe-kbd-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 2px 4px;
    font-size: 12px;
    color: var(--text);
  }
  .qe-kbd-keys {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 1px 6px;
    white-space: nowrap;
  }
  .btn.stop {
    border-color: color-mix(in srgb, var(--status-exited) 55%, transparent);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
    font-weight: 600;
  }
  .btn.stop:hover {
    background: color-mix(in srgb, var(--status-exited) 26%, transparent);
  }
  .kbd {
    font-size: 9.5px;
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
  }
  .qe-lang {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 0 6px;
    height: 18px;
    line-height: 18px;
    border-radius: 999px;
    background: var(--surface-2);
  }
  .qe-limit,
  .qe-db,
  .qe-timeout {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .qe-limit select,
  .qe-db select {
    height: 24px;
    padding: 0 4px;
    font-size: 11px;
    width: auto;
    max-width: 160px;
  }
  .qe-timeout-input {
    height: 24px;
    padding: 0 4px;
    font-size: 11px;
    width: 72px;
  }
  /* Mask PII/prod toggle — styled like a small button, highlights when active. */
  .qe-mask {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
    height: 24px;
    padding: 0 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
    user-select: none;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .qe-mask:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .qe-mask.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    border-color: var(--accent);
    color: var(--accent);
  }
  .qe-masked-badge {
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .save-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 0 8px;
  }
  .qe-edit {
    flex: 0 0 auto;
    min-height: 100px;
    /* Edge-to-edge: only a hairline separating it from the results below. The
       inset rounded box cost ~20px of writing space and made the editor read as
       a floating widget rather than the surface it is. */
    border: none;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    overflow: hidden;
  }
  /* Draggable divider between editor and results. */
  .qe-splitter {
    flex: 0 0 auto;
    height: 7px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: row-resize;
    touch-action: none;
  }
  .qe-grip {
    width: 40px;
    height: 3px;
    border-radius: 2px;
    background: var(--border);
    transition: background 120ms ease-out;
  }
  .qe-splitter:hover .qe-grip,
  .qe-splitter.resizing .qe-grip {
    background: var(--accent);
  }
  .qe-results {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  /* Query-plan panel sits above the grid, bounded so the results stay visible. */
  .qe-plan {
    flex: 0 1 auto;
    min-height: 0;
    max-height: 45%;
    display: flex;
    margin-bottom: 8px;
  }
  .grow {
    flex: 1;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     On desktop this is a fixed-height flex column (editor + flex:1 results)
     living inside the page's clipped viewport. On a phone that height chain
     is broken upstream (DatabasePage lets the page scroll) so here we make the
     editor a modest fixed height, let the dense toolbar WRAP instead of
     overflowing off-screen, and give the results their own bounded,
     internally-scrolling block so a query's rows are always reachable. */
  /* Phone accordion headers for the Editor / Results blocks. */
  .qe-acc-head {
    display: none;
  }
  /* Tablet (641–1200px): the side-by-side DB layout narrows the editor column,
     so wrap the dense toolbar (Run/Save/Explain/Ask-AI + Limit/Timeout/Mask)
     onto multiple rows instead of letting it overflow and get clipped — the same
     wrap the phone layout uses, but WITHOUT forcing the compact editor height.
     Upper bound covers iPad landscape (1194px); desktop (≥1280) keeps one row. */
  @media (min-width: 641px) and (max-width: 1200px) {
    .qe-toolbar {
      flex-wrap: wrap;
      gap: 6px;
      row-gap: 6px;
    }
    .qe-toolbar .grow {
      flex-basis: 100%;
      height: 0;
      flex: 0 0 100%;
    }
  }

  @media (max-width: 640px) {
    .query-editor {
      height: auto;
      min-height: 0;
    }
    /* Editor: ignore the persisted desktop drag-height — keep it compact so the
       results sit just below it (the inline style sets height, so override it). */
    .qe-edit {
      height: 200px !important;
    }
    /* Dense toolbar → wrap onto multiple rows so nothing runs off the edge. */
    .qe-toolbar {
      flex-wrap: wrap;
      gap: 6px;
      row-gap: 6px;
    }
    /* The flexible spacer would force the controls onto a wider line — collapse
       it on mobile so the controls pack tightly and wrap naturally. */
    .qe-toolbar .grow {
      flex-basis: 100%;
      height: 0;
      flex: 0 0 100%;
    }
    /* Bigger tap targets / readable controls. */
    .qe-limit select,
    .qe-db select,
    .qe-timeout-input,
    .qe-mask {
      height: 32px;
      font-size: 12.5px;
    }
    .qe-limit,
    .qe-db,
    .qe-timeout {
      font-size: 12.5px;
    }
    .qe-tab {
      height: 32px;
      font-size: 13px;
      max-width: 60vw;
    }
    /* Collapsible Editor / Results accordion headers. */
    .qe-acc-head {
      display: flex;
      align-items: center;
      gap: 8px;
      width: 100%;
      min-height: 44px;
      padding: 8px 4px;
      border: none;
      border-top: 1px solid var(--border);
      background: transparent;
      color: var(--text-dim);
      cursor: pointer;
      text-align: start;
    }
    .qe-acc-title {
      font-size: 12.5px;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.04em;
    }
    .qe-acc-count {
      font-size: 11.5px;
      color: var(--text-dim);
      background: var(--surface-2);
      border-radius: 999px;
      padding: 1px 8px;
      font-variant-numeric: tabular-nums;
    }
    .qe-acc-count.err {
      color: var(--status-exited);
      background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    }
    /* A collapsed block is removed from flow. */
    .qe-collapsed {
      display: none !important;
    }
    /* The drag splitter has no role on touch (we resize via the editor's fixed
       height + accordion) — hide it. */
    .qe-splitter {
      display: none;
    }
    /* Results: own bounded, internally-scrolling block — always reachable. A
       small result fits naturally; a large one caps at ~70vh and the grid
       scrolls inside it (its child .grid-scroll is overflow:auto) so the page
       doesn't grow unbounded. */
    .qe-results {
      flex: 0 0 auto;
      min-height: 340px;
      max-height: 70vh;
    }
    .qe-tabs {
      scrollbar-width: none;
    }
    .qe-tabs::-webkit-scrollbar {
      display: none;
    }
  }
</style>
