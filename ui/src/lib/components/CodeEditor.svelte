<script lang="ts">
  // CodeMirror 6 editor with LSP hover/diagnostics/completion/definitions.
  // readOnly=true by default (Files viewer is read-only; LSP still works).
  import { onDestroy, untrack } from 'svelte';
  import { EditorView, lineNumbers, keymap, drawSelection, placeholder as cmPlaceholder } from '@codemirror/view';
  import { EditorState, Compartment, Prec, Text, type StateEffect } from '@codemirror/state';
  import { defaultKeymap, history, historyField, historyKeymap, selectAll } from '@codemirror/commands';
  import {
    loadEditorState,
    prepareEditorHistory,
    registerLiveParker,
    saveEditorState,
    PERSIST_MAX_BYTES,
  } from '../editor-history';
  import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
  import {
    autocompletion,
    completionKeymap,
    startCompletion,
    closeBrackets,
    closeBracketsKeymap,
  } from '@codemirror/autocomplete';
  import type { CompletionSource } from '@codemirror/autocomplete';
  import { lintGutter, lintKeymap } from '@codemirror/lint';
  import {
    indentOnInput,
    bracketMatching,
    foldGutter,
    foldKeymap,
    defaultHighlightStyle,
    syntaxHighlighting,
    foldedRanges,
    unfoldEffect,
  } from '@codemirror/language';
  import { oneDark, oneDarkTheme } from '@codemirror/theme-one-dark';
  import type { Extension } from '@codemirror/state';

  // Language packages load on demand (cm-langs.ts); SQL/Redis are static there.
  import { cmLangNow, cmLangPending, loadCmLang } from './cm-langs';
  import type { SqlDialectName } from '../sql-dialects';
  import { createChangeEmitter } from './changeEmitter';

  // LSP — `@marimo-team/codemirror-languageserver` (the all-in-one factory that
  // manages the WS transport) is imported in attachLsp, only once the daemon
  // reports an available server for the doc's language.

  import { api, baseUrl } from '../api/client';
  import type { LspCapabilities } from '../api/types';
  import { ws } from '../stores/workspace.svelte';
  import { ui } from '../stores/ui.svelte';
  import { keyContext } from '../keys';
  import { registerFindProvider } from '../findProviders';
  import { toasts } from '../toast.svelte';

  // Editor theme follows the app scheme: oneDark for dark, a light theme keyed to
  // the app's CSS variables for light (so the editor + its selection are visible).
  const themeCompartment = new Compartment();
  const lightTheme = EditorView.theme(
    {
      '&': { color: 'var(--text)', backgroundColor: 'transparent' },
      '.cm-content': { caretColor: 'var(--text)' },
      '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
      '.cm-selectionBackground': {
        backgroundColor: 'color-mix(in srgb, var(--accent) 30%, transparent)',
      },
      '&.cm-focused .cm-selectionBackground': {
        backgroundColor: 'color-mix(in srgb, var(--accent) 40%, transparent)',
      },
      '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--text-dim) 8%, transparent)' },
      '.cm-gutters': { backgroundColor: 'transparent', color: 'var(--text-dim)', border: 'none' },
      '.cm-activeLineGutter': { backgroundColor: 'transparent' },
    },
    { dark: false },
  );
  function themeExt(scheme: 'light' | 'dark', plain = false): Extension {
    // `plain` (see `highlightLineLimit`): the same theme without syntax colours.
    if (plain) return scheme === 'dark' ? oneDarkTheme : lightTheme;
    // Dark: oneDark already bundles a syntax highlight style. Light: pair the
    // light theme with the default (light-oriented) highlight style so SQL — and
    // every language — is actually COLORED in light mode (previously it wasn't).
    return scheme === 'dark' ? oneDark : [lightTheme, syntaxHighlighting(defaultHighlightStyle)];
  }

  // ── Props ──────────────────────────────────────────────────────────────────

  interface Props {
    path: string;
    content: string;
    root: string;
    language?: string;
    readOnly?: boolean;
    /**
     * Fired with the full document text after edits (only when !readOnly).
     * Synchronous per edit below 256 K chars; at/above that it is coalesced
     * (one emit after 150 ms of quiet, flushed on blur, ⌘/Ctrl-chords, submit,
     * doc switch and unmount) so a multi-MB body doesn't copy itself per key.
     */
    onchange?: (value: string) => void;
    /**
     * Optional custom autocompletion source. When set, it overrides the default
     * completion (and suppresses LSP for the doc) — used by the DB query editor
     * to surface server-driven SQL/Redis/Mongo completions. Reapplied live via a
     * Compartment so callers can toggle it without remounting the editor.
     */
    completionSource?: CompletionSource | null;
    /**
     * With `completionSource`: may the popup open proactively (after `.` or a
     * clause keyword + space) at `pos`? `startCompletion` is an EXPLICIT query,
     * which the source can't tell from Ctrl-Space, so a gate the source only
     * applies to typing (e.g. "not inside a string") must also be passed here.
     */
    autoTriggerGate?: ((state: EditorState, pos: number) => boolean) | null;
    /** Hide the gutters (line numbers + fold) for a leaner single-statement editor. */
    minimal?: boolean;
    /** Run handler bound to Cmd/Ctrl+Enter (e.g. execute the query). */
    onsubmit?: () => void;
    /**
     * Fired on every selection / cursor change with the selected text (empty
     * string when there's no selection) and the cursor offset. Lets the DB query
     * editor run only the selected — or current — statement.
     */
    onselect?: (s: { text: string; cursor: number }) => void;
    /**
     * Optional 1-based line to scroll to and select on mount (and whenever this
     * value changes for the same doc) — used to jump to a `file:line` reference
     * clicked in the terminal. `gotoCol` (1-based) refines the cursor column.
     */
    gotoLine?: number | null;
    gotoCol?: number | null;
    /**
     * This editor OWNS Cmd/Ctrl+F while focused (default): it registers a global
     * find opener (like the terminal) so the keymap opens CodeMirror's in-editor
     * search/replace panel here instead of the page-wide find-in-page overlay.
     * `false` for an editor whose host already routes ⌘F to it (the DB document
     * modals' `claimFind`) — two claimants would drop the host's on blur.
     * Unfocused, the page-wide overlay still searches the WHOLE document either
     * way (the find provider below).
     */
    findOwner?: boolean;
    /** Hint shown while the document is EMPTY (e.g. "Write a query — ⌘↵ to
     *  run"). Reapplied live; '' / omitted shows nothing. */
    placeholder?: string;
    /**
     * Soft-wrap long lines (`EditorView.lineWrapping`). Reapplied live (no
     * remount). Off by default; the DB query editor turns it on — unwrapped
     * long, highlighted lines (a pasted INSERT dump) make every keystroke
     * re-lay out every rendered line (20–41 ms/key in WebKit at 150 KB vs ~3 ms
     * wrapped), because the edited line changes the content's intrinsic width.
     */
    wrap?: boolean;
    /**
     * Keep each `path`'s EditorState when `path` changes and restore it when
     * that path comes back, instead of rebuilding the view: switching between
     * documents (DB query tabs) then re-parses nothing and keeps each doc's
     * undo history, selection and scroll. Bounded (LRU); opt-in.
     */
    keepStates?: boolean;
    /**
     * Opt-in: keep each doc's state + undo history in the shared store
     * (lib/editor-history.ts) under the returned key, so ⌘Z survives this
     * editor being destroyed and rebuilt (a view switch, leaving the page) —
     * and, with `persist`, a reload (IndexedDB). Return null for a doc whose
     * history must not be kept; `persist: false` keeps it in memory only (a
     * masked DB tab: history holds every pasted string).
     */
    historyKey?: ((path: string) => { key: string; persist: boolean } | null) | null;
    /**
     * Called with the copied/cut text when the user copies or cuts inside the
     * editor (the clipboard ring, lib/stores/clipHistory.svelte.ts). Absent =
     * not recorded.
     */
    oncopy?: ((text: string) => void) | null;
    /**
     * Attach a language server for this doc. OPT-IN (default false): only an
     * editor showing a REAL file under `root` (the Files viewer) passes it.
     * Scratch / virtual-path editors (API body/scripts, canvases, vault notes,
     * skill/context files, Athena SQL, conflict hunks, response viewer) must
     * not: a server runs with `cwd` = `root` (indexing the whole workspace for
     * a 3-line `pre.js`) and a partial/fake doc only yields noise diagnostics.
     * The daemon shares one server per `(lang, root)` across editors.
     */
    lsp?: boolean;
    /**
     * SQL dialect for `.sql` docs (default `standard`). Drives tokenizing, so it
     * decides what a string or comment IS: StandardSQL has no `\'` escapes and
     * no `#` comments, so MySQL data like `'O\'Brien'` flips every later
     * string/code boundary. Reapplied live (a connection switch) without a
     * remount. The DB query editor passes the connection's engine.
     */
    sqlDialect?: SqlDialectName;
    /**
     * Drop syntax COLOURING while any line is longer than this many chars
     * (0 = never; opt-in). The language itself stays — brackets, indentation,
     * folding, completion behave the same. A 200 KB minified JSON body is one
     * soft-wrapped line of ~26k highlight spans, and every typed character
     * re-laid out all of them (11–21 ms/key vs ~4 ms uncoloured). VS Code
     * skips tokenizing lines past 20k chars for the same reason. Re-checked as
     * the doc changes (Format brings the colours back).
     */
    highlightLineLimit?: number;
  }

  let {
    path,
    content,
    root,
    language,
    readOnly = true,
    onchange,
    completionSource = null,
    autoTriggerGate = null,
    minimal = false,
    onsubmit,
    onselect,
    gotoLine = null,
    gotoCol = null,
    findOwner = true,
    placeholder = '',
    wrap = false,
    keepStates = false,
    historyKey = null,
    oncopy = null,
    lsp = false,
    sqlDialect = 'standard',
    highlightLineLimit = 0,
  }: Props = $props();

  // ── Container ─────────────────────────────────────────────────────────────

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let lspCompartment = new Compartment();
  // Holds either the default autocompletion() or one overridden with the
  // caller's completionSource (DB query editor). Reconfigured reactively.
  let completionCompartment = new Compartment();
  let placeholderCompartment = new Compartment();
  /** The hint the live view currently carries (plain field — not reactive). */
  let appliedPlaceholder = '';
  let wrapCompartment = new Compartment();
  /** Whether the live view currently soft-wraps (plain field — not reactive). */
  let appliedWrap = false;
  function wrapExt(on: boolean): Extension {
    return on ? EditorView.lineWrapping : [];
  }
  // Placeholder text uses the app's dim token in BOTH schemes (oneDark ships none).
  const placeholderTheme = EditorView.theme({
    '.cm-placeholder': { color: 'var(--text-dim)', fontStyle: 'normal' },
  });
  function placeholderExt(text: string): Extension {
    return text ? cmPlaceholder(text) : [];
  }

  // ── Long-line plain mode (highlightLineLimit) ───────────────────────────────
  /** Whether the live view is currently uncoloured (plain field — not reactive). */
  let appliedPlain = false;
  let plainRecheck: ReturnType<typeof setTimeout> | null = null;

  /** Any line of `doc` longer than `limit`? Walks the line strings the doc
   *  already holds (no slicing) — O(lines). */
  function hasLongLine(doc: Text, limit: number): boolean {
    if (doc.length <= limit) return false;
    let run = 0;
    for (const it = doc.iter(); !it.next().done; ) {
      if (it.lineBreak) run = 0;
      else if ((run += it.value.length) > limit) return true;
    }
    return false;
  }

  function setPlain(target: EditorView, on: boolean): void {
    if (on === appliedPlain) return;
    appliedPlain = on;
    // Called from an update listener: reconfigure once that update is done.
    queueMicrotask(() => {
      if (view !== target) return;
      const scheme = untrack(() => ui.resolvedScheme);
      target.dispatch({ effects: themeCompartment.reconfigure(themeExt(scheme, appliedPlain)) });
    });
  }

  /** Enter plain mode when an edit leaves a long line behind (only the lines
   *  the edit touched are measured); leave it once no long line is left
   *  (a whole-doc scan, debounced — typing inside the long line keeps it). */
  const plainWatch = EditorView.updateListener.of((u) => {
    const limit = highlightLineLimit;
    if (!u.docChanged || limit <= 0) return;
    if (appliedPlain) {
      if (plainRecheck) clearTimeout(plainRecheck);
      const target = u.view;
      plainRecheck = setTimeout(() => {
        plainRecheck = null;
        if (view === target && !hasLongLine(target.state.doc, limit)) setPlain(target, false);
      }, 300);
      return;
    }
    const doc = u.state.doc;
    let long = false;
    u.changes.iterChangedRanges((_fa, _ta, fromB, toB) => {
      for (let pos = fromB; !long; ) {
        const line = doc.lineAt(pos);
        if (line.length > limit) long = true;
        if (line.to >= toB) break;
        pos = line.to + 1;
      }
    });
    if (long) setPlain(u.view, true);
  });

  // ── Language extension map ─────────────────────────────────────────────────

  /** The dialect the live view's language was built with (plain field). */
  let appliedDialect: SqlDialectName = 'standard';
  /** Holds the language extension so a dialect change reaches a live view. */
  let langCompartment = new Compartment();

  /** False while the live doc's language pack is still loading (the view
   *  shows plain text until `ensureLang` reconfigures it). Plain field. */
  let appliedLangReady = true;

  // LSP language IDs (maps file extension → LSP lang id)
  const EXT_TO_LSP_LANG: Record<string, string> = {
    js:   'javascript',
    jsx:  'javascriptreact',
    ts:   'typescript',
    tsx:  'typescriptreact',
    mjs:  'javascript',
    cjs:  'javascript',
    py:   'python',
    go:   'go',
    rs:   'rust',
    json: 'json',
    jsonc:'json',
    html: 'html',
    htm:  'html',
    css:  'css',
    scss: 'scss',
    less: 'less',
    md:   'markdown',
    mdx:  'markdown',
    java: 'java',
  };

  function extOf(p: string): string {
    return p.split('.').pop()?.toLowerCase() ?? '';
  }

  // ── Selection state ───────────────────────────────────────────────────────

  // Offsets only: the selected TEXT is sliced lazily (Send to agent / the
  // caller's `onselect.text`), never on every selection change — a ⌘A over a
  // 150 KB buffer used to copy the whole doc twice per selection update.
  interface Sel {
    from: number;
    to: number;
    startLine: number;
    endLine: number;
  }

  let sel: Sel | null = $state(null);

  /** Mirror `state`'s main selection into `sel` and the caller's `onselect`. */
  function emitSelection(state: EditorState): void {
    const { from, to, head } = state.selection.main;
    if (from === to) {
      sel = null;
    } else {
      const startLine = state.doc.lineAt(from).number;
      const endLine = state.doc.lineAt(to).number;
      sel = { from, to, startLine, endLine };
    }
    if (!onselect) return;
    // Surface selection + cursor so callers can run only the selected/current
    // statement (text is '' when there's no selection). `text` is a getter over
    // this immutable state, sliced once on first read.
    let text: string | undefined = from === to ? '' : undefined;
    onselect({
      get text() {
        return (text ??= state.sliceDoc(from, to));
      },
      cursor: head,
    });
  }

  /** CodeMirror update listener that tracks the current text selection. */
  const selectionListener = EditorView.updateListener.of((update) => {
    if (!update.selectionSet && !update.docChanged) return;
    emitSelection(update.state);
  });

  // Last value we emitted via onchange — lets the rebuild effect ignore the
  // content prop echoing back our own edit (which would needlessly remount and
  // drop the cursor).
  let lastEmitted: string | null = null;

  /** Large docs defer the (whole-doc) emit; small ones emit per edit. */
  const LAZY_CHANGE_MIN = 256 * 1024;
  const changes = createChangeEmitter({
    threshold: LAZY_CHANGE_MIN,
    delayMs: 150,
    read: () => view?.state.doc.toString() ?? null,
    emit: (value) => {
      lastEmitted = value;
      onchange?.(value);
    },
  });

  /** Emits the doc text on edits so editable callers stay in sync. */
  const changeListener = EditorView.updateListener.of((update) => {
    if (!update.docChanged || !onchange) return;
    changes.changed(update.state.doc.length);
  });

  /** Flush a deferred emit before anything that may act on the value: focus
   *  leaving the editor, or a ⌘/Ctrl chord (Send / Run / Save shortcuts are
   *  handled by window listeners after the editor sees the key). */
  const changeFlushHandlers = EditorView.domEventHandlers({
    blur: () => {
      changes.flush();
      return false;
    },
    keydown: (e) => {
      if ((e.metaKey || e.ctrlKey) && changes.pending) changes.flush();
      return false;
    },
  });

  // ── Send-to-agent handler ─────────────────────────────────────────────────

  async function sendToAgent(): Promise<void> {
    if (!sel || !view) return;
    const text = view.state.sliceDoc(sel.from, sel.to);

    const sessionId = ws.activeSessionId;
    if (!sessionId || ws.activeSession?.kind !== 'agent') {
      toasts.error('No agent session', 'Open an agent session first.');
      return;
    }

    const ext = extOf(path);
    const langHint = ext || '';
    const snippet = `Re: ${path}:${sel.startLine}-${sel.endLine}\n\n\`\`\`${langHint}\n${text}\n\`\`\`\n\n`;

    try {
      await api.post(`/sessions/${sessionId}/input`, { text: snippet, submit: false });
      toasts.success('Sent to agent', `${path} lines ${sel.startLine}-${sel.endLine}`);
    } catch {
      toasts.error('Failed to send', 'Could not inject text into the agent session.');
    }
  }

  // ── LSP capability cache (module-level) ────────────────────────────────────

  let capabilitiesCache: LspCapabilities | null = null;
  let capabilitiesFetching: Promise<LspCapabilities | null> | null = null;

  async function getCapabilities(): Promise<LspCapabilities | null> {
    if (capabilitiesCache) return capabilitiesCache;
    if (capabilitiesFetching) return capabilitiesFetching;
    capabilitiesFetching = api.get<LspCapabilities>('/lsp/capabilities').then((c) => {
      capabilitiesCache = c;
      return c;
    }).catch(() => null);
    return capabilitiesFetching;
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function langExtOf(filePath: string, hint?: string): string {
    return extOf(filePath) || (hint ?? '');
  }

  function cmLangFor(filePath: string, hint?: string): Extension | null {
    return cmLangNow(langExtOf(filePath, hint), appliedDialect);
  }

  /** The live doc started without its (not yet loaded) language: load the pack
   *  and apply it — unless the view was rebuilt or switched docs meanwhile
   *  (a parked doc catches up in swapState). */
  function ensureLang(filePath: string): void {
    if (appliedLangReady) return;
    const target = view;
    loadCmLang(langExtOf(filePath, language)).then(
      () => {
        if (!view || view !== target || livePath !== filePath || appliedLangReady) return;
        const langExt = cmLangFor(filePath, language);
        if (!langExt) return;
        appliedLangReady = true;
        view.dispatch({ effects: langCompartment.reconfigure(langExt) });
      },
      () => {
        /* chunk failed to load — the doc stays plain text */
      },
    );
  }

  function lspLangFor(filePath: string): string | null {
    const ext = extOf(filePath);
    return EXT_TO_LSP_LANG[ext] ?? null;
  }

  // Build a WSS/WS URL for the LSP relay.
  // Path: /ws/lsp?lang=<lang>&root=<root>&token=<token>
  function lspWsUrl(lang: string, rootPath: string): string {
    const base = new URL(baseUrl());
    const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
    const token = localStorage.getItem('otto_token') ?? '';
    const params = new URLSearchParams({ lang, root: rootPath, token });
    return `${proto}//${base.host}/ws/lsp?${params.toString()}`;
  }

  // ── Tear-down helpers ──────────────────────────────────────────────────────

  function teardownEditor(): void {
    // A deferred emit can't be delivered once `path` moved on: `onchange` is
    // the NEW doc's binding by now (it'd write this text into the wrong tab).
    // Real switches flush first anyway — the click blurs, ⌘-chords flush — so
    // this only drops an edit under a programmatic switch mid-debounce.
    // Unmount (same path) flushes in onDestroy before calling this.
    changes.cancel();
    if (plainRecheck) clearTimeout(plainRecheck);
    plainRecheck = null;
    try { view?.destroy(); } catch { /* ignore */ }
    view = null;
    // Release the global find opener if this editor still holds it (blur may not
    // fire on unmount), so Cmd+F falls back to the page-wide overlay.
    if (keyContext.openFind === openEditorSearch) keyContext.openFind = null;
  }

  // ── Attach LSP (fire-and-forget, never breaks editor) ─────────────────────

  async function attachLsp(editorView: EditorView, filePath: string, rootPath: string): Promise<void> {
    try {
      const caps = await getCapabilities();
      if (!caps) return;
      const lspLang = lspLangFor(filePath);
      if (!lspLang) return;
      const server = caps.servers.find((s) => s.lang === lspLang && s.available);
      if (!server) return;
      const { languageServer } = await import('@marimo-team/codemirror-languageserver');
      // The view may have been rebuilt / destroyed while the client loaded.
      if (view !== editorView) return;

      const wsUri = lspWsUrl(lspLang, rootPath);
      // `languageServer` from marimo accepts serverUri and creates the WS transport
      const lspExtensions: Extension[] = languageServer({
        serverUri: wsUri as `ws://${string}` | `wss://${string}`,
        rootUri: `file://${rootPath}`,
        workspaceFolders: [{ name: 'workspace', uri: `file://${rootPath}` }],
        documentUri: `file://${filePath}`,
        languageId: lspLang,
      });

      editorView.dispatch({
        effects: lspCompartment.reconfigure(lspExtensions),
      });
    } catch {
      // LSP failures are silent — editor still shows highlighted code
    }
  }

  // ── Build and mount EditorView ─────────────────────────────────────────────

  /** The autocompletion extension for the current completionSource (if any). */
  function completionExt(): Extension {
    return completionSource
      ? [
          autocompletion({
            override: [completionSource],
            activateOnTyping: true,
            // The popup renders at most this many rows (CM's default is 100).
            // Mounting ~100–200 option rows was a 49–160 ms forced-layout frame
            // on every popup open while typing identifiers; the list still
            // filters over every option, only the DOM is capped.
            maxRenderedOptions: 40,
          }),
          autoTriggerExt(),
        ]
      : autocompletion();
  }

  // Keywords after which a SPACE should open completion (so `select * from `
  // immediately offers tables, `where `/`and ` offers columns).
  const SPACE_TRIGGER_RE =
    /(?:^|[\s({,])(?:from|join|where|and|or|on|into|update|set|by|using|select)\s$/i;
  // Languages where a space after `(` / `{` / `,` opens completion (Mongo's
  // `find({ `, `{ a: 1, `). NOT SQL: there `, ` is what you type between every
  // INSERT value, and forcing the popup open there sent a completion request
  // per value while editing data.
  const PUNCT_TRIGGER_LANGS = new Set(['js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs', 'json', 'jsonc']);

  /**
   * Open the completion popup proactively for the DB query editor (only when a
   * `completionSource` is set) right after the user types:
   *  - a `.` — member access: `alias.`, `db.coll.`, or a Mongo embedded `x.`;
   *  - a space following a clause keyword, or an opening `(`/`{`/`,` (Mongo).
   * Deferred a tick to avoid re-entrancy with the change that triggered it.
   */
  function autoTriggerExt(): Extension {
    return EditorView.updateListener.of((u) => {
      if (!u.docChanged || !u.view.hasFocus) return;
      let fire = false;
      for (const tr of u.transactions) {
        if (!tr.isUserEvent('input.type') && !tr.isUserEvent('input.paste')) continue;
        tr.changes.iterChanges((_fa, _ta, _fb, _tb, inserted) => {
          // Only a single typed char can trigger — never stringify a paste.
          if (inserted.length !== 1) return;
          const text = inserted.toString();
          if (text === '.') {
            fire = true;
          } else if (text === ' ') {
            const head = u.state.selection.main.head;
            const back = u.state.sliceDoc(Math.max(0, head - 48), head);
            if (SPACE_TRIGGER_RE.test(back)) fire = true;
            else if (/[({,]\s$/.test(back) && PUNCT_TRIGGER_LANGS.has(extOf(path) || (language ?? ''))) {
              fire = true;
            }
          }
        });
      }
      // `and ` inside `'O\'Brien and …` must not open (and hold open) an
      // explicit query that then asks the daemon on every letter.
      if (fire && autoTriggerGate && !autoTriggerGate(u.state, u.state.selection.main.head)) fire = false;
      if (fire) {
        const view = u.view;
        setTimeout(() => startCompletion(view), 0);
      }
    });
  }

  /** Submit keybinding (Cmd/Ctrl+Enter) — used by the DB query editor. */
  const submitKeymap = keymap.of([
    {
      key: 'Mod-Enter',
      run: () => {
        if (onsubmit) {
          changes.flush();
          onsubmit();
          return true;
        }
        return false;
      },
    },
  ]);

  // Stable opener for the in-editor search panel (a fixed reference so the
  // focus/blur handlers can register/deregister it on `keyContext.openFind`).
  function openEditorSearch(): void {
    if (view) openSearchPanel(view);
  }

  /** Open this editor's search panel (via `bind:this`) — for a caller's own
   *  Find button, or a modal that routes ⌘F here without `findOwner`. */
  export function openSearch(): void {
    openEditorSearch();
  }

  // ⌘F over the WHOLE document (lib/findProviders.ts). CodeMirror mounts only
  // the lines near its viewport, so the page-wide overlay's DOM walk missed
  // the rest (and matched gutter numbers / search-panel labels). Idle cost is
  // one Set entry: the model is only read while a find runs.
  /** The fold hiding line `ln` (1-based), if any — its text isn't mounted. */
  function foldOver(v: EditorView, ln: number): { from: number; to: number } | null {
    const line = v.state.doc.line(ln);
    let hit: { from: number; to: number } | null = null;
    foldedRanges(v.state).between(line.from, line.to, (from, to) => {
      if (from < line.to && to > line.from) {
        hit = { from, to };
        return false;
      }
    });
    return hit;
  }
  $effect(() =>
    registerFindProvider({
      root: () => view?.dom ?? null,
      count: () => view?.state.doc.lines ?? 0,
      text: (i) => (view && i < view.state.doc.lines ? view.state.doc.line(i + 1).text : ''),
      reveal: async (i) => {
        const v = view;
        if (!v || i >= v.state.doc.lines) return;
        // Unfold every fold over the line (folds nest; bounded), then centre it.
        for (let n = 0, fold = foldOver(v, i + 1); fold && n < 32; n++, fold = foldOver(v, i + 1)) {
          v.dispatch({ effects: unfoldEffect.of(fold) });
        }
        v.dispatch({ effects: EditorView.scrollIntoView(v.state.doc.line(i + 1).from, { y: 'center' }) });
        // CodeMirror renders the new viewport on its next measure frame.
        await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
      },
      rowElement: (i) => {
        const v = view;
        if (!v || i >= v.state.doc.lines) return null;
        const from = v.state.doc.line(i + 1).from;
        if (from < v.viewport.from || from > v.viewport.to || foldOver(v, i + 1)) return null;
        const { node } = v.domAtPos(from);
        const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
        return el?.closest('.cm-line') ?? null;
      },
    }),
  );

  /** Move keyboard focus into the editor (via `bind:this`). */
  export function focus(): void {
    view?.focus();
  }

  /** Insert `text` at the cursor (replacing the selection) as ONE undoable
   *  transaction, then focus the editor (the clipboard-history picker). */
  export function insertText(text: string): void {
    if (!view || readOnly) return;
    view.dispatch(view.state.replaceSelection(text), { scrollIntoView: true, userEvent: 'input.paste' });
    view.focus();
  }

  /** A fresh EditorState for `filePath` with the full extension set, wired to
   *  the current compartments (so live reconfigures keep reaching it). */
  function createState(filePath: string, fileContent: string): EditorState {
    appliedDialect = sqlDialect;
    const hk = readOnly ? null : (historyKey?.(filePath) ?? null);
    // Long-line plain mode needs the doc's lines up front (to pick the theme);
    // split them once here instead of letting EditorState.create do it again.
    const limit = highlightLineLimit;
    const doc = limit > 0 ? Text.of(fileContent.split(/\r\n?|\n/)) : null;
    appliedPlain = !!doc && hasLongLine(doc, limit);
    const langExt = cmLangFor(filePath, language);
    appliedLangReady = langExt != null || !cmLangPending(langExtOf(filePath, language));
    const baseExtensions: Extension[] = [
      ...(minimal ? [] : [lineNumbers(), foldGutter()]),
      indentOnInput(),
      bracketMatching(),
      // Auto-close brackets/quotes, and WRAP the selection when you type a
      // bracket/quote with text selected (e.g. select a word, press `"`).
      ...(readOnly ? [] : [closeBrackets()]),
      lintGutter(),
      drawSelection(),
      completionCompartment.of(completionExt()),
      placeholderCompartment.of(placeholderExt((appliedPlaceholder = placeholder))),
      wrapCompartment.of(wrapExt((appliedWrap = wrap))),
      placeholderTheme,
      search({ top: false }),
      themeCompartment.of(themeExt(ui.resolvedScheme, appliedPlain)),
      lspCompartment.of([]),
      selectionListener,
      changeListener,
      ...(limit > 0 ? [plainWatch] : []),
      changeFlushHandlers,
      submitKeymap,
      // When this editor owns find, claim the global Cmd/Ctrl+F opener while it
      // has focus so the keymap opens THIS editor's search/replace panel (with
      // working next/prev + replace) instead of the page-wide find overlay.
      ...(findOwner
        ? [
            EditorView.domEventHandlers({
              focus: () => {
                keyContext.openFind = openEditorSearch;
                return false;
              },
              blur: () => {
                if (keyContext.openFind === openEditorSearch) keyContext.openFind = null;
                return false;
              },
            }),
          ]
        : []),
      EditorState.readOnly.of(readOnly),
      // Highest-precedence Cmd/Ctrl+A → select the WHOLE document. `defaultKeymap`
      // already binds this (selectAll operates on the doc model, not the rendered
      // viewport), so this is belt-and-braces: it guarantees nothing can shadow
      // Mod-a and that CM handles the key (and preventDefault) before any other
      // extension. The macOS Edit ▸ Select All menu item used to shadow this
      // entirely (AppKit resolves ⌘A before the webview sees the key) and acted
      // on the virtualized contenteditable, selecting only the on-screen lines;
      // it is now a custom menu id routed through `lib/selectall.ts`, which
      // selects this view's whole DOCUMENT.
      Prec.highest(keymap.of([{ key: 'Mod-a', run: selectAll }])),
      keymap.of([
        ...(readOnly ? [] : closeBracketsKeymap),
        ...defaultKeymap,
        ...searchKeymap,
        ...lintKeymap,
        ...completionKeymap,
        ...foldKeymap,
        ...(readOnly ? [] : historyKeymap),
      ]),
      langCompartment.of(langExt ?? []),
      ...(readOnly ? [] : [history(hk ? { minDepth: 500 } : undefined)]),
      ...(hk ? [historySaver] : []),
      ...(oncopy ? [copyRecorder] : []),
    ];

    // A doc parked by an earlier editor instance (or a previous page load):
    // rebuild it with THIS instance's extensions, then reconcile the text with
    // `content` as one undoable change. A disk entry whose doc no longer
    // matches the draft is ignored (its history belongs to another text).
    if (hk?.persist) prepareEditorHistory();
    const saved = hk ? loadEditorState(hk.key) : null;
    if (saved && (!saved.fromDisk || saved.json.doc === fileContent)) {
      try {
        let st = EditorState.fromJSON(
          saved.json,
          { extensions: baseExtensions },
          { history: historyField },
        );
        if (st.doc.toString() !== fileContent) {
          st = st.update({ changes: { from: 0, to: st.doc.length, insert: fileContent } }).state;
        }
        pendingScrollTop = saved.scrollTop;
        return st;
      } catch {
        /* incompatible JSON (an older build) — start fresh below */
      }
    }

    return EditorState.create({
      doc: doc ?? fileContent,
      extensions: baseExtensions,
    });
  }

  // ── Shared history store (historyKey) ──────────────────────────────────────
  /** The path the live view shows (set by buildEditor / swapState). */
  let livePath = '';
  /** Scroll offset to restore once a store-rebuilt state is on screen. */
  let pendingScrollTop: number | null = null;
  let historySaveTimer: ReturnType<typeof setTimeout> | null = null;
  /** Park `state` (of `forPath`) in the shared store, when it opted in. */
  function parkHistory(forPath: string, state: EditorState, scrollTop: number): void {
    if (readOnly || !historyKey || !forPath) return;
    const hk = historyKey(forPath);
    if (!hk) return;
    try {
      saveEditorState(
        hk.key,
        state.toJSON({ history: historyField }) as { doc: string },
        scrollTop,
        // A doc already past the disk cap would only be serialized again and
        // dropped (editor-history.ts) — keep it in the memory tier only.
        hk.persist && state.doc.length <= PERSIST_MAX_BYTES,
      );
    } catch {
      /* a state without the history field — nothing worth keeping */
    }
  }
  function parkLive(): void {
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop);
  }
  function applyPendingScroll(): void {
    const top = pendingScrollTop;
    pendingScrollTop = null;
    if (top == null || top <= 0) return;
    requestAnimationFrame(() => {
      if (view) view.scrollDOM.scrollTop = top;
    });
  }
  // Save 1.5 s after the last edit (on disk ≈ 2 s), so a reload (no unmount)
  // still finds it; a page hide parks it immediately (registerLiveParker).
  const historySaver = EditorView.updateListener.of((u) => {
    if (!u.docChanged) return;
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    // The idle save exists so a RELOAD finds the history on disk. A doc past
    // the disk cap can't go there, so serializing it (doc + every undo step)
    // every 1.5 s of typing bought nothing; unmount / page hide still park it
    // in the memory tier (parkLive / registerLiveParker).
    if (u.state.doc.length > PERSIST_MAX_BYTES) return;
    historySaveTimer = setTimeout(() => {
      historySaveTimer = null;
      if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop);
    }, 1500);
  });
  // Copies/cuts made in the editor feed the clipboard ring (oncopy).
  const copyRecorder = EditorView.domEventHandlers({
    copy: (_e, v) => {
      recordCopy(v.state);
      return false;
    },
    cut: (_e, v) => {
      recordCopy(v.state);
      return false;
    },
  });
  function recordCopy(state: EditorState): void {
    const text = state.selection.ranges
      .filter((r) => !r.empty)
      .map((r) => state.sliceDoc(r.from, r.to))
      .join('\n');
    if (text) oncopy?.(text);
  }

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string): void {
    // Park the outgoing doc + every kept one before they are dropped.
    parkLive();
    for (const [p, k] of keptStates) parkHistory(p, k.state, 0);
    teardownEditor();
    lspCompartment = new Compartment();
    completionCompartment = new Compartment();
    placeholderCompartment = new Compartment();
    wrapCompartment = new Compartment();
    langCompartment = new Compartment();
    // Kept states are wired to the compartments just replaced — drop them.
    keptStates.clear();
    // Reset selection when a new file is opened
    sel = null;

    view = new EditorView({ state: createState(filePath, fileContent), parent: el });
    livePath = filePath;
    applyPendingScroll();
    ensureLang(filePath);

    // A caller-supplied completion source replaces LSP for this doc; only attach
    // the language server when no custom source is wired.
    if (!completionSource && lsp) void attachLsp(view, filePath, rootPath);

    // Jump to the requested line on first paint (e.g. a `file:line` ref clicked
    // in the terminal). Done after layout settles so scrollIntoView measures the
    // real geometry.
    if (gotoLine != null) revealLine(gotoLine, gotoCol);
    // A new document starts with a collapsed cursor at 0 — say so, so a caller
    // tracking the selection never keeps the previous document's.
    const built = view;
    untrack(() => emitSelection(built.state));
  }

  // ── Kept per-path states (keepStates) ──────────────────────────────────────

  /** Parked EditorStates by path, oldest first (Map order = LRU order). */
  const keptStates = new Map<
    string,
    { state: EditorState; scroll: StateEffect<unknown>; dialect: SqlDialectName; langReady: boolean }
  >();
  const MAX_KEPT_STATES = 24;

  /**
   * Switch the live view from `fromPath` to `toPath` without rebuilding it:
   * park the current state, then restore `toPath`'s parked state (brought up
   * to date with `content` and the current live options) or create a fresh one.
   */
  function swapState(fromPath: string, toPath: string, toContent: string): void {
    if (!view) return;
    // See teardownEditor: `onchange` already targets `toPath`.
    changes.cancel();
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    keptStates.delete(fromPath);
    keptStates.set(fromPath, {
      state: view.state,
      scroll: view.scrollSnapshot(),
      dialect: appliedDialect,
      langReady: appliedLangReady,
    });
    while (keptStates.size > MAX_KEPT_STATES) {
      const oldest = keptStates.keys().next().value;
      if (oldest === undefined) break;
      // Evicted from this instance: spill it to the shared store (historyKey).
      const ev = keptStates.get(oldest);
      if (ev) parkHistory(oldest, ev.state, 0);
      keptStates.delete(oldest);
    }
    livePath = toPath;
    const kept = keptStates.get(toPath);
    keptStates.delete(toPath);
    if (!kept) {
      view.setState(createState(toPath, toContent));
      applyPendingScroll();
      ensureLang(toPath);
    } else {
      view.setState(kept.state);
      if (plainRecheck) clearTimeout(plainRecheck);
      plainRecheck = null;
      appliedPlain = highlightLineLimit > 0 && hasLongLine(kept.state.doc, highlightLineLimit);
      // Options may have changed while the state was parked; the text may have
      // been edited from outside (store/agent) — reconcile both, undoably.
      const effects: StateEffect<unknown>[] = [
        completionCompartment.reconfigure(completionExt()),
        placeholderCompartment.reconfigure(placeholderExt((appliedPlaceholder = placeholder))),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap))),
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        kept.scroll,
      ];
      // Re-parse only when the dialect changed while parked (a connection
      // switch) — a plain tab switch keeps the parked tree.
      appliedDialect = kept.dialect;
      appliedLangReady = kept.langReady;
      if (sqlDialect !== kept.dialect || !kept.langReady) {
        appliedDialect = sqlDialect;
        const langExt = cmLangFor(toPath, language);
        // Parked before its pack loaded: apply it now if it has since.
        if (langExt) appliedLangReady = true;
        effects.push(langCompartment.reconfigure(langExt ?? []));
      }
      const doc = view.state.doc;
      const stale = doc.length !== toContent.length || doc.toString() !== toContent;
      view.dispatch({
        effects,
        ...(stale ? { changes: { from: 0, to: doc.length, insert: toContent } } : {}),
      });
      ensureLang(toPath);
    }
    prevCompletion = completionSource;
    // setState fires no update listeners — re-announce the restored selection.
    emitSelection(view.state);
  }

  /** Scroll to + select the given 1-based line (clamped to the doc), centering
   *  it in the viewport. `col` (1-based) refines the cursor within the line. */
  function revealLine(line: number, col?: number | null): void {
    if (!view) return;
    const doc = view.state.doc;
    const lineNo = Math.max(1, Math.min(line, doc.lines));
    const lineInfo = doc.line(lineNo);
    const pos =
      col != null && col > 0
        ? Math.min(lineInfo.from + (col - 1), lineInfo.to)
        : lineInfo.from;
    requestAnimationFrame(() => {
      if (!view) return;
      try {
        view.dispatch({
          selection: { anchor: pos, head: pos },
          effects: EditorView.scrollIntoView(pos, { y: 'center' }),
        });
        view.focus();
      } catch {
        /* doc replaced mid-flight — harmless */
      }
    });
  }

  // ── Reactive effects ───────────────────────────────────────────────────────

  // Initial mount + rebuild when path/content/root change
  let prevPath = '';
  let prevRoot = '';
  let prevContent = '';

  $effect(() => {
    const el = container;
    const curPath = path;
    const curContent = content;
    const curRoot = root;
    if (!el) return;

    // Always rebuild on first mount or when key props change — but skip a
    // rebuild when the content prop merely echoes back an edit we just emitted
    // (editable mode), which would remount the view and drop the cursor.
    const contentEchoed = curContent === lastEmitted;
    if (view && keepStates && curPath !== prevPath && curRoot === prevRoot) {
      // Another document in the same editor (a DB query tab): swap states.
      const fromPath = prevPath;
      prevPath = curPath;
      prevContent = curContent;
      untrack(() => swapState(fromPath, curPath, curContent));
    } else if (!view || curPath !== prevPath || curRoot !== prevRoot) {
      // Structural (re)build: first mount, a different file, or a new root.
      prevPath = curPath;
      prevRoot = curRoot;
      prevContent = curContent;
      buildEditor(el, curPath, curContent, curRoot);
    } else if (curContent !== prevContent && !contentEchoed) {
      // EXTERNAL content change for the SAME doc (e.g. "Query by value", Format,
      // var substitution). Apply it as an undoable transaction rather than
      // rebuilding the view — a rebuild teardowns the EditorView and WIPES the
      // undo history, so Cmd+Z couldn't revert a "Query by value". A transaction
      // lands in the history stack, so undo restores the prior statement.
      prevContent = curContent;
      if (curContent !== view.state.doc.toString()) {
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: curContent },
        });
      }
    } else if (curContent !== prevContent) {
      // Content equals our last emit — record it without rebuilding.
      prevContent = curContent;
    }

    // NOTE: deliberately no cleanup returned here. A Svelte 5 $effect cleanup
    // runs *before every re-run* (not only on unmount), so tearing the editor
    // down here destroyed the view on every keystroke — `content` echoes back
    // our own edit, the effect re-runs, the (now-destroyed) view fails the
    // `!view` guard and rebuilds, dropping focus after a single character.
    // Teardown on unmount is handled by onDestroy; rebuilds are handled by
    // buildEditor() (which tears down first).
  });

  // Reconfigure the completion source live (no remount) when it changes — the
  // DB query editor swaps it as the active connection/engine changes.
  let prevCompletion: CompletionSource | null = null;
  $effect(() => {
    const src = completionSource;
    if (!view) return;
    if (src === prevCompletion) return;
    prevCompletion = src;
    view.dispatch({ effects: completionCompartment.reconfigure(completionExt()) });
  });

  // Toggle soft-wrapping live (no remount).
  $effect(() => {
    const on = wrap;
    if (!view || on === appliedWrap) return;
    appliedWrap = on;
    view.dispatch({ effects: wrapCompartment.reconfigure(wrapExt(on)) });
  });

  // Re-tokenize with the new SQL dialect live (no remount) — e.g. the DB query
  // editor switching from a Postgres to a MySQL connection.
  $effect(() => {
    const d = sqlDialect;
    if (!view || d === appliedDialect) return;
    appliedDialect = d;
    const langExt = untrack(() => cmLangFor(path, language));
    if (langExt) view.dispatch({ effects: langCompartment.reconfigure(langExt) });
  });

  // Swap the empty-doc hint live (no remount) when the caller changes it.
  $effect(() => {
    const text = placeholder;
    if (!view || text === appliedPlaceholder) return;
    appliedPlaceholder = text;
    view.dispatch({ effects: placeholderCompartment.reconfigure(placeholderExt(text)) });
  });

  // Re-theme live when the app scheme (light/dark) changes.
  $effect(() => {
    const scheme = ui.resolvedScheme;
    if (view) view.dispatch({ effects: themeCompartment.reconfigure(themeExt(scheme, appliedPlain)) });
  });

  // Re-reveal when the target line/col changes for an already-mounted doc (e.g.
  // clicking a second `file:line` ref into the same open file — no rebuild). The
  // initial reveal happens inside buildEditor on mount.
  let prevGoto: string | null = null;
  $effect(() => {
    const line = gotoLine;
    const col = gotoCol;
    if (!view || line == null) return;
    const key = `${line}:${col ?? ''}`;
    if (key === prevGoto) return;
    prevGoto = key;
    revealLine(line, col);
  });

  // A reload / app hide parks the live doc right away (only with historyKey;
  // parkHistory is a no-op otherwise).
  const unregisterParker = registerLiveParker(() => {
    if (historySaveTimer !== null) parkLive();
  });
  onDestroy(() => {
    unregisterParker();
    // Same doc, going away (a tab/view toggle): deliver its last edit.
    changes.flush();
    // Keep every doc's undo history for the next editor instance (historyKey).
    parkLive();
    for (const [p, k] of keptStates) parkHistory(p, k.state, 0);
    teardownEditor();
  });
</script>

<div class="code-editor-outer" data-lang={language ?? ''}>
  <div class="code-editor-wrap" dir="ltr" bind:this={container}></div>
  {#if sel}
    <button class="send-to-agent-btn" onclick={sendToAgent} type="button">
      Send to agent ↗
    </button>
  {/if}
</div>

<style>
  .code-editor-outer {
    position: relative;
    width: 100%;
    height: 100%;
  }

  .code-editor-wrap {
    width: 100%;
    height: 100%;
    overflow: auto;
  }

  .send-to-agent-btn {
    position: absolute;
    top: 6px;
    inset-inline-end: 10px;
    z-index: 20;
    padding: 3px 10px;
    font-size: var(--fs-xs);
    font-family: var(--font-ui);
    font-weight: 500;
    color: var(--accent-contrast);
    background: var(--accent-solid);
    border: none;
    border-radius: 4px;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    transition: background 0.15s;
    white-space: nowrap;
    user-select: none;
  }

  .send-to-agent-btn:hover {
    background: color-mix(in srgb, var(--accent-solid) 88%, var(--text));
  }

  .send-to-agent-btn:active {
    background: color-mix(in srgb, var(--accent-solid) 80%, var(--bg));
  }

  /* Make the CM editor fill the container fully */
  .code-editor-wrap :global(.cm-editor) {
    height: 100%;
    font-family: var(--font-mono, 'SF Mono', SFMono-Regular, Menlo, Monaco, 'Courier New', monospace);
    font-size: var(--fs-xs);
    line-height: 1.55;
  }

  .code-editor-wrap :global(.cm-scroller) {
    overflow: auto;
  }

  /* One-dark background matches the app dark theme */
  .code-editor-wrap :global(.cm-editor.cm-focused) {
    outline: none;
  }
</style>
