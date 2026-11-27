<script lang="ts">
  // One inbox row (docs/design/guidelines/patterns.md "Inbox rows, status
  // words"; first shipped as the Threads sidebar row): three lines, the whole
  // row one button —
  //   1. the project's badge and name (+ `context` chips) … the status WORD
  //      (lib/statusWords.ts), else the time;
  //   2. the title at --fs-l, bold while it needs you or finished unread;
  //   3. a dim third line (`line3`: branch, folder, kind · host …).
  // A `child` row (nested under its lead) is two lines: title, then its word
  // and `line3`. A `twoLine` row (a list with no project, e.g. connections)
  // drops line 1: the title carries the status word at its end, then `line3`. Hover/focus actions (`actions`) sit over line 3's end; on
  // touch they live in the row's long-press menu (use:rowMenu → oncontextmenu),
  // and an `onPage` row keeps them visible at its end as well.
  // Purely presentational: the page derives the words, the state and the
  // handlers. The `tr-*` class names are kept (specs and the sidebar keyboard
  // walk select them).
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import ProjectBadge from './ProjectBadge.svelte';
  import { rowMenu } from '../rowMenu';
  import type { WordTone } from '../statusWords';

  interface Props {
    /** Line 2. */
    title: string;
    /** Line 1's badge + name; omitted on a child row. */
    project?: string | null;
    /** The status word (null → `time` shows instead) and its tone. */
    word?: string | null;
    tone?: WordTone;
    time?: string;
    /** Hover tooltip of the row button. */
    tip?: string;
    child?: boolean;
    /** Two lines: title … word, then `line3` (no project line). */
    twoLine?: boolean;
    /** Needs you / finished unread: the title goes bold. */
    strong?: boolean;
    selected?: boolean;
    /** Open in a pane but not the focused one (a faint accent tint). */
    inPane?: boolean;
    /** The row sits on the page background, not the sidebar's. */
    onPage?: boolean;
    testid?: string;
    statusTestid?: string;
    /** `data-state` / `data-class` for specs and styling. */
    state?: string;
    cls?: string;
    /** Extra chips after the project name on line 1. */
    context?: Snippet;
    /** After the title on line 2 (a count). */
    titleEnd?: Snippet;
    line3?: Snippet;
    actions?: Snippet;
    /** A fold chevron before the row, for rows with nested rows. */
    fold?: { folded: boolean; label: string; testid?: string; ontoggle: () => void };
    onclick: (e: MouseEvent) => void;
    oncontextmenu?: (e: MouseEvent) => void;
  }

  let {
    title,
    project = null,
    word = null,
    tone = '',
    time = '',
    tip,
    child = false,
    twoLine = false,
    strong = false,
    selected = false,
    inPane = false,
    onPage = false,
    testid,
    statusTestid,
    state,
    cls,
    context,
    titleEnd,
    line3,
    actions,
    fold,
    onclick,
    oncontextmenu,
  }: Props = $props();
</script>

<div
  class="tr-row reveal-host"
  class:child
  class:two-line={twoLine}
  class:tr-family={!!fold}
  class:strong
  class:selected
  class:in-pane={inPane && !selected}
  class:on-page={onPage}
  data-testid={testid}
  data-state={state}
  data-class={cls}
>
  {#if fold}
    <button type="button" class="icon-btn tr-fold" aria-expanded={!fold.folded} aria-label="{fold.folded ? 'Show' : 'Fold'} {fold.label}" title={fold.folded ? `Show ${fold.label}` : `Fold ${fold.label}`} data-testid={fold.testid} onclick={fold.ontoggle}>
      <Icon name={fold.folded ? 'chevronRight' : 'chevronDown'} size={12} />
    </button>
  {/if}
  <button type="button" class="tr-main" title={tip} aria-current={selected ? 'true' : undefined} {onclick} {oncontextmenu} use:rowMenu>
    {#if !child && !twoLine}
      <span class="tr-line tr-l1">
        {#if project}
          <ProjectBadge name={project} />
          <span class="tr-proj ellipsis" dir="auto">{project}</span>
        {/if}
        {@render context?.()}
        <span class="tr-status {tone}" dir="auto" data-testid={statusTestid}>{word ?? time}</span>
      </span>
    {/if}
    <span class="tr-line tr-l2">
      <span class="tr-title ellipsis" dir="auto">{title}</span>
      {@render titleEnd?.()}
      {#if twoLine}
        <span class="tr-status {tone}" dir="auto" data-testid={statusTestid}>{word ?? time}</span>
      {/if}
    </span>
    <span class="tr-line tr-l3">
      {#if child}
        <span class="tr-status {tone}" dir="auto" data-testid={statusTestid}>{word ?? time}</span>
      {/if}
      {@render line3?.()}
    </span>
  </button>
  {#if actions}
    <div class="tr-actions reveal-on-hover">
      {@render actions()}
    </div>
  {/if}
</div>

<style>
  .tr-row {
    --tr-tint: transparent;
    --tr-under: var(--bg-sidebar);
    position: relative;
    display: flex;
    align-items: flex-start;
    min-width: 0;
    border-radius: var(--radius-s);
    background: var(--tr-tint);
  }
  .tr-row.on-page {
    --tr-under: var(--bg);
  }
  .tr-row:hover {
    --tr-tint: var(--hover);
  }
  .tr-row.in-pane {
    --tr-tint: var(--accent-faint);
  }
  /* The one selection look of every list (design.md §7.10): the accent
     tint and a bold title; keyboard focus is an inset ring, no fill. */
  .tr-row.selected {
    --tr-tint: var(--accent-soft);
  }
  .tr-row.selected .tr-title {
    font-weight: 600;
  }
  .tr-main:focus-visible {
    outline-offset: -2px;
  }
  .tr-row.child {
    margin-inline-start: var(--sp-6);
  }
  .tr-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    /* Chrome before 128 gives a <button> `align-items: flex-start` (UA
       sheet): the lines then size to their text and a long title overflows
       the row instead of ending in an ellipsis. */
    align-items: stretch;
    gap: var(--sp-1);
    min-block-size: 64px;
    padding-block: var(--sp-3);
    /* 8 px: with the pane's 8 px inset, text starts 16 px in, as everywhere. */
    padding-inline: var(--sp-4);
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .tr-row.child .tr-main {
    min-block-size: 46px;
  }
  .tr-row.two-line .tr-main {
    min-block-size: 56px;
  }
  .tr-line {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
    block-size: 18px;
  }
  .tr-l1,
  .tr-l3 {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tr-l2 {
    block-size: 22px;
  }
  .tr-l3 :global(svg) {
    flex-shrink: 0;
  }
  .tr-proj {
    min-width: 0;
  }
  .tr-title {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-l);
    font-weight: 500;
  }
  .tr-row.strong .tr-title {
    font-weight: 600;
  }
  .tr-status {
    flex-shrink: 0;
    margin-inline-start: auto;
    font-variant-numeric: tabular-nums;
  }
  .tr-row.child .tr-status {
    margin-inline-start: 0;
  }
  .tr-status.warning,
  .tr-status.danger,
  .tr-status.success {
    font-weight: 600;
  }
  .tr-status.warning {
    color: var(--warning);
  }
  .tr-status.danger {
    color: var(--danger);
  }
  .tr-status.success {
    color: var(--success);
  }
  /* Hover actions over line 3's end, never over the status word or the
     title. Opaque so the line never shows through. */
  .tr-actions {
    position: absolute;
    inset-block-end: var(--sp-2);
    inset-inline-end: var(--sp-2);
    display: flex;
    align-items: center;
    padding-inline-start: var(--sp-3);
    border-radius: var(--radius-s);
    background: linear-gradient(var(--tr-tint), var(--tr-tint)), var(--tr-under);
  }
  /* Line 3's height, so they never reach the title above it. */
  .tr-actions :global(.icon-btn) {
    block-size: 20px;
    min-block-size: 20px;
  }
  .tr-fold {
    flex: none;
    margin-block-start: var(--sp-6);
    margin-inline-end: calc(-1 * var(--sp-4));
  }
  /* Touch: the row is one tap target; its actions live in its long-press
     menu (use:rowMenu); the fold chevron is finger-sized. */
  @media (hover: none), (pointer: coarse) {
    .tr-actions {
      display: none;
    }
    /* …except on a full-page list (Connections, Git, Home): it has the room,
       so its ⋯ stays a visible, finger-sized control at the row's end. */
    .tr-row.on-page .tr-actions {
      display: flex;
      position: static;
      align-self: center;
      padding: 0;
      background: none;
    }
    .tr-row.on-page .tr-actions :global(.icon-btn) {
      inline-size: 44px;
      block-size: 44px;
    }
    .tr-fold {
      inline-size: var(--hit-min);
      block-size: var(--hit-min);
      margin-block-start: var(--sp-5);
    }
  }
</style>
