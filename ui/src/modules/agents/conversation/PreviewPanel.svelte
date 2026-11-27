<script lang="ts">
  // The chat's Detail pane (t3code's right-hand Diff panel; design.md §3): a
  // file, a code block or a turn's diff next to the conversation. Its views
  // (Preview | Source | Diff, where they apply) ARE its header; Full view
  // (Modal) is its one action; Wrap, Reload, Copy path and Open in Files are
  // under ⋯. DetailPane places it: beside the chat while the chat keeps its
  // floor, else a right sheet (backdrop, focus trap, Esc, focus return).
  // Rendered at the end of ConversationView's `.conv-main` row.
  import { copyWithToast } from '../../../lib/clipboard';
  import { getContext, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import DetailPane from '../../../lib/components/side/DetailPane.svelte';
  import type { MenuItem } from '../../../lib/contextmenu.svelte';
  import { openFile } from '../../../lib/stores/openfile.svelte';
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
    await copyWithToast(path, 'path');
  }
  const canFiles = $derived(!!path && !!ctx.sessionId && !ctx.readonly);
  const fullWidth = $derived(typeof window === 'undefined' ? 1100 : Math.max(480, Math.min(1400, window.innerWidth - 80)));
  function menu(): MenuItem[] {
    return [
      { label: 'Wrap long lines', checked: wrap, action: () => (wrap = !wrap) },
      ...(req.kind === 'file' && req.content == null ? [{ label: 'Reload from disk', icon: 'refresh' as const, action: reload }] : []),
      ...(path ? [{ label: 'Copy path', icon: 'copy' as const, action: () => void copyPath() }] : []),
      ...(canFiles ? [{ label: 'Open in Files', icon: 'folder' as const, action: openInFiles }] : []),
    ];
  }
</script>

{#snippet modeTabs()}
  <div class="segmented pv-seg" role="tablist" aria-label="View">
    {#each modes as m (m)}
      <button role="tab" aria-selected={mode === m} tabindex={mode === m ? 0 : -1} class:on={mode === m} onclick={() => (mode = m)} onkeydown={onTabKey} data-mode={m}>{LABEL[m]}</button>
    {/each}
  </div>
{/snippet}

<DetailPane open module="agents" pane="preview" title={title.base} {onclose} {menu} views={modes.length > 1 ? modeTabs : undefined} legacy={{ w: 'otto_chat_panel_w' }} testid="chat-preview">
  {#snippet action()}
    <button type="button" class="icon-btn" onclick={() => (full = true)} aria-label="Full view" title="Full view"><Icon name="maximize" size={14} /></button>
  {/snippet}
  <div class="pv" data-preview={req.kind}>
    {#if modes.length > 1 || title.dir}
      <!-- The views took the header: the file's name and folder lead the body. -->
      <div class="pv-path" title={path ?? title.base}>
        <Icon name={req.kind === 'diff' ? 'split' : req.kind === 'code' ? 'format' : 'file'} size={13} />
        {#if modes.length > 1}<span class="pv-base">{title.base}</span>{/if}
        {#if title.dir}<span class="pv-dir mono">{title.dir}</span>{/if}
        {#if modes.length === 1}<span class="pv-only">{LABEL[modes[0]]}</span>{/if}
      </div>
    {/if}
    {#if req.kind === 'file' && req.content != null}
      <div class="pv-src-note">As written by the agent in this conversation — Reload shows the file on disk now.</div>
    {/if}
    <div class="pv-body">
      <PreviewBody {req} {mode} {wrap} {reloadKey} />
    </div>
  </div>
</DetailPane>

{#if full}
  <Modal title={title.dir ? `${title.base} — ${title.dir}` : title.base} width={fullWidth} onclose={() => (full = false)}>
    <div class="pv-full">
      {#if modes.length > 1}{@render modeTabs()}{/if}
      <div class="pv-full-body"><PreviewBody {req} {mode} {wrap} {reloadKey} /></div>
    </div>
  </Modal>
{/if}

<style>
  .pv {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .pv-path {
    flex: none;
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    padding: 6px 16px;
    border-block-end: 1px solid var(--separator);
    color: var(--text-dim);
    font-size: var(--fs-s);
    white-space: nowrap;
    overflow: hidden;
  }
  .pv-path :global(svg) {
    flex: none;
    align-self: center;
  }
  .pv-base {
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 0;
    max-width: 100%;
  }
  .pv-dir {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: start;
    min-width: 0;
  }
  .pv-only {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    font-weight: 600;
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
    font-size: var(--fs-s);
    padding: 2px 10px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .pv-seg button.on {
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    box-shadow: var(--shadow-card);
  }
  .pv-seg button:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .pv-src-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 4px 16px;
    border-bottom: 1px solid var(--separator);
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
