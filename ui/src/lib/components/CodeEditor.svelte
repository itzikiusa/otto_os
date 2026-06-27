<script lang="ts">
  // CodeMirror 6 editor with LSP hover/diagnostics/completion/definitions.
  // readOnly=true by default (Files viewer is read-only; LSP still works).
  import { onDestroy } from 'svelte';
  import { EditorView, lineNumbers, keymap, drawSelection } from '@codemirror/view';
  import { EditorState, Compartment, Prec } from '@codemirror/state';
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
  import { redisLang } from './redis-lang';

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
    /** Fired with the full document text on every edit (only when !readOnly). */
    onchange?: (value: string) => void;
    /**
     * Optional custom autocompletion source. When set, it overrides the default
     * completion (and suppresses LSP for the doc) — used by the DB query editor
     * to surface server-driven SQL/Redis/Mongo completions. Reapplied live via a
     * Compartment so callers can toggle it without remounting the editor.
     */
    completionSource?: CompletionSource | null;
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
  }

  let {
    path,
    content,
    root,
    language,
    readOnly = true,
    onchange,
    completionSource = null,
    minimal = false,
    onsubmit,
    onselect,
    gotoLine = null,
    gotoCol = null,
    findOwner = false,
  }: Props = $props();

  // ── Container ─────────────────────────────────────────────────────────────

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let lspCompartment = new Compartment();
  // Holds either the default autocompletion() or one overridden with the
  // caller's completionSource (DB query editor). Reconfigured reactively.
  let completionCompartment = new Compartment();

  // ── Language extension map ─────────────────────────────────────────────────

  type AnyLangExtension = ReturnType<typeof javascript>;

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
    sql:  () => sql(),
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

  interface Sel {
    text: string;
    startLine: number;
    endLine: number;
  }

  let sel: Sel | null = $state(null);

  /** CodeMirror update listener that tracks the current text selection. */
  const selectionListener = EditorView.updateListener.of((update) => {
    if (!update.selectionSet && !update.docChanged) return;
    const { from, to, head } = update.state.selection.main;
    if (from === to) {
      sel = null;
    } else {
      const text = update.state.sliceDoc(from, to);
      const startLine = update.state.doc.lineAt(from).number;
      const endLine = update.state.doc.lineAt(to).number;
      sel = { text, startLine, endLine };
    }
    // Surface selection + cursor so callers can run only the selected/current
    // statement (text is '' when there's no selection).
    onselect?.({ text: from === to ? '' : update.state.sliceDoc(from, to), cursor: head });
  });

  // Last value we emitted via onchange — lets the rebuild effect ignore the
  // content prop echoing back our own edit (which would needlessly remount and
  // drop the cursor).
  let lastEmitted: string | null = null;

  /** Emits the full doc text on edits so editable callers stay in sync. */
  const changeListener = EditorView.updateListener.of((update) => {
    if (!update.docChanged || !onchange) return;
    const value = update.state.doc.toString();
    lastEmitted = value;
    onchange(value);
  });

  // ── Send-to-agent handler ─────────────────────────────────────────────────

  async function sendToAgent(): Promise<void> {
    if (!sel) return;

    const sessionId = ws.activeSessionId;
    if (!sessionId || ws.activeSession?.kind !== 'agent') {
      toasts.error('No agent session', 'Open an agent session first.');
      return;
    }

    const ext = extOf(path);
    const langHint = ext || '';
    const snippet = `Re: ${path}:${sel.startLine}-${sel.endLine}\n\n\`\`\`${langHint}\n${sel.text}\n\`\`\`\n\n`;

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
      ? [autocompletion({ override: [completionSource], activateOnTyping: true }), autoTriggerExt()]
      : autocompletion();
  }

  // Keywords after which a SPACE should open completion (so `select * from `
  // immediately offers tables, `where `/`and ` offers columns).
  const SPACE_TRIGGER_RE =
    /(?:^|[\s({,])(?:from|join|where|and|or|on|into|update|set|by|using|select)\s$/i;

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
          const text = inserted.toString();
          if (text === '.') {
            fire = true;
          } else if (text === ' ') {
            const head = u.state.selection.main.head;
            const back = u.state.sliceDoc(Math.max(0, head - 48), head);
            if (SPACE_TRIGGER_RE.test(back) || /[({,]\s$/.test(back)) fire = true;
          }
        });
      }
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

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string): void {
    teardownEditor();
    lspCompartment = new Compartment();
    completionCompartment = new Compartment();

    const langExt = cmLangFor(filePath, language);
    // Reset selection when a new file is opened
    sel = null;

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
      search({ top: false }),
      themeCompartment.of(themeExt(ui.resolvedScheme)),
      lspCompartment.of([]),
      selectionListener,
      changeListener,
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
      // extension. Note: a macOS webview's native "Select All" menu action can
      // still act directly on the virtualized contenteditable (only on-screen
      // lines exist in the DOM) — that path is outside CM's keymap.
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
      ...(langExt ? [langExt] : []),
      ...(readOnly ? [] : [history()]),
    ];

    const state = EditorState.create({
      doc: fileContent,
      extensions: baseExtensions,
    });

    view = new EditorView({ state, parent: el });

    // A caller-supplied completion source replaces LSP for this doc; only attach
    // the language server when no custom source is wired.
    if (!completionSource) void attachLsp(view, filePath, rootPath);

    // Jump to the requested line on first paint (e.g. a `file:line` ref clicked
    // in the terminal). Done after layout settles so scrollIntoView measures the
    // real geometry.
    if (gotoLine != null) revealLine(gotoLine, gotoCol);
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
    if (!view || curPath !== prevPath || curRoot !== prevRoot) {
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
    teardownEditor();
  });
</script>

<div class="code-editor-outer" data-lang={language ?? ''}>
  <div class="code-editor-wrap" bind:this={container}></div>
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
    font-size: 11px;
    font-family: var(--font-sans, system-ui, sans-serif);
    font-weight: 500;
    color: #e2e8f0;
    background: #2d5eff;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    transition: background 0.15s;
    white-space: nowrap;
    user-select: none;
  }

  .send-to-agent-btn:hover {
    background: #4a72ff;
  }

  .send-to-agent-btn:active {
    background: #1a43cc;
  }

  /* Make the CM editor fill the container fully */
  .code-editor-wrap :global(.cm-editor) {
    height: 100%;
    font-family: var(--font-mono, 'SF Mono', SFMono-Regular, Menlo, Monaco, 'Courier New', monospace);
    font-size: 11px;
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
