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
  import { autoLang, ensureHljs, highlightBlock, highlightLine, langFromPath } from '../../../lib/hl';
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
  // Edit calls carry `structuredPatch` on the result; older records (and a
  // failed edit) do not — synthesize a −old/+new hunk from the input so the
  // change is still shown as a diff, never as two blobs of text.
  function editInputPatch(): string | null {
    const input = block.input;
    if (block.tool !== 'edit' || input == null || typeof input !== 'object') return null;
    const o = input as { old_string?: unknown; new_string?: unknown; file_path?: unknown };
    if (typeof o.old_string !== 'string' || typeof o.new_string !== 'string') return null;
    const oldL = o.old_string.split('\n');
    const newL = o.new_string.split('\n');
    const path = typeof o.file_path === 'string' ? o.file_path : 'file';
    return [
      `--- a/${path}`,
      `+++ b/${path}`,
      `@@ -1,${oldL.length} +1,${newL.length} @@`,
      ...oldL.map((l) => `-${l}`),
      ...newL.map((l) => `+${l}`),
    ].join('\n');
  }
  const diff = $derived.by(() => {
    if (block.result?.patch) return patchToDiff(block.result.patch, block.result.file_path);
    const synth = editInputPatch();
    return synth ? patchToDiff(synth, block.result?.file_path ?? null) : null;
  });
  const diffStats = $derived.by(() => {
    if (!diff) return null;
    let add = 0;
    let del = 0;
    for (const f of diff.files) {
      add += f.added ?? 0;
      del += f.deleted ?? 0;
    }
    return add || del ? { add, del } : null;
  });

  // Syntax colors: hljs loads lazily (stays out of the main bundle); until it
  // arrives, and for plain command output, text renders escaped. Language =
  // the file's extension for read/edit/write, else a bounded auto-detect.
  let hlReady = $state(false);
  $effect(() => {
    if (!open) return;
    void ensureHljs().then(() => (hlReady = true));
  });
  const lang = $derived.by(() => {
    if (!hlReady) return null;
    const byPath = filePath ? langFromPath(filePath) : null;
    if (byPath) return byPath;
    return autoLang(text);
  });

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
  const outHtml = $derived(open && !windowed && text ? highlightBlock(text, lang) : '');
  const inputHtml = $derived(open && showInput && inputJson ? highlightBlock(inputJson, hlReady ? 'json' : null) : '');

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
    {#if diffStats}
      <span class="step-stats mono" title="Lines added / removed"><span class="add">+{diffStats.add}</span> <span class="del">−{diffStats.del}</span></span>
    {/if}
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
          <pre class="out mono hljs" dir="ltr">{@html inputHtml}</pre>
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
          {#snippet row(line)}<div class="out-line mono hljs" dir="ltr">{@html highlightLine(line || ' ', lang)}</div>{/snippet}
        </VirtualList>
      {:else if text}
        <pre class="out mono hljs" class:err={status === 'err'} dir="ltr" data-lang={lang}>{@html outHtml}</pre>
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
  /* Added / removed lines must be unmistakable inside a chat: stronger tint
     than the git diff viewer's default plus a colored edge, theme-mixed. */
  .diff-wrap :global(tr.dline.add),
  .diff-wrap :global(.vrow.dline.add),
  .diff-wrap :global(.code.half.add) {
    background: color-mix(in srgb, var(--status-working, #3fb950) 22%, transparent);
    box-shadow: inset 3px 0 0 var(--status-working, #3fb950);
  }
  .diff-wrap :global(tr.dline.del),
  .diff-wrap :global(.vrow.dline.del),
  .diff-wrap :global(.code.half.del) {
    background: color-mix(in srgb, var(--status-exited, #e5534b) 20%, transparent);
    box-shadow: inset 3px 0 0 var(--status-exited, #e5534b);
  }
  .step-stats {
    font-size: 10.5px;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .step-stats .add {
    color: var(--status-working, #3fb950);
    font-weight: 600;
  }
  .step-stats .del {
    color: var(--status-exited, #e5534b);
    font-weight: 600;
  }
  /* hljs paints tokens only; the block keeps the chat's surface. */
  .out.hljs,
  .out-line.hljs {
    color: var(--text);
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
