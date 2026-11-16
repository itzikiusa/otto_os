<script lang="ts">
  // Individual virtualized row renderer (unified, split, hunk header, file header,
  // lazy/pending/error states) for `DiffViewer.svelte`.
  import type { SvelteSet } from 'svelte/reactivity';
  import type { DiffLine, FileDiff, Hunk, HunkOp, StageHunkResp } from '../api/types';
  import Icon from '../components/Icon.svelte';
  import LoadState from '../components/LoadState.svelte';
  import Skeleton from '../components/Skeleton.svelte';
  import type { DeferredHighlighter } from './diff-highlight.svelte';
  import { fileStat, type Row } from './diff-model';
  import { formatDiffBytes } from './diff-viewer-helpers';
  import { lineWords, markWords, type WordRange } from './diff-words';
  import { refocusRow } from './diffRowFocus';

  interface MeasureActionReturn {
    update(q: [string, number]): void;
    destroy(): void;
  }

  interface Props {
    r: Row;
    i: number;
    repoId?: string;
    wip?: {
      target: 'worktree' | 'staged';
      onapplied: (r: StageHunkResp) => void;
    };
    applying: boolean;
    hl: DeferredHighlighter;
    expandedLines: SvelteSet<string>;
    loadedFiles: Map<string, FileDiff>;
    rootEl: HTMLDivElement | undefined;
    measure: (node: HTMLElement, p: [string, number]) => MeasureActionReturn;
    langOf: (path: string) => string | null;
    isSelected: (path: string, hi: number, li: number) => boolean;
    selLabel: (verb: string, path: string, hi: number) => string;
    ontogglefile: (file: FileDiff) => void;
    onfiletools: (e: MouseEvent, file: FileDiff) => void;
    onapplyhunk: (file: FileDiff, hi: number, hunk: Hunk, op: HunkOp) => void;
    onselectline: (
      e: MouseEvent,
      path: string,
      hi: number,
      li: number,
      line: DiffLine,
      hunkLines?: DiffLine[],
    ) => void;
    onshowmore: (file: FileDiff, hi: number) => void;
    onloadanyway: (file: FileDiff) => void;
    onretryload: (file: FileDiff) => void;
  }

  let {
    r,
    i,
    repoId,
    wip,
    applying,
    hl,
    expandedLines,
    loadedFiles,
    rootEl,
    measure,
    langOf,
    isSelected,
    selLabel,
    ontogglefile,
    onfiletools,
    onapplyhunk,
    onselectline,
    onshowmore,
    onloadanyway,
    onretryload,
  }: Props = $props();

  const LINE_CUT = 10_000;
  const sign = (l: DiffLine) => (l.origin === 'add' ? '+' : l.origin === 'del' ? '−' : '');
</script>

{#snippet gut(
  side: 'old' | 'new',
  n: number | null,
  other: number | null,
  act: ((e: MouseEvent) => void) | null,
  pressed?: boolean,
)}
  {#if act && n != null}
    <button
      type="button"
      data-find-skip
      class="gut {side} selectable"
      tabindex={side === 'new' || other == null ? 0 : -1}
      aria-label="Select line {n}"
      aria-pressed={pressed === true}
      onclick={act}
    >{n}</button>
  {:else}
    <span data-find-skip class="gut {side}">{n ?? ''}</span>
  {/if}
{/snippet}

{#snippet codeText(
  content: string,
  lang: string | null,
  key: string,
  words: WordRange[] | null = null,
)}{#if content.length > LINE_CUT && !expandedLines.has(key)}{@html hl.html(
      content.slice(0, LINE_CUT),
      lang,
    )}<button
      type="button"
      class="line-cut-btn"
      data-find-skip
      aria-label={`Expand line (${formatDiffBytes(content.length)})`}
      title={`Show the full line (${formatDiffBytes(content.length)})`}
      onclick={(e) => {
        expandedLines.add(key);
        void refocusRow(e.currentTarget, rootEl);
      }}>… expand line ({formatDiffBytes(content.length)})</button
    >{:else}{@html markWords(hl.html(content, lang), words)}{/if}{/snippet}

{#if r.kind === 'file'}
  {@const stats = fileStat(r.file)}
  <div
    class="drow drow-file"
    class:first={r.first}
    class:open={!r.collapsed}
    id="dfile-{r.file.path}"
    data-rk={r.key}
    use:measure={[r.key, i]}
  >
    <div class="dfile-headrow">
      <button
        type="button"
        class="dfile-head"
        aria-expanded={!r.collapsed}
        onclick={() => ontogglefile(r.file)}
      >
        <span class="dfile-chevron">
          <Icon name={r.collapsed ? 'chevronRight' : 'chevronDown'} size={12} />
        </span>
        <span class="dfile-path mono" dir="ltr">
          {#if r.file.old_path}{r.file.old_path}<span class="rename-arrow"> → </span>{/if}{r.file.path}
        </span>
        <span class="grow"></span>
        <span class="add" data-find-skip>+{stats.add}</span>
        <span class="del" data-find-skip>−{stats.del}</span>
      </button>
      {#if repoId}
        <button
          type="button"
          class="dfile-tools"
          title="File history / blame"
          aria-label="File tools for {r.file.path}"
          onclick={(e) => {
            e.stopPropagation();
            onfiletools(e, r.file);
          }}
        ><Icon name="more" size={13} /></button>
      {/if}
    </div>
  </div>
{:else if r.kind === 'hunk'}
  <div class="drow inf hunk-header mono" dir="ltr" data-rk={r.key} use:measure={[r.key, i]}>
    <span>{r.hunk.header}</span>
    {#if wip && repoId}
      {@const w = wip}
      <span class="grow"></span>
      <button
        type="button"
        class="hunk-btn"
        disabled={applying}
        onclick={() => onapplyhunk(r.eff, r.hi, r.hunk, w.target === 'staged' ? 'unstage' : 'stage')}
      >{selLabel(w.target === 'staged' ? 'Unstage' : 'Stage', r.file.path, r.hi)}</button>
      {#if w.target === 'worktree'}
        <button
          type="button"
          class="hunk-btn danger"
          disabled={applying}
          onclick={() => onapplyhunk(r.eff, r.hi, r.hunk, 'discard')}
        >{selLabel('Discard', r.file.path, r.hi)}</button>
      {/if}
    {/if}
  </div>
{:else if r.kind === 'line'}
  {@const lang = langOf(r.file.path)}
  {@const effFile = loadedFiles.get(r.file.path) ?? r.file}
  {@const pick = !!wip && r.line.origin !== 'context'}
  {@const act = pick
    ? (e: MouseEvent) => onselectline(e, r.file.path, r.hi, r.li, r.line, effFile.hunks[r.hi]?.lines)
    : null}
  {@const picked = pick ? isSelected(r.file.path, r.hi, r.li) : undefined}
  <div
    class="vrow dline {r.line.origin}"
    class:selected={picked === true}
    data-rk={r.key}
    use:measure={[r.key, i]}
  >
    {@render gut('old', r.line.old_line, r.line.new_line, act, picked)}
    {@render gut('new', r.line.new_line, r.line.old_line, act, picked)}
    <span class="sign" data-find-skip>{sign(r.line)}</span>
    <span class="code mono">{@render codeText(r.line.content, lang, r.key, lineWords(effFile, r.hi, r.line))}</span>
  </div>
{:else if r.kind === 'split'}
  {@const lang = langOf(r.file.path)}
  {@const effFile = loadedFiles.get(r.file.path) ?? r.file}
  {@const L = r.sr.left}
  {@const R = r.sr.right}
  <div class="vrow split-vrow" data-rk={r.key} use:measure={[r.key, i]}>
    {@render gut('old', L?.old_line ?? null, null, null)}
    <span class="code mono half {L ? (L.origin === 'del' ? 'del' : '') : 'void'}"
      >{#if L}{@render codeText(L.content, lang, r.key + ':L', lineWords(effFile, r.hi, L))}{/if}</span>
    {@render gut('new', R?.new_line ?? null, null, null)}
    <span class="code mono half {R ? (R.origin === 'add' ? 'add' : '') : 'void'}"
      >{#if R}{@render codeText(R.content, lang, r.key + ':R', lineWords(effFile, r.hi, R))}{/if}</span>
  </div>
{:else if r.kind === 'more'}
  <div class="hunk-cap-cell" data-rk={r.key} use:measure={[r.key, i]}>
    <button type="button" class="btn small ghost hunk-cap-btn" onclick={() => onshowmore(r.file, r.hi)}>
      Show {r.remaining.toLocaleString()} more {r.remaining === 1 ? 'line' : 'lines'}
    </button>
  </div>
{:else if r.kind === 'note'}
  <div class="drow inf dfile-binary dim" data-rk={r.key} use:measure={[r.key, i]}>{r.text}</div>
{:else if r.kind === 'large'}
  <div class="drow inf dfile-large" data-rk={r.key} use:measure={[r.key, i]}>
    <Icon name="file" size={13} />
    <span>Large file — {r.lines.toLocaleString()} changed lines</span>
    {#if r.canLoad}
      <span aria-hidden="true">·</span>
      <button type="button" class="btn small" onclick={() => onloadanyway(r.file)}>Load anyway</button>
    {/if}
  </div>
{:else if r.kind === 'pending'}
  <div class="drow inf dfile-pending" data-rk={r.key} use:measure={[r.key, i]}>
    <Skeleton rows={1} height={14} label="this file’s diff" grace={false} />
  </div>
{:else if r.kind === 'error'}
  <div class="drow inf dfile-pending" data-rk={r.key} use:measure={[r.key, i]}>
    <LoadState what="file diff" error={r.message} empty onretry={() => onretryload(r.file)} variant="compact" />
  </div>
{:else if r.kind === 'end'}
  <div class="drow drow-end" data-rk={r.key} use:measure={[r.key, i]}></div>
{/if}

<style>
  .drow-file {
    padding-block-start: 10px;
  }
  .drow-file.first {
    padding-block-start: 0;
  }
  .inf {
    border-inline: 1px solid var(--border);
    background: var(--surface);
  }
  .drow-end {
    block-size: 4px;
    border: 1px solid var(--border);
    border-block-start: none;
    border-end-start-radius: var(--radius-m);
    border-end-end-radius: var(--radius-m);
    background: var(--surface);
  }
  .dfile-large {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 12px;
    padding-inline: 14px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .dfile-pending {
    padding-block: 12px;
    padding-inline: 14px;
    font-size: var(--fs-s);
  }
  .dfile-head {
    display: flex;
    align-items: center;
    gap: 8px;
    inline-size: 100%;
    padding-block: 6px;
    padding-inline: 12px;
    border: none;
    background: var(--surface-2);
    cursor: pointer;
    font-size: var(--fs-s);
    color: var(--text);
    text-align: start;
  }
  .dfile-head .add {
    color: var(--success);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .dfile-head .del {
    color: var(--danger);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .dfile-path {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dfile-chevron,
  .rename-arrow {
    display: inline-flex;
  }
  :global([dir='rtl']) .rename-arrow {
    transform: scaleX(-1);
  }
  .dfile-binary {
    padding: 14px;
    font-size: var(--fs-s);
  }
  .dfile-headrow {
    display: flex;
    align-items: stretch;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .drow-file.open .dfile-headrow {
    border-end-start-radius: 0;
    border-end-end-radius: 0;
  }
  .dfile-headrow .dfile-head {
    min-inline-size: 0;
  }
  .dfile-tools {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    padding-block: 0;
    padding-inline: 10px;
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: var(--fs-m);
    line-height: 1;
  }
  .dfile-tools:hover {
    color: var(--accent-text);
  }
  .hunk-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 2px;
    padding-inline: 12px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
    border-block-start: 1px solid var(--border);
    border-block-end: 1px solid var(--border);
  }
  .gut {
    inline-size: 42px;
    min-inline-size: 42px;
    text-align: end;
    padding-block: 0;
    padding-inline: 4px 8px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    user-select: none;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  button.gut {
    appearance: none;
    margin: 0;
    background: none;
    border-block: 0;
    border-inline-start: 0;
    line-height: inherit;
    border-radius: 0;
  }
  button.gut:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .gut.selectable {
    cursor: pointer;
  }
  .gut.selectable:hover {
    background: var(--accent-soft-strong);
    color: var(--accent-text);
  }
  .hunk-btn {
    flex-shrink: 0;
    padding-block: 1px;
    padding-inline: 6px;
    border: 1px solid var(--accent-line);
    border-radius: var(--radius-s);
    background: none;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .hunk-btn:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .hunk-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .hunk-btn.danger {
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
  }
  .hunk-btn.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
  }
  .vrow.dline.selected,
  .vrow.dline.selected .gut {
    background: var(--accent-soft);
  }
  .sign {
    inline-size: 16px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    font-family: var(--font-mono);
  }
  .code {
    padding-block: 0;
    padding-inline: 4px 10px;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .line-cut-btn {
    display: inline;
    margin-inline-start: 6px;
    padding-block: 0;
    padding-inline: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .line-cut-btn:hover {
    background: var(--accent-soft);
  }
  .line-cut-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .hunk-cap-cell {
    text-align: center;
    padding-block: 4px;
    padding-inline: 8px;
    border-block-start: 1px dashed var(--border);
  }
  .hunk-cap-btn {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hunk-cap-btn:hover {
    color: var(--text);
  }
  .vrow {
    display: grid;
    grid-template-columns: 42px 42px 16px 1fr;
    align-items: start;
  }
  .vrow .gut {
    display: block;
    inline-size: auto;
    min-inline-size: 0;
    align-self: stretch;
  }
  .vrow.dline.add {
    background: color-mix(in srgb, var(--success) 11%, transparent);
  }
  .vrow.dline.del {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .split-vrow {
    grid-template-columns: 42px 1fr 42px 1fr;
  }
  .split-vrow .code.half.add {
    background: color-mix(in srgb, var(--success) 11%, transparent);
  }
  .split-vrow .code.half.del {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .split-vrow .code.half.void {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
  .dline.add :global(.dw),
  .code.half.add :global(.dw) {
    background: color-mix(in srgb, var(--success) 32%, transparent);
    border-radius: 2px;
  }
  .dline.del :global(.dw),
  .code.half.del :global(.dw) {
    background: color-mix(in srgb, var(--danger) 30%, transparent);
    border-radius: 2px;
  }
  @media (max-width: 1024px) {
    .dfile-head {
      font-size: var(--fs-m);
      padding-block: 8px;
      padding-inline: 12px;
      min-block-size: 40px;
    }
    .dfile-head :is(.add, .del) {
      font-size: var(--fs-m);
    }
    .dfile-path {
      font-size: var(--fs-m);
    }
    .hunk-header {
      font-size: var(--fs-s);
      padding-block: 4px;
      padding-inline: 12px;
    }
    .gut {
      font-size: var(--fs-xs);
      inline-size: 30px;
      min-inline-size: 30px;
      padding-block: 0;
      padding-inline: 2px 4px;
    }
    .sign {
      inline-size: 13px;
    }
    .code {
      font-size: var(--fs-s);
      white-space: pre-wrap;
      word-break: break-word;
      overflow-wrap: anywhere;
    }
    .vrow.dline {
      grid-template-columns: 30px 30px 13px 1fr;
    }
    .vrow .code {
      white-space: pre-wrap;
      word-break: break-word;
      overflow-wrap: anywhere;
    }
    .hunk-cap-btn {
      font-size: var(--fs-s);
      min-block-size: 32px;
      padding-block: 6px;
      padding-inline: 12px;
    }
  }
</style>
