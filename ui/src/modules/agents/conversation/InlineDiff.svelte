<script lang="ts">
  // An edit's change, inline in the chat: one compact unified view per file —
  // line number, +/− gutter, syntax-coloured code, tinted added/removed rows —
  // capped at `maxLines` with "Show all". The git module's DiffViewer (file
  // list, search, unified/split toolbar) is built for reviewing a branch; a
  // chat edit is usually a few lines and wants none of that chrome.
  import { ensureHljs, highlightLine, langFromPath } from '../../../lib/hl';
  import type { DiffResp } from '../../../lib/api/types';

  interface Props {
    diff: DiffResp;
    /** Rows shown before "Show all N lines". */
    maxLines?: number;
  }
  let { diff, maxLines = 60 }: Props = $props();

  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });

  type Row =
    | { kind: 'file'; path: string }
    | { kind: 'gap'; text: string }
    | { kind: 'line'; origin: 'add' | 'del' | 'context'; no: number | null; html: string };

  const rows = $derived.by(() => {
    const out: Row[] = [];
    const multi = diff.files.length > 1;
    for (const f of diff.files) {
      const lang = hlReady ? langFromPath(f.path) : null;
      if (multi) out.push({ kind: 'file', path: f.path });
      f.hunks.forEach((h, i) => {
        const first = h.lines.find((l) => l.new_line != null || l.old_line != null);
        const at = first?.new_line ?? first?.old_line;
        if (i > 0 || (at != null && at > 1)) out.push({ kind: 'gap', text: at != null ? `line ${at}` : h.header });
        for (const l of h.lines) {
          out.push({
            kind: 'line',
            origin: l.origin === 'add' || l.origin === 'del' ? l.origin : 'context',
            no: l.origin === 'del' ? l.old_line : l.new_line,
            html: highlightLine(l.content || ' ', lang),
          });
        }
      });
    }
    return out;
  });
  const lineCount = $derived(rows.filter((r) => r.kind === 'line').length);
  let all = $state(false);
  const shown = $derived(all || rows.length <= maxLines ? rows : rows.slice(0, maxLines));
</script>

<div class="idiff diff-wrap" dir="ltr" data-lines={lineCount}>
  <div class="idiff-scroll"><div class="idiff-inner">
    {#each shown as r, i (i)}
      {#if r.kind === 'file'}
        <div class="idiff-file mono">{r.path}</div>
      {:else if r.kind === 'gap'}
        <div class="idiff-gap mono" aria-hidden="true">⋯ {r.text}</div>
      {:else}
        <div class="idiff-row dline {r.origin}">
          <span class="idiff-no mono">{r.no ?? ''}</span>
          <span class="idiff-sign mono" aria-hidden="true">{r.origin === 'add' ? '+' : r.origin === 'del' ? '−' : ''}</span>
          <span class="idiff-code mono hljs">{@html r.html}</span>
        </div>
      {/if}
    {/each}
  </div></div>
  {#if rows.length > maxLines}
    <button class="idiff-more" onclick={() => (all = !all)} aria-expanded={all}>
      {all ? 'Show less' : `Show all ${lineCount} lines`}
    </button>
  {/if}
</div>

<style>
  .idiff {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
    font-size: var(--fs-xs);
    text-align: start;
  }
  .idiff-scroll {
    max-height: 420px;
    overflow: auto;
    padding: 4px 0;
  }
  /* Rows share the widest line's width, so a tint spans the whole scroll. */
  .idiff-inner {
    width: max-content;
    min-width: 100%;
  }
  .idiff-row {
    display: grid;
    grid-template-columns: 3.5em 1.5em 1fr;
    line-height: 18px;
  }
  .idiff-no {
    color: var(--text-dim);
    text-align: end;
    padding-inline-end: 8px;
    user-select: none;
    opacity: 0.8;
  }
  .idiff-sign {
    text-align: center;
    user-select: none;
    color: var(--text-dim);
  }
  .idiff-code {
    white-space: pre;
    padding-inline-end: 12px;
    color: var(--text);
  }
  /* Added / removed must be unmistakable in a chat: tint + a coloured sign,
     from the semantic tones (theme-aware in every scheme). */
  .idiff-row.add {
    background: var(--success-soft);
  }
  .idiff-row.add .idiff-sign {
    color: var(--success);
    font-weight: 700;
  }
  .idiff-row.del {
    background: var(--danger-soft);
  }
  .idiff-row.del .idiff-sign {
    color: var(--danger);
    font-weight: 700;
  }
  .idiff-gap {
    color: var(--text-dim);
    padding: 2px 12px;
    font-size: var(--fs-xs);
    opacity: 0.85;
  }
  .idiff-file {
    padding: 4px 12px;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .idiff-more {
    display: block;
    width: 100%;
    background: none;
    border: 0;
    border-top: 1px solid var(--border);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 5px 10px;
    cursor: pointer;
  }
  .idiff-more:hover {
    color: var(--text);
    background: var(--hover);
  }
</style>
