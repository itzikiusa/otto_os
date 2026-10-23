<script lang="ts">
  // One colour-coded ref chip (local / remote / tag / HEAD / stash / worktree),
  // shared by the commit rows (inline and collapsed) and the commit detail
  // header so the renderings never drift apart. `lane` tints the side facing
  // the graph in the colour of the node the ref sits on; `tracking` adds the
  // ahead/behind counts of a local branch.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { RefChip } from '../ref-menus';

  interface Props {
    chip: RefChip;
    label: string;
    title: string;
    lane?: string;
    tracking?: { ahead: number; behind: number } | null;
  }
  let { chip, label, title, lane, tracking = null }: Props = $props();
</script>

<span
  class="ref-chip kind-{chip.kind}"
  class:current-chip={chip.current}
  class:is-worktree={chip.worktree && !chip.current}
  style:--lane={lane}
  {title}
>
  {#if chip.current}<Icon name="check" size={12} />{/if}
  {#if chip.worktree && !chip.current}<Icon name="worktree" size={12} />{/if}
  {#if chip.kind === 'remote' || chip.onRemote}<Icon name="globe" size={12} />{/if}
  {#if chip.kind === 'tag'}<Icon name="tag" size={12} />{/if}
  {#if chip.kind === 'stash'}<Icon name="stash" size={12} />{/if}
  <span class="chip-label">{label}</span>
  {#if tracking && (tracking.ahead > 0 || tracking.behind > 0)}
    <span class="chip-ab" title="{tracking.ahead} ahead · {tracking.behind} behind upstream">
      {#if tracking.ahead > 0}<span class="ab-ahead">↑{tracking.ahead}</span>{/if}
      {#if tracking.behind > 0}<span class="ab-behind">↓{tracking.behind}</span>{/if}
    </span>
  {/if}
</span>

<style>
  .ref-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 1px 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    border: 1px solid transparent;
    /* Lane tint: a bar on the side facing the graph, in the SAME color as the
       node this ref sits on, so a label is readable as belonging to one lane
       (the SVG leader line completes the connection). */
    border-inline-end: 3px solid var(--lane, transparent);
    white-space: nowrap;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Local branch — subtle, neutral. */
  .ref-chip.kind-local {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  /* Remote-tracking branch — distinct teal/cyan so it never reads as local. */
  .ref-chip.kind-remote {
    background: color-mix(in srgb, var(--cat-6) 18%, transparent);
    color: var(--text);
  }
  /* Tag — amber. */
  .ref-chip.kind-tag {
    background: var(--warning-soft);
    color: var(--warning);
  }
  /* Detached HEAD — muted warning. */
  .ref-chip.kind-detached {
    background: color-mix(in srgb, var(--danger) 18%, transparent);
    color: var(--danger);
  }
  /* The checked-out branch — the most prominent chip (filled accent). */
  .ref-chip.kind-head,
  .ref-chip.current-chip {
    background: var(--accent-solid);
    color: var(--accent-contrast);
    border-color: var(--accent-solid);
  }
  /* Branch checked out in another worktree — violet, not local-branch blue. */
  .ref-chip.is-worktree {
    background: color-mix(in srgb, var(--worktree) 18%, transparent);
    color: var(--worktree-text);
    border-color: color-mix(in srgb, var(--worktree) 35%, transparent);
  }
  /* Stash decoration — dashed + muted so it reads as plumbing, not a branch. */
  .ref-chip.kind-stash {
    background: transparent;
    color: var(--text-dim);
    border: 1px dashed var(--text-dim);
  }
  .ref-chip > :global(svg) {
    flex-shrink: 0;
  }
  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .chip-ab {
    display: inline-flex;
    flex-shrink: 0;
    gap: 2px;
    margin-inline-start: 2px;
    padding-inline-start: 2px;
    border-inline-start: 1px solid color-mix(in srgb, var(--accent-contrast) 45%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .ab-ahead {
    color: var(--success);
  }
  .ab-behind {
    color: var(--danger);
  }
  .current-chip .ab-ahead,
  .current-chip .ab-behind,
  .kind-head .ab-ahead,
  .kind-head .ab-behind {
    color: var(--accent-contrast);
  }
</style>
