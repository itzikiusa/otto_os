<script lang="ts">
  // CodeMirror 6 editor with LSP hover/diagnostics/completion/definitions.
  // readOnly=true by default (Files viewer is read-only; LSP still works).
  import { onDestroy, untrack } from 'svelte';
  import { EditorView, lineNumbers, keymap, drawSelection, placeholder as cmPlaceholder } from '@codemirror/view';
  import { EditorState, Compartment, Prec, type StateEffect } from '@codemirror/state';
  import { defaultKeymap, history, historyKeymap, selectAll } from '@codemirror/commands';
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
  } from '@codemirror/language';
  import { oneDark } from '@codemirror/theme-one-dark';
  import type { Extension } from '@codemirror/state';

  // Language packages
  import { javascript } from '@codemirror/lang-javascript';
  import { python } from '@codemirror/lang-python';
  import { go } from '@codemirror/lang-go';
  import { rust } from '@codemirror/lang-rust';
  import { json } from '@codemirror/lang-json';
  import { html } from '@codemirror/lang-html';
  import { css } from '@codemirror/lang-css';
  import { markdown } from '@codemirror/lang-markdown';
  import { java } from '@codemirror/lang-java';
  import { sql } from '@codemirror/lang-sql';
  import { sqlDialect as dialectFor, type SqlDialectName } from '../sql-dialects';
  import { redisLang } from './redis-lang';
  import { createChangeEmitter } from './changeEmitter';

  // LSP — use the all-in-one factory that manages the WS transport internally
  import { languageServer } from '@marimo-team/codemirror-languageserver';

  import { api, baseUrl } from '../api/client';
  import type { LspCapabilities } from '../api/types';
  import { ws } from '../stores/workspace.svelte';
  import { ui } from '../stores/ui.svelte';
  import { keyContext } from '../keys';
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
  function themeExt(scheme: 'light' | 'dark'): Extension {
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
     * When true, this editor OWNS Cmd/Ctrl+F while focused: it registers a global
     * find opener (like the terminal) so the keymap opens CodeMirror's in-editor
     * search/replace panel here instead of the page-wide find-in-page overlay
     * (whose match navigation can't reach the editor's virtualized lines). Used
     * by the DB query editor.
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
    findOwner = false,
    placeholder = '',
    wrap = false,
    keepStates = false,
    lsp = false,
    sqlDialect = 'standard',
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

  // ── Language extension map ─────────────────────────────────────────────────

  type AnyLangExtension = ReturnType<typeof javascript>;

  /** The dialect the live view's language was built with (plain field). */
  let appliedDialect: SqlDialectName = 'standard';
  /** Holds the language extension so a dialect change reaches a live view. */
  let langCompartment = new Compartment();

  const EXT_TO_CM_LANG: Record<string, () => AnyLangExtension> = {
    js:   () => javascript(),
    jsx:  () => javascript({ jsx: true }),
    ts:   () => javascript({ typescript: true }),
    tsx:  () => javascript({ jsx: true, typescript: true }),
    mjs:  () => javascript(),
    cjs:  () => javascript(),
    py:   () => python(),
    go:   () => go(),
    rs:   () => rust(),
    json: () => json(),
    jsonc:() => json(),
    html: () => html(),
    htm:  () => html(),
    xml:  () => html(),
    css:  () => css(),
    scss: () => css(),
    less: () => css(),
    md:   () => markdown(),
    mdx:  () => markdown(),
    java: () => java(),
    sql:  () => sql({ dialect: dialectFor(appliedDialect) }),
    redis: () => redisLang() as AnyLangExtension,
  };

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

  function cmLangFor(filePath: string, hint?: string): AnyLangExtension | null {
    const ext = extOf(filePath) || (hint ?? '');
    return EXT_TO_CM_LANG[ext]?.() ?? null;
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

  /** A fresh EditorState for `filePath` with the full extension set, wired to
   *  the current compartments (so live reconfigures keep reaching it). */
  function createState(filePath: string, fileContent: string): EditorState {
    appliedDialect = sqlDialect;
    const langExt = cmLangFor(filePath, language);
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
      themeCompartment.of(themeExt(ui.resolvedScheme)),
      lspCompartment.of([]),
      selectionListener,
      changeListener,
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
      ...(readOnly ? [] : [history()]),
    ];

    return EditorState.create({
      doc: fileContent,
      extensions: baseExtensions,
    });
  }

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string): void {
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
    { state: EditorState; scroll: StateEffect<unknown>; dialect: SqlDialectName }
  >();
  const MAX_KEPT_STATES = 8;

  /**
   * Switch the live view from `fromPath` to `toPath` without rebuilding it:
   * park the current state, then restore `toPath`'s parked state (brought up
   * to date with `content` and the current live options) or create a fresh one.
   */
  function swapState(fromPath: string, toPath: string, toContent: string): void {
    if (!view) return;
    // See teardownEditor: `onchange` already targets `toPath`.
    changes.cancel();
    keptStates.delete(fromPath);
    keptStates.set(fromPath, {
      state: view.state,
      scroll: view.scrollSnapshot(),
      dialect: appliedDialect,
    });
    while (keptStates.size > MAX_KEPT_STATES) {
      const oldest = keptStates.keys().next().value;
      if (oldest === undefined) break;
      keptStates.delete(oldest);
    }
    const kept = keptStates.get(toPath);
    keptStates.delete(toPath);
    if (!kept) {
      view.setState(createState(toPath, toContent));
    } else {
      view.setState(kept.state);
      // Options may have changed while the state was parked; the text may have
      // been edited from outside (store/agent) — reconcile both, undoably.
      const effects: StateEffect<unknown>[] = [
        completionCompartment.reconfigure(completionExt()),
        placeholderCompartment.reconfigure(placeholderExt((appliedPlaceholder = placeholder))),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap))),
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme)),
        kept.scroll,
      ];
      // Re-parse only when the dialect changed while parked (a connection
      // switch) — a plain tab switch keeps the parked tree.
      appliedDialect = kept.dialect;
      if (sqlDialect !== kept.dialect) {
        appliedDialect = sqlDialect;
        effects.push(langCompartment.reconfigure(cmLangFor(toPath, language) ?? []));
      }
      const doc = view.state.doc;
      const stale = doc.length !== toContent.length || doc.toString() !== toContent;
      view.dispatch({
        effects,
        ...(stale ? { changes: { from: 0, to: doc.length, insert: toContent } } : {}),
      });
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
    if (view) view.dispatch({ effects: themeCompartment.reconfigure(themeExt(scheme)) });
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

  onDestroy(() => {
    // Same doc, going away (a tab/view toggle): deliver its last edit.
    changes.flush();
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
