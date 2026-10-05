<script lang="ts">
  // The chat's side panel (t3code's right-hand Diff panel): a file, a code
  // block or a turn's diff next to the conversation, with Preview | Source |
  // Diff tabs where they apply, Wrap, Reload, Open in Files, Copy path and a
  // full view (Modal). The host (ConversationView) owns placement: beside the
  // chat when the pane is wide, over it when narrow. Esc closes.
  import { getContext, untrack } from 'svelte';
  import { toastError } from '../../../lib/toastError';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { openFile } from '../../../lib/stores/openfile.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import PreviewBody, { forgetRead, type PreviewMode } from './PreviewBody.svelte';
  import { previewKind, previewKindForLang, requestKind, splitName } from './preview';
  import { relPath, resolvePath } from './format';
  import { onTabKey } from '../../../lib/tabKeys';
  import { CONV_CTX, type ConvContext, type PreviewReq } from './context';

  interface Props {
    req: PreviewReq;
    onclose: () => void;
  }
  let { req, onclose }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);
  const cwd = $derived(ctx.cwd ?? null);

  const path = $derived(req.kind === 'file' ? resolvePath(req.path, cwd) : req.kind === 'code' && req.file ? resolvePath(req.file, cwd) : null);
  const kind = $derived(requestKind(req));
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
    if (r.kind === 'code') return (r.file ? previewKind(r.file) : previewKindForLang(r.lang)) !== 'code' ? 'preview' : 'source';
    return previewKind(r.path) !== 'code' ? 'preview' : 'source';
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
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && !full) {
      e.stopPropagation();
      onclose();
    }
  }

  // Narrow pane: the host lays the panel OVER the chat. Then the covered chat
  // (and its composer) must not stay reachable by Tab / screen readers: mark it
  // inert, move focus into the panel, and hand focus back when it closes.
  let panelEl = $state<HTMLElement | null>(null);
  let covering = $state(false);
  $effect(() => {
    const slot = panelEl?.parentElement;
    const host = slot?.parentElement;
    if (!slot || !host || typeof ResizeObserver === 'undefined') return;
    const measure = (): void => {
      const pos = getComputedStyle(slot).position;
      covering = pos === 'absolute' || pos === 'fixed';
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(host);
    return () => ro.disconnect();
  });
  $effect(() => {
    if (!covering || !panelEl) return;
    const slot = panelEl.parentElement;
    const host = slot?.parentElement;
    if (!slot || !host) return;
    const covered = Array.from(host.children).filter((c): c is HTMLElement => c !== slot && c instanceof HTMLElement && !c.inert);
    const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    for (const el of covered) el.inert = true;
    untrack(() => panelEl?.querySelector<HTMLElement>('button, [href], [tabindex]:not([tabindex="-1"])')?.focus());
    return () => {
      for (const el of covered) el.inert = false;
      if (returnTo?.isConnected) returnTo.focus();
    };
  });
  const canFiles = $derived(!!path && !!ctx.sessionId && !ctx.readonly);
  const fullWidth = $derived(typeof window === 'undefined' ? 1100 : Math.max(480, Math.min(1400, window.innerWidth - 80)));
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside class="pv" bind:this={panelEl} aria-label="Preview: {title.base}" data-preview={req.kind} onkeydown={onKey}>
  <header class="pv-head">
    <span class="pv-icon" aria-hidden="true"><Icon name={req.kind === 'diff' ? 'split' : req.kind === 'code' ? 'format' : 'file'} size={13} /></span>
    <span class="pv-title" title={path ?? title.base}>
      <span class="pv-base">{title.base}</span>
      {#if title.dir}<span class="pv-dir mono">{title.dir}</span>{/if}
    </span>
    <button class="icon-btn" onclick={() => (full = true)} aria-label="Full view" title="Full view"><Icon name="maximize" size={13} /></button>
    <button class="icon-btn" onclick={onclose} aria-label="Close preview" title="Close preview" aria-keyshortcuts="Escape"><Icon name="x" size={13} /></button>
  </header>
  <div class="pv-bar">
    {#if modes.length > 1}
      <div class="pv-seg" role="tablist" aria-label="View">
        {#each modes as m (m)}
          <button role="tab" aria-selected={mode === m} tabindex={mode === m ? 0 : -1} class:on={mode === m} onclick={() => (mode = m)} onkeydown={onTabKey} data-mode={m}>{LABEL[m]}</button>
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
            <button role="tab" aria-selected={mode === m} tabindex={mode === m ? 0 : -1} class:on={mode === m} onclick={() => (mode = m)} onkeydown={onTabKey}>{LABEL[m]}</button>
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
    border-radius: var(--radius-s);
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
    outline: 2px solid var(--accent-text);
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
