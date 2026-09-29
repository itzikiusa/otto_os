<script lang="ts">
  // Read-only source view for the side panel, editor-style: a line-number
  // gutter, whole-file syntax colours (lib/hl `highlightLines`, lazy hljs),
  // the target line (`file.rs:88`) tinted and scrolled to, optional wrapping.
  // Past WINDOW_AT lines the rows go through the windowed list (uniform rows,
  // no wrapping) so a 10k-line file mounts a screenful, not 10k nodes.
  import { tick } from 'svelte';
  import VirtualList from '../../../lib/components/VirtualList.svelte';
  import { ensureHljs, highlightLines } from '../../../lib/hl';

  interface Props {
    text: string;
    lang: string | null;
    /** 1-based line to reveal. */
    line?: number | null;
    wrap?: boolean;
    /** Number of the first line (a Read of lines 80–84 starts at 80). */
    start?: number;
  }
  let { text, lang, line = null, wrap = false, start = 1 }: Props = $props();

  const WINDOW_AT = 3000;
  const ROW_H = 19;
  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });
  const lines = $derived(highlightLines(text.replace(/\n$/, ''), hlReady ? lang : null));
  const windowed = $derived(lines.length > WINDOW_AT);
  const plain = $derived(windowed ? text.split('\n') : []);
  const gut = $derived(`${String(lines.length + start - 1).length + 1}ch`);
  const rows = $derived(lines.map((html, i) => ({ html, n: i + start })));

  let rootEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const target = line;
    void lines.length;
    if (!target || windowed) return;
    void tick().then(() => rootEl?.querySelector(`[data-ln="${target}"]`)?.scrollIntoView({ block: 'center' }));
  });
</script>

{#if windowed}
  <VirtualList items={rows} estimateHeight={ROW_H} class="cv-vlist" scrollIndex={line ? line - start : -1} findText={(r: { html: string; n: number }) => plain[r.n - start] ?? ''}>
    {#snippet row(r)}
      <div class="cv-row" class:target={r.n === line} style:--gut={gut}>
        <span class="cv-no mono" aria-hidden="true">{r.n}</span><span class="cv-code mono hljs">{@html r.html || ' '}</span>
      </div>
    {/snippet}
  </VirtualList>
{:else}
  <div class="cv" class:wrap dir="ltr" bind:this={rootEl} style:--gut={gut} data-lang={lang}>
    <pre class="cv-pre"><code class="hljs">{#each rows as r (r.n)}<span class="cv-row" class:target={r.n === line} data-ln={r.n}><span class="cv-no mono" aria-hidden="true">{r.n}</span><span class="cv-code mono">{@html r.html}</span></span>{/each}</code></pre>
  </div>
{/if}

<style>
  .cv {
    min-width: 0;
    background: var(--code-bg, var(--surface-2));
  }
  .cv-pre {
    margin: 0;
    padding: 6px 0 10px;
    font-size: var(--fs-xs);
    line-height: 19px;
    overflow-x: auto;
    text-align: start;
  }
  .cv-pre code {
    display: block;
    min-width: max-content;
  }
  .cv.wrap .cv-pre code {
    min-width: 0;
  }
  .cv-row {
    display: grid;
    grid-template-columns: calc(var(--gut) + 12px) 1fr;
    min-height: 19px;
  }
  .cv-row:hover {
    background: var(--hover);
  }
  .cv-row.target {
    background: var(--accent-soft);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .cv-no {
    color: var(--text-dim);
    opacity: 0.65;
    text-align: end;
    padding-inline-end: 12px;
    user-select: none;
    -webkit-user-select: none;
  }
  .cv-code {
    white-space: pre;
    padding-inline-end: 16px;
    color: var(--text);
  }
  .cv.wrap .cv-code {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  :global(.cv-vlist) {
    height: 100%;
    overflow: auto;
    background: var(--code-bg, var(--surface-2));
    direction: ltr;
    font-size: var(--fs-xs);
  }
  :global(.cv-vlist) .cv-row {
    height: 19px;
    line-height: 19px;
    width: max-content;
    min-width: 100%;
  }
</style>
