<script lang="ts">
  // Interactive Jupyter Notebook (.ipynb) editor for the Otto IDE.
  // Supports opening/editing/saving nbformat v4.5 notebooks, running individual or
  // all code cells on a persistent built-in Python kernel, rich outputs (streams,
  // _repr_html_ tables in sandboxed iframes, matplotlib/PNG/SVG images, tracebacks),
  // interpreter selection (venv/uv/conda/poetry/system Python), GPU/MPS badge,
  // interrupt, restart, live Variables inspector, and isolated cell DAP debugging.
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../../../lib/api/client';
  import type {
    IdeMlAcceleratorInfo,
    IdeNotebookExecuteResp,
    IdeNotebookKernelInfo,
    IdeNotebookKernelsResp,
    IdeNotebookVariable,
    IdeNotebookVariablesResp,
  } from '../../../lib/api/types';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { ide } from '../ideStore.svelte';
  import {
    addNotebookCell,
    changeNotebookCellType,
    clearNotebookOutputs,
    deleteNotebookCell,
    moveNotebookCell,
    normalizeNotebookOutputs,
    parseIpynb,
    serializeIpynb,
    updateNotebookCellSource,
    type NotebookCellType,
    type NotebookDoc,
  } from './ideNotebookModel';
  import { executeNotebookCellGuarded } from './ideNotebookRunner';

  interface Props {
    path: string;
    content: string;
    readOnly?: boolean;
    onchange: (targetPath: string, nextJson: string) => void;
    onswitchtext: (targetPath: string) => void;
  }

  let { path, content, readOnly = false, onchange, onswitchtext }: Props = $props();

  let doc = $state<NotebookDoc>(parseIpynb('').doc);
  let parseError = $state<string | null>(null);
  let lastSerialized = '';
  let lastParsedPath = '';
  let kernels = $state<IdeNotebookKernelInfo[]>([]);
  let accelerators = $state<IdeMlAcceleratorInfo[]>([]);
  let selectedInterpreter = $state('');
  let kernelsLoading = $state(false);
  let runningCellId = $state<string | null>(null);
  let runningAll = $state(false);
  let variables = $state<IdeNotebookVariable[]>([]);
  let varsOpen = $state(false);
  let activeSessionId = $state<string | null>(null);
  let disposed = false;
  let viewToken = 0;

  onDestroy(() => {
    disposed = true;
    viewToken += 1;
    runningAll = false;
  });

  function switchToRawText(): void {
    const origPath = path;
    disposed = true;
    viewToken += 1;
    runningAll = false;
    onswitchtext(origPath);
  }

  $effect(() => {
    if (path === lastParsedPath && content === lastSerialized) return;
    const res = parseIpynb(content);
    doc = res.doc;
    parseError = res.parseError;
    lastParsedPath = path;
    lastSerialized = content;
  });

  $effect(() => {
    const p = path;
    untrack(() => {
      viewToken += 1;
      runningCellId = null;
      runningAll = false;
      variables = [];
      activeSessionId = null;
      if (ide.hasTarget && p) {
        void refreshKernels(p);
      }
    });
  });

  function commitDoc(next: NotebookDoc, targetPath: string = path): void {
    if (disposed || targetPath !== path) return;
    doc = next;
    const json = serializeIpynb(next);
    lastParsedPath = targetPath;
    lastSerialized = json;
    onchange(targetPath, json);
  }

  async function refreshKernels(targetPath: string = path): Promise<void> {
    if (!ide.hasTarget) return;
    kernelsLoading = true;
    try {
      const resp = await api.post<IdeNotebookKernelsResp>('/ide/notebooks/kernels', {
        target: ide.target,
        notebook_path: targetPath,
      });
      if (path !== targetPath) return;
      kernels = resp.kernels ?? [];
      accelerators = resp.accelerators ?? [];
      activeSessionId = resp.active_session_id ?? null;
      if (!selectedInterpreter) {
        const active =
          resp.active_interpreter ||
          kernels.find((k) => k.active)?.interpreter ||
          kernels[0]?.interpreter ||
          '';
        selectedInterpreter = active;
      }
    } catch {
      /* kernels optional while offline or SSH */
    } finally {
      if (path === targetPath) kernelsLoading = false;
    }
  }

  async function runCellById(cellId: string): Promise<boolean> {
    return executeNotebookCellGuarded(cellId, {
      getDoc: () => doc,
      getPath: () => path,
      isDisposed: () => disposed,
      getViewToken: () => viewToken,
      getRunningCellId: () => runningCellId,
      setRunningCellId: (id) => { runningCellId = id; },
      setSessionAndVars: (sid, vars) => {
        activeSessionId = sid;
        variables = vars;
      },
      commitDoc,
      onError: (e) => toastError('Couldn’t run cell', e),
      executeRequest: (origPath, code) =>
        api.post<IdeNotebookExecuteResp>('/ide/notebooks/execute', {
          target: ide.target,
          notebook_path: origPath,
          code,
          interpreter: selectedInterpreter || undefined,
        }),
    });
  }

  async function runCell(index: number): Promise<boolean> {
    const cell = doc.cells[index];
    if (!cell) return false;
    return runCellById(cell.id);
  }

  async function runAllCells(): Promise<void> {
    if (runningAll || runningCellId !== null) return;
    const origPath = path;
    const codeIds = doc.cells.filter((c) => c.cell_type === 'code').map((c) => c.id);
    runningAll = true;
    try {
      for (const id of codeIds) {
        if (!runningAll || path !== origPath) break;
        const ok = await runCellById(id);
        if (!ok) break;
      }
    } finally {
      runningAll = false;
    }
  }

  async function interruptKernel(): Promise<void> {
    runningAll = false;
    const origPath = path;
    try {
      await api.post('/ide/notebooks/interrupt', { target: ide.target, notebook_path: origPath });
      toasts.info('Interrupt sent to Python kernel');
    } catch (e) {
      toastError('Couldn’t interrupt kernel', e);
    }
  }

  async function restartKernel(): Promise<void> {
    runningAll = false;
    const origPath = path;
    try {
      await api.post('/ide/notebooks/restart', {
        target: ide.target,
        notebook_path: origPath,
        interpreter: selectedInterpreter || undefined,
      });
      if (path !== origPath) return;
      variables = [];
      toasts.success('Python kernel restarted');
      await refreshKernels(origPath);
    } catch (e) {
      toastError('Couldn’t restart kernel', e);
    }
  }

  async function toggleVariables(): Promise<void> {
    varsOpen = !varsOpen;
    const origPath = path;
    if (varsOpen && activeSessionId) {
      try {
        const resp = await api.post<IdeNotebookVariablesResp>('/ide/notebooks/variables', {
          target: ide.target,
          notebook_path: origPath,
        });
        if (path === origPath) variables = resp.variables ?? [];
      } catch {
        /* keep last known variables */
      }
    }
  }

  async function debugCell(index: number): Promise<void> {
    const cell = doc.cells[index];
    if (!cell || cell.cell_type !== 'code') return;
    const origPath = path;
    const saved = await ide.saveFile(origPath);
    if (!saved || path !== origPath) return;
    await ide.startDebug({
      language: 'python',
      launch: {
        program: origPath,
        args: ['--cell', String(index)],
        cwd: ide.target.root,
        sdk_path: selectedInterpreter || undefined,
      },
    });
  }

  function formatAccelBadge(a: IdeMlAcceleratorInfo): string {
    const mem = a.memory_mb ? ` · ${Math.round(a.memory_mb / 1024)} GB` : '';
    return `${a.kind.toUpperCase()}: ${a.name}${mem}`;
  }

  function handleCellKeydown(e: KeyboardEvent, index: number): void {
    if ((e.metaKey || e.ctrlKey || e.shiftKey) && e.key === 'Enter') {
      e.preventDefault();
      void runCell(index);
    }
  }
</script>

<div class="nb-root" data-testid="ide-notebook-view">
  <header class="nb-bar">
    <div class="nb-bar-group">
      <button
        type="button"
        class="btn small primary"
        disabled={readOnly || runningCellId !== null || runningAll}
        onclick={() => void runAllCells()}
        data-testid="ide-nb-run-all"
      >
        <Icon name="play" size={12} />
        Run All
      </button>
      <button
        type="button"
        class="btn small ghost"
        disabled={runningCellId === null && !runningAll}
        onclick={() => void interruptKernel()}
        title="Interrupt running cell (SIGINT)"
        aria-label="Interrupt running cell"
      >
        <Icon name="stop" size={12} />
        Interrupt
      </button>
      <button
        type="button"
        class="btn small ghost"
        disabled={readOnly || runningCellId !== null}
        onclick={() => void restartKernel()}
        title="Restart kernel"
        aria-label="Restart kernel"
      >
        <Icon name="refresh" size={12} />
        Restart
      </button>
      <button
        type="button"
        class="btn small ghost"
        disabled={readOnly}
        onclick={() => commitDoc(clearNotebookOutputs(doc))}
        title="Clear all outputs"
      >
        Clear Outputs
      </button>
      <button
        type="button"
        class="btn small ghost"
        class:is-on={varsOpen}
        onclick={() => void toggleVariables()}
        data-testid="ide-nb-vars-toggle"
      >
        Variables ({variables.length})
      </button>
    </div>

    <div class="nb-bar-group nb-bar-end">
      {#if accelerators.length > 0}
        <Badge tone="accent" variant="outline" label={formatAccelBadge(accelerators[0])} />
      {/if}
      <select
        class="input nb-kernel-sel"
        aria-label="Python interpreter kernel"
        title="Built-in Python kernel (standard Python; IPython %magic not supported)"
        disabled={kernelsLoading || runningCellId !== null}
        bind:value={selectedInterpreter}
        data-testid="ide-nb-kernel-select"
      >
        {#if kernels.length === 0}
          <option value="">Python 3 (auto)</option>
        {:else}
          {#each kernels as k (k.interpreter)}
            <option value={k.interpreter}>{k.display_name}</option>
          {/each}
        {/if}
      </select>
      <button
        type="button"
        class="btn small ghost"
        onclick={switchToRawText}
        title="View raw .ipynb JSON"
      >
        JSON
      </button>
    </div>
  </header>

  {#if parseError}
    <div class="nb-banner-err">
      <span>Couldn’t parse notebook JSON: {parseError}</span>
      <button type="button" class="btn small" onclick={switchToRawText}>Edit raw JSON</button>
    </div>
  {/if}

  <div class="nb-body">
    <div class="nb-cells">
      {#each doc.cells as cell, idx (cell.id)}
        {@const isRunning = runningCellId === cell.id}
        {@const normOutputs = normalizeNotebookOutputs(cell.outputs)}
        <section class="nb-cell" class:is-code={cell.cell_type === 'code'} data-testid={`ide-nb-cell-${idx}`}>
          <div class="nb-cell-head">
            <div class="nb-cell-meta">
              {#if cell.cell_type === 'code'}
                <span class="nb-exec-count">[{isRunning ? '*' : cell.execution_count ?? ' '}]</span>
                <button
                  type="button"
                  class="icon-btn nb-run-btn"
                  disabled={readOnly || runningCellId !== null}
                  onclick={() => void runCell(idx)}
                  aria-label={`Run cell ${idx + 1}`}
                  title="Run cell (⇧⏎ / ⌘⏎)"
                  data-testid={`ide-nb-run-${idx}`}
                >
                  <Icon name="play" size={12} />
                </button>
                <button
                  type="button"
                  class="icon-btn"
                  disabled={readOnly || runningCellId !== null}
                  onclick={() => void debugCell(idx)}
                  aria-label={`Debug cell ${idx + 1} (isolated)`}
                  title="Debug cell in an isolated Python process (uses selected interpreter)"
                >
                  <Icon name="bug" size={12} />
                </button>
              {:else}
                <Badge tone="neutral" variant="outline" label={cell.cell_type === 'markdown' ? 'Markdown' : 'Raw'} />
              {/if}
            </div>
            <div class="nb-cell-actions">
              <select
                class="input nb-type-sel"
                aria-label={`Cell ${idx + 1} type`}
                disabled={readOnly}
                value={cell.cell_type}
                onchange={(e) => commitDoc(changeNotebookCellType(doc, idx, (e.currentTarget as HTMLSelectElement).value as NotebookCellType))}
              >
                <option value="code">Code</option>
                <option value="markdown">Markdown</option>
                <option value="raw">Raw</option>
              </select>
              <button type="button" class="icon-btn" disabled={readOnly || idx === 0} onclick={() => commitDoc(moveNotebookCell(doc, idx, -1))} aria-label="Move cell up" title="Move cell up">
                <Icon name="chevronUp" size={12} />
              </button>
              <button type="button" class="icon-btn" disabled={readOnly || idx === doc.cells.length - 1} onclick={() => commitDoc(moveNotebookCell(doc, idx, 1))} aria-label="Move cell down" title="Move cell down">
                <Icon name="chevronDown" size={12} />
              </button>
              <button type="button" class="icon-btn" disabled={readOnly} onclick={() => commitDoc(addNotebookCell(doc, idx, 'code'))} aria-label="Insert code cell below" title="Insert code cell below">
                <Icon name="plus" size={12} />
              </button>
              <button type="button" class="icon-btn" disabled={readOnly || doc.cells.length <= 1} onclick={() => commitDoc(deleteNotebookCell(doc, idx))} aria-label="Delete cell" title="Delete cell">
                <Icon name="trash" size={12} />
              </button>
            </div>
          </div>

          <textarea
            class="nb-editor"
            class:is-md={cell.cell_type !== 'code'}
            dir={cell.cell_type === 'markdown' ? 'auto' : 'ltr'}
            rows={Math.max(2, Math.min(24, cell.source.split('\n').length + 1))}
            spellcheck={cell.cell_type === 'markdown'}
            readonly={readOnly}
            aria-label={`Cell ${idx + 1} source`}
            value={cell.source}
            oninput={(e) => commitDoc(updateNotebookCellSource(doc, idx, (e.currentTarget as HTMLTextAreaElement).value))}
            onkeydown={(e) => handleCellKeydown(e, idx)}
          ></textarea>

          {#if normOutputs.length > 0}
            <div class="nb-outputs" data-testid={`ide-nb-outputs-${idx}`}>
              {#each normOutputs as out, oi (oi)}
                {#if out.kind === 'stream'}
                  <pre class="nb-out-pre" class:is-stderr={out.streamName === 'stderr'}>{out.text}</pre>
                {:else if out.kind === 'error'}
                  <pre class="nb-out-pre is-error">{out.text}</pre>
                {:else if out.kind === 'image' || out.kind === 'svg'}
                  <div class="nb-out-img-wrap">
                    <img class="nb-out-img" src={out.dataUri} alt="Notebook cell figure output" />
                  </div>
                {:else if out.kind === 'html'}
                  <iframe
                    class="nb-out-html"
                    sandbox=""
                    title={`Cell ${idx + 1} HTML output`}
                    srcdoc={`<!doctype html><html><head><style>body{margin:6px;font-family:-apple-system,sans-serif;line-height:1.4;color:CanvasText;background:transparent}table{border-collapse:collapse;font-size:inherit}th,td{border:1px solid color-mix(in srgb,CanvasText 20%,transparent);padding:4px 8px;text-align:start}</style></head><body>${out.text}</body></html>`}
                  ></iframe>
                {:else}
                  <pre class="nb-out-pre">{out.text}</pre>
                {/if}
              {/each}
            </div>
          {/if}
        </section>
      {/each}

      <div class="nb-add-row">
        <button type="button" class="btn small ghost" disabled={readOnly} onclick={() => commitDoc(addNotebookCell(doc, doc.cells.length - 1, 'code'))}>
          <Icon name="plus" size={12} /> Code cell
        </button>
        <button type="button" class="btn small ghost" disabled={readOnly} onclick={() => commitDoc(addNotebookCell(doc, doc.cells.length - 1, 'markdown'))}>
          <Icon name="plus" size={12} /> Markdown cell
        </button>
      </div>
    </div>

    {#if varsOpen}
      <aside class="nb-vars" aria-label="Kernel variables" data-testid="ide-nb-vars-pane">
        <div class="nb-vars-head">
          <span class="nb-vars-title">Kernel Variables</span>
          <button type="button" class="icon-btn" onclick={() => (varsOpen = false)} aria-label="Close variables" title="Close variables">
            <Icon name="x" size={12} />
          </button>
        </div>
        {#if variables.length === 0}
          <p class="nb-vars-empty">Run a cell to inspect live Python variables, DataFrame shapes, and tensor sizes.</p>
        {:else}
          <div class="nb-vars-list">
            {#each variables as v (v.name)}
              <div class="nb-var-row">
                <div class="nb-var-top">
                  <code class="nb-var-name">{v.name}</code>
                  <span class="nb-var-type">{v.var_type}{v.shape ? ` (${v.shape})` : ''}</span>
                </div>
                <code class="nb-var-prev">{v.preview}</code>
              </div>
            {/each}
          </div>
        {/if}
      </aside>
    {/if}
  </div>
</div>

<style>
  .nb-root { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; background: var(--surface); }
  .nb-bar {
    display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;
    padding: 6px 10px; border-block-end: 1px solid var(--border); background: var(--surface-2);
  }
  .nb-bar-group { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .nb-bar-end { margin-inline-start: auto; }
  .nb-kernel-sel { max-inline-size: 260px; font-size: var(--fs-xs); font-family: var(--font-mono); }
  .nb-banner-err {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 8px 12px; background: var(--danger-soft); color: var(--danger); font-size: var(--fs-xs);
  }
  .nb-body { display: flex; flex: 1; min-block-size: 0; overflow: hidden; }
  .nb-cells { flex: 1; min-inline-size: 0; overflow-y: auto; padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .nb-cell {
    border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface-2);
    display: flex; flex-direction: column; overflow: hidden;
  }
  .nb-cell-head {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 4px 8px; border-block-end: 1px solid var(--border); background: var(--surface);
  }
  .nb-cell-meta, .nb-cell-actions { display: flex; align-items: center; gap: 4px; }
  .nb-exec-count { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); min-inline-size: 28px; }
  .nb-type-sel { font-size: var(--fs-xs); padding-block: 2px; }
  .nb-editor {
    inline-size: 100%; border: 0; resize: vertical; padding: 8px 10px; box-sizing: border-box;
    background: var(--surface); color: var(--text); font-family: var(--font-mono); font-size: var(--fs-s); line-height: 1.5;
  }
  .nb-editor.is-md { font-family: var(--font-ui); }
  .nb-editor:focus { outline: none; background: var(--surface-2); }
  .nb-outputs {
    display: flex; flex-direction: column; gap: 6px; padding: 8px 10px;
    border-block-start: 1px solid var(--border); background: var(--surface-2);
  }
  .nb-out-pre {
    margin: 0; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.45;
    color: var(--text); white-space: pre-wrap; word-break: break-word;
  }
  .nb-out-pre.is-stderr { color: var(--warning); }
  .nb-out-pre.is-error { color: var(--danger); background: var(--danger-soft); padding: 6px 8px; border-radius: var(--radius-s); }
  .nb-out-img-wrap { padding: 4px; background: var(--surface); border-radius: var(--radius-s); border: 1px solid var(--border); }
  .nb-out-img { max-inline-size: 100%; block-size: auto; display: block; }
  .nb-out-html { inline-size: 100%; min-block-size: 140px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); }
  .nb-add-row { display: flex; gap: 8px; justify-content: center; padding-block: 6px; }
  .nb-vars {
    inline-size: 280px; flex-shrink: 0; border-inline-start: 1px solid var(--border);
    background: var(--surface-2); display: flex; flex-direction: column; overflow: hidden;
  }
  .nb-vars-head {
    display: flex; align-items: center; justify-content: space-between; padding: 8px 10px;
    border-block-end: 1px solid var(--border);
  }
  .nb-vars-title { font-size: var(--fs-xs); font-weight: 600; color: var(--text); }
  .nb-vars-empty { margin: 0; padding: 12px 10px; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.5; }
  .nb-vars-list { flex: 1; overflow-y: auto; padding: 8px; display: flex; flex-direction: column; gap: 6px; }
  .nb-var-row {
    padding: 6px 8px; border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--surface);
    display: flex; flex-direction: column; gap: 4px;
  }
  .nb-var-top { display: flex; align-items: baseline; justify-content: space-between; gap: 6px; }
  .nb-var-name { font-family: var(--font-mono); font-size: var(--fs-xs); font-weight: 600; color: var(--accent-text); }
  .nb-var-type { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); }
  .nb-var-prev { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); word-break: break-all; }
  .is-on { background: var(--accent-soft); color: var(--accent-text); }
</style>
