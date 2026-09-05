<script lang="ts">
  // One tool call row inside a "Worked for …" group: kind icon, title, status
  // dot; expand → capped output <pre> (windowed when long) / diff via
  // git/DiffViewer / file chip (opens the Files panel) / result images.
  import { getContext } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import VirtualList from '../../../lib/components/VirtualList.svelte';
  import DiffViewer from '../../git/DiffViewer.svelte';
  import ImageBlock from './ImageBlock.svelte';
  import Markdown from './Markdown.svelte';
  import { openFile } from '../../../lib/stores/openfile.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { TOOL_CHROME, fmtBytes, patchToDiff, toolSubtitle } from './format';
  import type { Block } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    block: Extract<Block, { kind: 'tool_call' }>;
  }
  let { block }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);

  let open = $state(false);
  const chrome = $derived(TOOL_CHROME[block.tool] ?? TOOL_CHROME.other);
  const subtitle = $derived(toolSubtitle(block));
  const title = $derived(block.title?.trim() || block.name);
  const status = $derived<'ok' | 'err' | 'pending'>(
    block.result == null ? 'pending' : block.result.ok ? 'ok' : 'err',
  );
  const diff = $derived(block.result?.patch ? patchToDiff(block.result.patch, block.result.file_path) : null);
  const filePath = $derived(block.result?.file_path ?? (block.tool === 'read' || block.tool === 'edit' || block.tool === 'write' ? subtitle || null : null));
  const text = $derived(block.result?.text ?? '');
  const lines = $derived(text ? text.split('\n') : []);
  // Long outputs render through the windowed list (uniform mono rows); short
  // ones as a plain <pre> so selection/copy stays natural.
  const windowed = $derived(lines.length > 400);
  const inputJson = $derived.by(() => {
    if (block.input == null) return '';
    try {
      return typeof block.input === 'string' ? block.input : JSON.stringify(block.input, null, 2);
    } catch {
      return String(block.input);
    }
  });
  let showInput = $state(false);

  function openInFiles(): void {
    if (!filePath) return;
    if (ctx.sessionId && !ctx.readonly) openFile.open(filePath);
    else {
      void navigator.clipboard?.writeText(filePath);
      toasts.info('Path copied', filePath);
    }
  }
</script>

<div class="step" class:open data-tool={block.tool} data-status={status}>
  <button class="step-row" onclick={() => (open = !open)} aria-expanded={open} title={subtitle || title}>
    <span class="step-icon"><Icon name={chrome.icon} size={13} /></span>
    <span class="step-title">
      <span class="step-label">{chrome.label}</span>
      <span class="step-name">{title}</span>
      {#if subtitle && subtitle !== title}<span class="step-sub mono">{subtitle}</span>{/if}
    </span>
    {#if block.result?.truncated}
      <span class="chip step-trunc" title="Output capped at 64 KB">{fmtBytes(block.result.bytes)}</span>
    {/if}
    <span class="step-dot {status}" title={status === 'ok' ? 'Succeeded' : status === 'err' ? 'Failed' : 'Running…'}></span>
    <span class="step-caret">{open ? '▾' : '▸'}</span>
  </button>
  {#if open}
    <div class="step-body">
      {#if filePath}
        <button class="file-chip mono" onclick={openInFiles} title={ctx.sessionId && !ctx.readonly ? 'Open in Files' : 'Copy path'}>
          <Icon name="file" size={11} /> {filePath}
        </button>
      {/if}
      {#if inputJson}
        <button class="link-btn" onclick={() => (showInput = !showInput)}>{showInput ? 'Hide input' : 'Show input'}</button>
        {#if showInput}
          <pre class="out mono" dir="ltr">{inputJson}</pre>
        {/if}
      {/if}
      {#if diff && diff.files.some((f) => f.hunks.length)}
        <div class="diff-wrap">
          <DiffViewer {diff} />
        </div>
      {:else if block.result == null}
        <div class="pending">Waiting for the result…</div>
      {:else if block.tool === 'web' || block.tool === 'ask'}
        <Markdown md={text} small />
      {:else if windowed}
        <VirtualList items={lines} estimateHeight={18} class="out-vlist">
          {#snippet row(line)}<div class="out-line mono" dir="ltr">{line || ' '}</div>{/snippet}
        </VirtualList>
      {:else if text}
        <pre class="out mono" class:err={status === 'err'} dir="ltr">{text}</pre>
      {:else if !block.result.image_ids.length}
        <div class="pending dim">(no output)</div>
      {/if}
      {#if block.result?.truncated}
        <div class="dim trunc-note">Output truncated to 64 KB ({fmtBytes(block.result.bytes)} total).</div>
      {/if}
      {#if block.result?.image_ids.length}
        <div class="imgs">
          {#each block.result.image_ids as id (id)}
            <ImageBlock {id} alt="Tool result image" small />
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .step {
    border-top: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .step:first-child {
    border-top: 0;
  }
  .step-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 10px;
    background: none;
    border: 0;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font: inherit;
    min-width: 0;
  }
  .step-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .step-icon {
    color: var(--text-dim);
    display: inline-flex;
    flex-shrink: 0;
  }
  .step-title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 12.5px;
    overflow: hidden;
    white-space: nowrap;
  }
  .step-label {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .step-name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 1;
    min-width: 0;
  }
  .step-sub {
    color: var(--text-dim);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex-shrink: 4;
    direction: ltr;
    unicode-bidi: isolate;
  }
  .step-trunc {
    height: 16px;
    font-size: 10px;
  }
  .step-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .step-dot.ok {
    background: var(--status-working, #3fb950);
  }
  .step-dot.err {
    background: var(--status-exited, #e5534b);
  }
  .step-dot.pending {
    background: var(--status-warn, #febc2e);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .step-caret {
    color: var(--text-dim);
    font-size: 11px;
    flex-shrink: 0;
  }
  .step-body {
    padding: 0 10px 10px 31px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .out {
    margin: 0;
    max-height: 420px;
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    font-size: 11.5px;
    line-height: 18px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    text-align: start;
  }
  .out.err {
    border-color: color-mix(in srgb, var(--status-exited, #e5534b) 45%, transparent);
  }
  :global(.out-vlist) {
    max-height: 420px;
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 4px 0;
    direction: ltr;
  }
  .out-line {
    height: 18px;
    line-height: 18px;
    padding: 0 10px;
    font-size: 11.5px;
    white-space: pre;
    /* Long lines widen the row so the windowed list scrolls horizontally
       (AGENTS.md: wide content scrolls inside its own container). */
    width: max-content;
    min-width: 100%;
    box-sizing: border-box;
  }
  .diff-wrap {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    max-height: 520px;
    overflow-y: auto;
  }
  .file-chip {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 99px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .file-chip:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .link-btn {
    align-self: flex-start;
    background: none;
    border: 0;
    padding: 0;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    text-decoration: underline dotted;
  }
  .pending {
    font-size: 12px;
    color: var(--text-dim);
  }
  .trunc-note {
    font-size: 11px;
  }
  .imgs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
