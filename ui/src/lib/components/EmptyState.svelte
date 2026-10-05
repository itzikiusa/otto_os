<script lang="ts">
  // The one empty-state pattern: icon tile + title + one-line body + ONE
  // primary CTA.
  //
  // • variant="page"  — a whole page / main pane has nothing to show. Sits at a
  //   fixed top offset (≈15vh), NOT vertically centred, so every module's empty
  //   page reads the same regardless of pane height.
  // • variant="panel" (default) — inside a pane/card/list; compact padding, the
  //   parent decides placement (kept for the ~130 existing in-panel uses).
  //
  // `children` is an escape hatch for a secondary link/hint under the CTA —
  // keep it quiet (a `.btn ghost` or dim text), never a second primary.
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';

  interface Props {
    icon?: IconName;
    title: string;
    body?: string;
    actionLabel?: string;
    actionIcon?: IconName;
    onaction?: () => void;
    /** 'secondary' inside a card / widget / pane that already has (or sits
     *  beside) the view's one primary action. */
    actionKind?: 'primary' | 'secondary';
    variant?: 'page' | 'panel';
    /** 'error' marks a failed state: danger icon tile + role="alert". */
    tone?: 'neutral' | 'error';
    /** Heading level of the title, so it nests under the surrounding outline:
     *  2 for a page-level empty state under the page's h1, 4 inside a card
     *  under an h3. Default 3. Looks the same at every level. */
    headingLevel?: 2 | 3 | 4 | 5;
    children?: Snippet;
  }
  let { icon = 'box', title, body, actionLabel, actionIcon, onaction, actionKind = 'primary', variant = 'panel', tone = 'neutral', headingLevel = 3, children }: Props = $props();

  // A label without a handler would render a dead button, so the CTA needs
  // both — and a caller that passes only the label is told, not silently
  // left without its action.
  $effect(() => {
    if (import.meta.env.DEV && actionLabel && !onaction) {
      console.warn(`EmptyState “${title}”: actionLabel “${actionLabel}” has no onaction — the CTA is not rendered.`);
    }
  });
</script>

<div class="empty" class:page={variant === 'page'} role={tone === 'error' ? 'alert' : undefined} data-testid={variant === 'page' ? 'page-empty' : undefined}>
  <!-- Decorative: the title says what is (not) here. -->
  <div class="empty-icon" class:error={tone === 'error'} aria-hidden="true"><Icon name={icon} size={variant === 'page' ? 26 : 24} /></div>
  <svelte:element this={`h${headingLevel}`} class="empty-title">{title}</svelte:element>
  {#if body}<p>{body}</p>{/if}
  {#if actionLabel && onaction}
    <button class="btn" class:primary={actionKind === 'primary'} onclick={onaction}>
      {#if actionIcon}<Icon name={actionIcon} size={13} />{/if}
      {actionLabel}
    </button>
  {/if}
  {#if children}{@render children()}{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
    color: var(--text-dim);
  }
  /* Page variant: pinned near the top, full width of its pane. */
  .empty.page {
    justify-content: flex-start;
    width: 100%;
    box-sizing: border-box;
    padding: 15vh 24px 48px;
  }
  .empty-icon {
    width: 56px;
    height: 56px;
    border-radius: var(--radius-l);
    background: var(--surface-2);
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    margin-bottom: 4px;
    color: var(--text-dim);
  }
  .empty-icon.error {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .empty-title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .page .empty-title {
    font-size: var(--fs-l);
  }
  p {
    margin: 0;
    font-size: var(--fs-s);
    max-width: 380px;
    line-height: 1.5;
    white-space: pre-line;
    /* A body quoting a path or id has no break points. */
    overflow-wrap: anywhere;
  }
  button {
    margin-top: 8px;
  }
</style>
