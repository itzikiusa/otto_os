<script lang="ts">
  // Interactive Jupyter Notebook (.ipynb) editor for the Otto IDE.
  // Runs cells on a real Jupyter kernel (any kernelspec: ipykernel, R, Bash, …)
  // with live streamed outputs, input(), interrupt and restart, or on the
  // built-in Python kernel (zero-install fallback for an env without ipykernel).
  // Rich outputs (streams, ANSI tracebacks, HTML in a script-less sandboxed
  // iframe, PNG/SVG images), the kernel picker + status chip, the Variables
  // pane and Debug Cell.
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../../../lib/api/client';
  import type {
    IdeMlAcceleratorInfo,
    IdeNotebookExecuteResp,
    IdeNotebookKernelInfo,
    IdeNotebookKernelStatus,
    IdeNotebookKernelsResp,
    IdeNotebookStreamEvent,
    IdeNotebookVariable,
    IdeNotebookVariablesResp,
  } from '../../../lib/api/types';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { ide } from '../ideStore.svelte';
  import { streamIdeSse } from '../store/ideTypes';
  import {
    addNotebookCell,
    applyNotebookStreamOutput,
    changeNotebookCellType,
    clearNotebookOutputs,
    deleteNotebookCell,
    moveNotebookCell,
    normalizeNotebookOutputs,
    parseIpynb,
    pickNotebookKernel,
    serializeIpynb,
    updateNotebookCellSource,
    withKernelspecMetadata,
    type NotebookCellType,
    type NotebookDoc,
    type NotebookOutput,
  } from './ideNotebookModel';
  import IdeNotebookOutputs from './IdeNotebookOutputs.svelte';
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
  let selectedKernelId = $state('');
  let kernelsLoading = $state(false);
  let kernelStatus = $state<IdeNotebookKernelStatus | null>(null);
  let kernelError = $state<string | null>(null);
  let runningCellId = $state<string | null>(null);
  let runningAll = $state(false);
  let liveRunId = $state<string | null>(null);
  let liveOutputs = $state<NotebookOutput[]>([]);
  let inputReq = $state<{ prompt: string; password: boolean } | null>(null);
  let inputValue = $state('');
  let variables = $state<IdeNotebookVariable[]>([]);
  let varsOpen = $state(false);
  let activeSessionId = $state<string | null>(null);
  const trustedKernels = new Set<string>();
  let disposed = false;
  let viewToken = 0;
  let streamCtl: AbortController | null = null;

  const selected = $derived(kernels.find((k) => k.id === selectedKernelId));
  const isJupyter = $derived(selected?.kind === 'jupyter');
  const kernelDebugger = $derived(
    isJupyter && (kernelStatus?.kernel_id === selectedKernelId ? Boolean(kernelStatus?.debugger) : Boolean(selected?.debugger)),
  );
  const chipState = $derived(kernelStatus && kernelStatus.kernel_id === selectedKernelId ? kernelStatus.state : null);
  const chipTone = $derived(
    chipState === 'dead' ? 'bad' : chipState === 'busy' ? 'warn' : chipState === 'idle' ? 'ok' : 'neutral',
  );

  onDestroy(() => {
    disposed = true;
    viewToken += 1;
    runningAll = false;
    streamCtl?.abort();
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

  // Reset only when the notebook itself changes: the props object is rebuilt on
  // every buffer edit (each committed output), which must not drop a running cell.
  let effectPath = '';
  $effect(() => {
    const p = path;
    untrack(() => {
      if (p === effectPath) return;
      effectPath = p;
      viewToken += 1;
      runningCellId = null;
      runningAll = false;
      liveRunId = null;
      inputReq = null;
      variables = [];
      activeSessionId = null;
      kernelStatus = null;
      kernelError = null;
      selectedKernelId = '';
      if (ide.hasTarget && p) {
        void refreshKernels(p);
        subscribe(p);
      }
    });
  });

  /** Live kernel events: status chip, streamed outputs, input() prompts. */
  function subscribe(targetPath: string): void {
    streamCtl?.abort();
    const ctl = new AbortController();
    streamCtl = ctl;
    const q = new URLSearchParams({ root: ide.target.root, notebook_path: targetPath });
    void streamIdeSse(`/ide/notebooks/stream?${q}`, {}, ctl.signal, (raw) => {
      if (ctl.signal.aborted || path !== targetPath) return;
      let ev: IdeNotebookStreamEvent;
      try {
        ev = JSON.parse(raw) as IdeNotebookStreamEvent;
      } catch {
        return;
      }
      if (ev.kind === 'status') {
        kernelStatus = ev.state === 'none' ? null : (ev as IdeNotebookKernelStatus);
      } else if (ev.kind === 'output' || ev.kind === 'clear_output') {
        if (ev.run_id === liveRunId) liveOutputs = applyNotebookStreamOutput(liveOutputs, ev);
      } else if (ev.kind === 'input_request') {
        if (ev.run_id === liveRunId) {
          inputReq = { prompt: ev.prompt, password: ev.password };
          inputValue = '';
        }
      } else if (ev.kind === 'input_done') {
        inputReq = null;
      }
    }).catch(() => {
      /* the stream is best-effort: the final response still carries every output */
    });
  }

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
      kernelStatus = resp.kernel_status ?? kernelStatus;
      if (!selectedKernelId || !kernels.some((k) => k.id === selectedKernelId)) {
        selectedKernelId = resp.kernel_status?.kernel_id ?? pickNotebookKernel(kernels, doc.metadata);
      }
    } catch {
      /* kernels optional while offline or SSH */
    } finally {
      if (path === targetPath) kernelsLoading = false;
    }
  }

  function chooseKernel(id: string): void {
    selectedKernelId = id;
    kernelError = null;
    const k = kernels.find((x) => x.id === id);
    if (k && !readOnly) commitDoc(withKernelspecMetadata(doc, k));
  }

  /** A kernelspec stored in the project could run anything: ask once before its first start. */
  async function ensureTrusted(): Promise<boolean> {
    const k = selected;
    if (!k?.needs_trust || trustedKernels.has(k.id)) return true;
    const ok = await confirmer.ask(`Start kernel '${k.display_name}' from this project? It runs: ${k.argv_preview ?? k.interpreter}`, {
      title: 'Start project kernel?',
      confirmLabel: 'Start kernel',
    });
    if (ok) trustedKernels.add(k.id);
    return ok;
  }

  function kernelBody(origPath: string): Record<string, unknown> {
    return {
      target: ide.target,
      notebook_path: origPath,
      kernel_id: selectedKernelId || undefined,
      trust_kernel: trustedKernels.has(selectedKernelId),
    };
  }

  function errorLine(e: unknown): string {
    return loadErrorText(e).replace(/^(invalid|upstream|conflict|forbidden|not found):\s*/i, '');
  }

  async function runCellById(cellId: string): Promise<boolean> {
    kernelError = null;
    return executeNotebookCellGuarded(cellId, {
      getDoc: () => doc,
      getPath: () => path,
      isDisposed: () => disposed,
      getViewToken: () => viewToken,
      getRunningCellId: () => runningCellId,
      setRunningCellId: (id) => { runningCellId = id; },
      setRunId: (id) => {
        liveRunId = id;
        liveOutputs = [];
        inputReq = null;
      },
      setSessionAndVars: (sid, vars) => {
        activeSessionId = sid;
        variables = vars;
      },
      commitDoc,
      onError: (e) => { kernelError = errorLine(e); },
      executeRequest: (origPath, code, runId) =>
        api.post<IdeNotebookExecuteResp>('/ide/notebooks/execute', {
          ...kernelBody(origPath),
          code,
          run_id: runId,
        }),
    });
  }

  async function runCell(index: number): Promise<boolean> {
    const cell = doc.cells[index];
    if (!cell || !(await ensureTrusted())) return false;
    return runCellById(cell.id);
  }

  async function runAllCells(): Promise<void> {
    if (runningAll || runningCellId !== null || !(await ensureTrusted())) return;
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
    try {
      const resp = await api.post<{ ok: boolean; message?: string }>('/ide/notebooks/interrupt', {
        target: ide.target,
        notebook_path: path,
      });
      if (resp.ok) toasts.info('Interrupt sent to the kernel');
      else kernelError = resp.message ?? 'The kernel didn’t stop. Restart it to stop the cell.';
    } catch (e) {
      toastError('Couldn’t interrupt kernel', e);
    }
  }

  async function restartKernel(thenRunAll = false): Promise<void> {
    runningAll = false;
    const origPath = path;
    if (!(await ensureTrusted())) return;
    kernelError = null;
    try {
      await api.post('/ide/notebooks/restart', kernelBody(origPath));
      if (path !== origPath) return;
      variables = [];
      toasts.success('Kernel restarted');
      await refreshKernels(origPath);
      if (thenRunAll) await runAllCells();
    } catch (e) {
      kernelError = errorLine(e);
    }
  }

  async function sendInput(): Promise<void> {
    const value = inputValue;
    inputReq = null;
    try {
      await api.post('/ide/notebooks/input', { target: ide.target, notebook_path: path, value });
    } catch (e) {
      toastError('Couldn’t send input', e);
    }
  }

  async function installIpykernel(): Promise<void> {
    const k = selected;
    if (!k || k.kind !== 'builtin') return;
    const envDir = k.interpreter.replace(/\/bin\/python[0-9.]*$/, '');
    const envName = envDir.split('/').pop() || envDir;
    const ok = await confirmer.ask(`Install ipykernel into ${envName} with pip? This downloads packages from PyPI.`, {
      title: 'Install ipykernel?',
      confirmLabel: 'Install',
    });
    if (!ok) return;
    const uv = k.env_kind === 'uv';
    await ide.startRun({
      adhoc: {
        name: `Install ipykernel (${envName})`,
        runner: 'custom',
        command: uv ? 'uv' : k.interpreter,
        args: uv ? ['pip', 'install', '--python', k.interpreter, 'ipykernel'] : ['-m', 'pip', 'install', 'ipykernel'],
        cwd: ide.target.root,
        env_overrides: {},
      },
    });
  }

  function openMore(e: MouseEvent): void {
    const busy = runningCellId !== null || runningAll;
    const items: MenuItem[] = [
      { label: 'Restart & Run All', icon: 'refresh', disabled: readOnly || busy, action: () => void restartKernel(true) },
    ];
    if (selected?.kind === 'builtin') {
      items.push({ label: 'Install ipykernel…', icon: 'download', action: () => void installIpykernel() });
    }
    items.push(
      { separator: true },
      { label: 'Clear all outputs', icon: 'trash', disabled: readOnly, action: () => commitDoc(clearNotebookOutputs(doc)) },
      { label: 'View raw .ipynb JSON', icon: 'code', action: switchToRawText },
    );
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
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
    if (!cell || cell.cell_type !== 'code' || isJupyter) return;
    const origPath = path;
    const saved = await ide.saveFile(origPath);
    if (!saved || path !== origPath) return;
    await ide.startDebug({
      language: 'python',
      launch: {
        program: origPath,
        args: ['--cell', String(index)],
        cwd: ide.target.root,
        sdk_path: selected?.interpreter || undefined,
      },
    });
  }

  function debugTitle(): string {
    if (!isJupyter) return 'Debug cell in an isolated Python process (uses selected interpreter)';
    return kernelDebugger ? 'Debugging Jupyter kernels isn’t available yet' : 'This kernel doesn’t support debugging';
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
        title="Interrupt the running cell"
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
        title="Restart kernel (outputs stay)"
        aria-label="Restart kernel"
      >
        <Icon name="refresh" size={12} />
        Restart
      </button>
      {#if !isJupyter || kernelDebugger}
        <button
          type="button"
          class="btn small ghost"
          class:is-on={varsOpen}
          onclick={() => void toggleVariables()}
          data-testid="ide-nb-vars-toggle"
        >
          Variables ({variables.length})
        </button>
      {/if}
    </div>

    <div class="nb-bar-group nb-bar-end">
      {#if accelerators.length > 0}
        <Badge tone="accent" variant="outline" label={formatAccelBadge(accelerators[0])} />
      {/if}
      <select
        class="input nb-kernel-sel"
        aria-label="Notebook kernel"
        title={selected?.argv_preview ?? selected?.interpreter ?? 'Notebook kernel'}
        disabled={kernelsLoading || runningCellId !== null}
        value={selectedKernelId}
        onchange={(e) => chooseKernel((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-nb-kernel-select"
      >
        {#if kernels.length === 0}
          <option value="">{kernelsLoading ? 'Finding kernels…' : 'No kernel found'}</option>
        {:else}
          {#if kernels.some((k) => k.kind === 'jupyter')}
            <optgroup label="Jupyter kernels">
              {#each kernels.filter((k) => k.kind === 'jupyter') as k (k.id)}
                <option value={k.id}>{k.display_name}</option>
              {/each}
            </optgroup>
          {/if}
          {#if kernels.some((k) => k.kind === 'builtin')}
            <optgroup label="Built-in">
              {#each kernels.filter((k) => k.kind === 'builtin') as k (k.id)}
                <option value={k.id}>{k.display_name}</option>
              {/each}
            </optgroup>
          {/if}
        {/if}
      </select>
      {#if isJupyter && chipState}
        <Badge tone={chipTone} variant="outline" dot label={chipState} testid="ide-nb-kernel-status" title={chipState === 'dead' && kernelStatus?.exit_code != null ? `Exit code ${kernelStatus.exit_code}` : `Kernel ${chipState}`} />
      {/if}
      <button
        type="button"
        class="icon-btn"
        onclick={openMore}
        aria-label="More notebook actions"
        title="More notebook actions"
        data-testid="ide-nb-more"
      >
        <Icon name="more" size={14} />
      </button>
    </div>
  </header>

  {#if kernels.length === 0 && !kernelsLoading && ide.hasTarget}
    <p class="nb-note">No kernel found. Create a Python environment or install a Jupyter kernel.</p>
  {:else if selected?.kind === 'builtin'}
    <p class="nb-note">IPython magics and debugging need ipykernel. Install it from ⋯.</p>
  {/if}

  <div class="nb-kernel-err" data-testid="ide-nb-kernel-error">
    <LoadState what="the kernel" errorTitle="The kernel couldn’t run the cell" error={kernelError} empty variant="compact" onretry={() => void restartKernel()} />
  </div>

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
        {@const showLive = isRunning && liveRunId !== null}
        {@const normOutputs = normalizeNotebookOutputs(showLive ? liveOutputs : cell.outputs)}
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
                  disabled={readOnly || runningCellId !== null || isJupyter}
                  onclick={() => void debugCell(idx)}
                  aria-label={`Debug cell ${idx + 1}`}
                  title={debugTitle()}
                  data-testid={`ide-nb-debug-${idx}`}
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

          {#if normOutputs.length > 0 || (isRunning && inputReq)}
            <IdeNotebookOutputs
              outputs={normOutputs}
              cellIndex={idx}
              cellId={cell.id}
              input={isRunning ? inputReq : null}
              bind:inputValue
              onsubmitinput={() => void sendInput()}
            />
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
  .nb-kernel-sel { max-inline-size: 280px; min-inline-size: 0; font-size: var(--fs-xs); font-family: var(--font-mono); }
  .nb-note { margin: 0; padding: 4px 12px; font-size: var(--fs-xs); color: var(--text-dim); border-block-end: 1px solid var(--border); }
  .nb-kernel-err:empty { display: none; }
  .nb-kernel-err { padding-inline: 10px; }
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
