<script lang="ts">
  // The chat's side panel (t3code's right-hand Diff panel): a file, a code
  // block or a turn's diff next to the conversation, with Preview | Source |
  // Diff tabs where they apply, Wrap, Reload, Open in Files, Copy path and a
  // full view (Modal). The host (ConversationView) owns placement: beside the
  // chat when the pane is wide, over it when narrow. Esc closes.
  import { getContext, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { openFile } from '../../../lib/stores/openfile.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import PreviewBody, { forgetRead, type PreviewMode } from './PreviewBody.svelte';
  import { previewKind, splitName } from './preview';
  import { relPath, resolvePath } from './format';
  import { CONV_CTX, type ConvContext, type PreviewReq } from './context';

  interface Props {
    req: PreviewReq;
    onclose: () => void;
  }
  let { req, onclose }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);
  const cwd = $derived(ctx.cwd ?? null);

  const path = $derived(req.kind === 'file' ? resolvePath(req.path, cwd) : req.kind === 'code' && req.file ? resolvePath(req.file, cwd) : null);
  const kind = $derived(path ? previewKind(path) : 'code');
  const modes = $derived.by(() => {
    const m: PreviewMode[] = [];
    if (req.kind === 'diff') return ['diff'] as PreviewMode[];
    if (kind !== 'code') m.push('preview');
    m.push('source');
    if (req.kind === 'file' && req.diff) m.push('diff');
    return m;
  });
  function initialMode(r: PreviewReq): PreviewMode {
    if (r.kind === 'diff') return 'diff';
    if (r.kind === 'file' && r.line) return 'source';
    const p = r.kind === 'file' ? r.path : r.file;
    return p && previewKind(p) !== 'code' ? 'preview' : 'source';
  }
  let mode = $state<PreviewMode>(untrack(() => initialMode(req)));
  let lastReq: PreviewReq | null = null;
  $effect(() => {
    // A new request resets the tab (the same request keeps what you picked).
    if (req !== lastReq) {
      lastReq = req;
      mode = initialMode(req);
    }
  });
  let wrap = $state(false);
  let reloadKey = $state(0);
  let full = $state(false);

  const title = $derived.by(() => {
    if (req.kind === 'diff') return { base: req.title, dir: '' };
    if (path) {
      const { dir, base } = splitName(relPath(path, cwd));
      return { base, dir };
    }
    return { base: req.kind === 'code' ? req.title : 'Preview', dir: '' };
  });
  const LABEL: Record<PreviewMode, string> = { preview: 'Preview', source: 'Source', diff: 'Diff' };

  function reload(): void {
    if (path) forgetRead(path);
    reloadKey++;
  }
  function openInFiles(): void {
    if (path) openFile.open(path, req.kind === 'file' ? (req.line ?? undefined) : undefined);
  }
  async function copyPath(): Promise<void> {
    if (!path) return;
    try {
      await navigator.clipboard.writeText(path);
      toasts.info('Path copied', path);
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && !full) {
      e.stopPropagation();
      onclose();
    }
  }
  const canFiles = $derived(!!path && !!ctx.sessionId && !ctx.readonly);
  const fullWidth = $derived(typeof window === 'undefined' ? 1100 : Math.max(480, Math.min(1400, window.innerWidth - 80)));
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside class="pv" aria-label="Preview: {title.base}" data-preview={req.kind} onkeydown={onKey}>
  <header class="pv-head">
    <span class="pv-icon" aria-hidden="true"><Icon name={req.kind === 'diff' ? 'split' : req.kind === 'code' ? 'format' : 'file'} size={13} /></span>
    <span class="pv-title" title={path ?? title.base}>
      <span class="pv-base">{title.base}</span>
      {#if title.dir}<span class="pv-dir mono">{title.dir}</span>{/if}
    </span>
    <button class="icon-btn" onclick={() => (full = true)} aria-label="Full view" title="Full view"><Icon name="maximize" size={13} /></button>
    <button class="icon-btn" onclick={onclose} aria-label="Close preview" title="Close (Esc)"><Icon name="x" size={13} /></button>
  </header>
  <div class="pv-bar">
    {#if modes.length > 1}
      <div class="pv-seg" role="tablist" aria-label="View">
        {#each modes as m (m)}
          <button role="tab" aria-selected={mode === m} class:on={mode === m} onclick={() => (mode = m)} data-mode={m}>{LABEL[m]}</button>
        {/each}
      </div>
    {:else}
      <span class="pv-only">{LABEL[modes[0]]}</span>
    {/if}
    <span class="pv-grow"></span>
    <button class="pv-tool" class:on={wrap} aria-pressed={wrap} onclick={() => (wrap = !wrap)} title="Wrap long lines">Wrap</button>
    {#if req.kind === 'file' && req.content == null}
      <button class="icon-btn" onclick={reload} aria-label="Reload from disk" title="Reload from disk"><Icon name="refresh" size={12} /></button>
    {/if}
    {#if path}
      <button class="icon-btn" onclick={() => void copyPath()} aria-label="Copy path" title="Copy path"><Icon name="copy" size={12} /></button>
    {/if}
    {#if canFiles}
      <button class="icon-btn" onclick={openInFiles} aria-label="Open in Files" title="Open in Files"><Icon name="folder" size={12} /></button>
    {/if}
  </div>
  {#if req.kind === 'file' && req.content != null}
    <div class="pv-src-note">As written by the agent in this conversation — Reload shows the file on disk now.</div>
  {/if}
  <div class="pv-body">
    <PreviewBody {req} {mode} {wrap} {reloadKey} />
  </div>
</aside>

{#if full}
  <Modal title={title.dir ? `${title.base} — ${title.dir}` : title.base} width={fullWidth} onclose={() => (full = false)}>
    <div class="pv-full">
      {#if modes.length > 1}
        <div class="pv-seg" role="tablist" aria-label="View">
          {#each modes as m (m)}
            <button role="tab" aria-selected={mode === m} class:on={mode === m} onclick={() => (mode = m)}>{LABEL[m]}</button>
          {/each}
        </div>
      {/if}
      <div class="pv-full-body"><PreviewBody {req} {mode} {wrap} {reloadKey} /></div>
    </div>
  </Modal>
{/if}

<style>
  .pv {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    background: var(--bg);
    border-inline-start: 1px solid var(--border);
  }
  .pv-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding-inline: 12px 6px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    min-width: 0;
  }
  .pv-icon {
    display: inline-flex;
    color: var(--text-dim);
  }
  .pv-title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 8px;
    overflow: hidden;
    white-space: nowrap;
    font-size: var(--fs-s);
  }
  .pv-base {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 0;
    max-width: 100%;
  }
  .pv-dir {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: start;
    min-width: 0;
  }
  .pv-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .pv-seg {
    display: inline-flex;
    padding: 2px;
    gap: 2px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .pv-seg button {
    border: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 3px 10px;
    border-radius: 4px;
    cursor: pointer;
  }
  .pv-seg button.on {
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    box-shadow: var(--shadow-card);
  }
  .pv-seg button:focus-visible,
  .pv-tool:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .pv-only {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding-inline: 4px;
  }
  .pv-grow {
    flex: 1;
  }
  .pv-tool {
    border: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 3px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .pv-tool:hover {
    background: var(--hover);
    color: var(--text);
  }
  .pv-tool.on {
    color: var(--accent-text);
  }
  .pv-src-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 5px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--info-soft);
  }
  .pv-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .pv-full {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .pv-full-body {
    min-height: 60vh;
    max-height: 72vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
</style>
