<script lang="ts">
  // CodeMirror 6 editor with LSP hover/diagnostics/completion/definitions.
  // readOnly=true by default (Files viewer is read-only; LSP still works).
  import { onDestroy } from 'svelte';
  import { EditorView, lineNumbers, keymap } from '@codemirror/view';
  import { EditorState, Compartment } from '@codemirror/state';
  import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
  import { search, searchKeymap } from '@codemirror/search';
  import { autocompletion, completionKeymap } from '@codemirror/autocomplete';
  import { lintGutter, lintKeymap } from '@codemirror/lint';
  import { indentOnInput, bracketMatching, foldGutter, foldKeymap } from '@codemirror/language';
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

  // LSP — use the all-in-one factory that manages the WS transport internally
  import { languageServer } from '@marimo-team/codemirror-languageserver';

  import { api, baseUrl } from '../api/client';
  import type { LspCapabilities } from '../api/types';
  import { ws } from '../stores/workspace.svelte';
  import { toasts } from '../toast.svelte';

  // ── Props ──────────────────────────────────────────────────────────────────

  interface Props {
    path: string;
    content: string;
    root: string;
    language?: string;
    readOnly?: boolean;
    /** Fired with the full document text on every edit (only when !readOnly). */
    onchange?: (value: string) => void;
  }

  let { path, content, root, language, readOnly = true, onchange }: Props = $props();

  // ── Container ─────────────────────────────────────────────────────────────

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let lspCompartment = new Compartment();

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
    css:  () => css(),
    scss: () => css(),
    less: () => css(),
    md:   () => markdown(),
    mdx:  () => markdown(),
    java: () => java(),
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
    const { from, to } = update.state.selection.main;
    if (from === to) {
      sel = null;
    } else {
      const text = update.state.sliceDoc(from, to);
      const startLine = update.state.doc.lineAt(from).number;
      const endLine = update.state.doc.lineAt(to).number;
      sel = { text, startLine, endLine };
    }
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

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string): void {
    teardownEditor();
    lspCompartment = new Compartment();

    const langExt = cmLangFor(filePath, language);
    // Reset selection when a new file is opened
    sel = null;

    const baseExtensions: Extension[] = [
      lineNumbers(),
      foldGutter(),
      indentOnInput(),
      bracketMatching(),
      lintGutter(),
      autocompletion(),
      search({ top: false }),
      oneDark,
      lspCompartment.of([]),
      selectionListener,
      changeListener,
      EditorState.readOnly.of(readOnly),
      keymap.of([
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

    // Attach LSP without awaiting — failures are swallowed inside
    void attachLsp(view, filePath, rootPath);
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
    if (!view || curPath !== prevPath || curRoot !== prevRoot || (curContent !== prevContent && !contentEchoed)) {
      prevPath = curPath;
      prevRoot = curRoot;
      prevContent = curContent;
      buildEditor(el, curPath, curContent, curRoot);
    } else if (curContent !== prevContent) {
      // Content equals our last emit — record it without rebuilding.
      prevContent = curContent;
    }

    return () => teardownEditor();
  });

  onDestroy(() => {
    teardownEditor();
  });
</script>

<div class="code-editor-outer">
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
    right: 10px;
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
