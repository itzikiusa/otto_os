<script lang="ts">
  // An edit's change, inline in the chat: one compact unified view per file —
  // line number, +/− gutter, syntax-coloured code, tinted added/removed rows —
  // capped at `maxLines` with "Show all". The git module's DiffViewer (file
  // list, search, unified/split toolbar) is built for reviewing a branch; a
  // chat edit is usually a few lines and wants none of that chrome.
  // `full` is the side panel's variant (t3code's Diff panel): every file under
  // a sticky header with its +/− and a fold, old AND new line numbers, no cap,
  // optional wrapping, and a `focus` file scrolled into view.
  import { tick } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ensureHljs, highlightLine, langFromPath } from '../../../lib/hl';
  import type { DiffResp } from '../../../lib/api/types';

  interface Props {
    diff: DiffResp;
    /** Rows shown before "Show all N lines" (compact only). */
    maxLines?: number;
    full?: boolean;
    wrap?: boolean;
    /** Full: the file to scroll to. */
    focus?: string | null;
    /** Full: shown relative to this folder. */
    cwd?: string | null;
  }
  let { diff, maxLines = 60, full = false, wrap = false, focus = null, cwd = null }: Props = $props();

  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });

  type Line = { origin: 'add' | 'del' | 'context'; old: number | null; new: number | null; html: string };
  type Row = { kind: 'file'; path: string } | { kind: 'gap'; text: string } | ({ kind: 'line' } & Line);

  const rows = $derived.by(() => {
    if (full) return [] as Row[];
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
            old: l.old_line,
            new: l.new_line,
            html: highlightLine(l.content || ' ', lang),
          });
        }
      });
    }
    return out;
  });
  const lineCount = $derived(full ? diff.files.reduce((n, f) => n + f.hunks.reduce((m, h) => m + h.lines.length, 0), 0) : rows.filter((r) => r.kind === 'line').length);
  let all = $state(false);
  const shown = $derived(all || rows.length <= maxLines ? rows : rows.slice(0, maxLines));

  // ── full (panel) ──────────────────────────────────────────────────────────
  type FileView = { path: string; add: number; del: number; hunks: { header: string; at: number | null; lines: Line[] }[] };
  const files = $derived.by(() => {
    if (!full) return [] as FileView[];
    return diff.files.map((f) => {
      const lang = hlReady ? langFromPath(f.path) : null;
      let add = 0;
      let del = 0;
      const hunks = f.hunks.map((h) => {
        const first = h.lines.find((l) => l.new_line != null || l.old_line != null);
        return {
          header: h.header,
          at: first?.new_line ?? first?.old_line ?? null,
          lines: h.lines.map((l): Line => {
            if (l.origin === 'add') add++;
            else if (l.origin === 'del') del++;
            return {
              origin: l.origin === 'add' || l.origin === 'del' ? l.origin : 'context',
              old: l.old_line,
              new: l.new_line,
              html: highlightLine(l.content || ' ', lang),
            };
          }),
        };
      });
      return { path: f.path, add: f.added ?? add, del: f.deleted ?? del, hunks };
    });
  });
  let folded = $state<Record<string, boolean>>({});
  const rel = (p: string): string => {
    if (!cwd) return p;
    const root = cwd.replace(/\/$/, '') + '/';
    return p.startsWith(root) ? p.slice(root.length) : p;
  };
  let rootEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const f = focus;
    if (!full || !f) return;
    void tick().then(() => rootEl?.querySelector(`[data-diff-file="${CSS.escape(f)}"]`)?.scrollIntoView({ block: 'start' }));
  });
</script>

{#if full}
  <div class="pdiff" class:wrap dir="ltr" data-lines={lineCount} bind:this={rootEl}>
    {#each files as f (f.path)}
      <section class="pdiff-file" data-diff-file={f.path} class:focus={f.path === focus}>
        <button class="pdiff-head" onclick={() => (folded[f.path] = !folded[f.path])} aria-expanded={!folded[f.path]}>
          <Icon name={folded[f.path] ? 'chevronRight' : 'chevronDown'} size={12} />
          <span class="pdiff-path mono" title={f.path}>{rel(f.path)}</span>
          <span class="pdiff-stat mono"><span class="add">+{f.add}</span> <span class="del">−{f.del}</span></span>
        </button>
        {#if !folded[f.path]}
          <div class="pdiff-body">
            {#each f.hunks as h, hi (hi)}
              {#if hi > 0 || (h.at != null && h.at > 1)}
                <div class="pdiff-gap mono" aria-hidden="true">⋯ {h.at != null ? `line ${h.at}` : h.header}</div>
              {/if}
              {#each h.lines as l, li (li)}
                <div class="pdiff-row dline {l.origin}">
                  <span class="pdiff-no mono">{l.old ?? ''}</span>
                  <span class="pdiff-no mono">{l.new ?? ''}</span>
                  <span class="pdiff-sign mono" aria-hidden="true">{l.origin === 'add' ? '+' : l.origin === 'del' ? '−' : ''}</span>
                  <span class="pdiff-code mono hljs">{@html l.html}</span>
                </div>
              {/each}
            {/each}
          </div>
        {/if}
      </section>
    {/each}
  </div>
{:else}
  <div class="idiff diff-wrap" dir="ltr" data-lines={lineCount}>
    <div class="idiff-scroll"><div class="idiff-inner">
      {#each shown as r, i (i)}
        {#if r.kind === 'file'}
          <div class="idiff-file mono">{r.path}</div>
        {:else if r.kind === 'gap'}
          <div class="idiff-gap mono" aria-hidden="true">⋯ {r.text}</div>
        {:else}
          <div class="idiff-row dline {r.origin}">
            <span class="idiff-no mono">{(r.origin === 'del' ? r.old : r.new) ?? ''}</span>
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
{/if}

<style>
  .idiff {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--code-bg, var(--surface-2));
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
  .idiff-no,
  .pdiff-no {
    color: var(--text-dim);
    text-align: end;
    padding-inline-end: 8px;
    user-select: none;
    opacity: 0.8;
  }
  .idiff-sign,
  .pdiff-sign {
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
  .idiff-row.add,
  .pdiff-row.add {
    background: var(--success-soft);
  }
  .idiff-row.add .idiff-sign,
  .pdiff-row.add .pdiff-sign {
    color: var(--success);
    font-weight: 700;
  }
  .idiff-row.del,
  .pdiff-row.del {
    background: var(--danger-soft);
  }
  .idiff-row.del .idiff-sign,
  .pdiff-row.del .pdiff-sign {
    color: var(--danger);
    font-weight: 700;
  }
  /* The changed line's edge, like an editor gutter mark. */
  .pdiff-row.add {
    box-shadow: inset 3px 0 0 color-mix(in srgb, var(--success) 70%, transparent);
  }
  .pdiff-row.del {
    box-shadow: inset 3px 0 0 color-mix(in srgb, var(--danger) 70%, transparent);
  }
  .idiff-gap,
  .pdiff-gap {
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
  /* ── full ── */
  .pdiff {
    font-size: var(--fs-xs);
    text-align: start;
    min-width: 0;
  }
  .pdiff-file {
    border-bottom: 1px solid var(--border);
  }
  .pdiff-head {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 12px;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    text-align: start;
  }
  .pdiff-head:hover {
    background: var(--surface-2);
  }
  .pdiff-head:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .pdiff-file.focus .pdiff-head {
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .pdiff-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pdiff-stat {
    flex-shrink: 0;
  }
  .add {
    color: var(--success);
    font-weight: 600;
  }
  .del {
    color: var(--danger);
    font-weight: 600;
  }
  .pdiff-body {
    overflow-x: auto;
    padding: 4px 0;
    background: var(--code-bg, var(--surface-2));
  }
  .pdiff-row {
    display: grid;
    grid-template-columns: 3.6em 3.6em 1.6em 1fr;
    line-height: 19px;
    width: max-content;
    min-width: 100%;
  }
  .pdiff-code {
    white-space: pre;
    padding-inline-end: 16px;
    color: var(--text);
  }
  .pdiff.wrap .pdiff-row {
    width: auto;
  }
  .pdiff.wrap .pdiff-code {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
