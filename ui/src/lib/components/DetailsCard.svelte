<script lang="ts">
  // The details card (patterns.md "Details card"): ONE card of facts and the
  // next action for the open object — it floats beside a reading column when
  // there is room (`floating`), and heads the right panel otherwise. First
  // shipped as Threads' details card (modules/threads/ThreadDetailsCard). The
  // shell owns the frame and the row vocabulary every card uses:
  //   .dc-group        a section (hairline between sections)
  //   .dc-line         a row with an end control (.dc-end-btn / .dc-more / .dc-x)
  //   .dc-row          one fact or door (a <button> when it opens something;
  //                    + .dc-static when it is not), .dc-grow fills a .dc-line
  //   .dc-label        the row's text (one line, ellipsis); .dc-stat / .dc-end
  //                    a dim number at its end; .dc-warn / .dc-danger tones
  //   .dc-cap / .dc-note  a dim caption / note line
  // `action`: the card's one primary, last (it follows state: Commit → Push → …).
  import type { Snippet } from 'svelte';

  interface Props {
    /** The card's landmark name ("Thread details"). */
    label: string;
    floating?: boolean;
    testid?: string;
    children: Snippet;
    action?: Snippet;
  }

  let { label, floating = false, testid, children, action }: Props = $props();
</script>

<section class="dc-card" class:floating aria-label={label} data-testid={testid}>
  {@render children()}
  {#if action}
    <div class="dc-group dc-action">{@render action()}</div>
  {/if}
</section>

<style>
  .dc-card {
    display: flex;
    flex-direction: column;
    min-inline-size: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    font-size: var(--fs-m);
    overflow: hidden;
  }
  .dc-card.floating {
    box-shadow: var(--shadow-card);
  }
  .dc-card :global(.dc-group) {
    display: flex;
    flex-direction: column;
    padding-block: var(--sp-3);
  }
  .dc-card :global(.dc-group + .dc-group) {
    border-block-start: 1px solid var(--border);
  }
  .dc-card :global(.dc-line) {
    display: flex;
    align-items: center;
    min-inline-size: 0;
  }
  .dc-card :global(.dc-row) {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    min-inline-size: 0;
    min-block-size: 34px;
    padding-inline: var(--sp-6);
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .dc-card :global(.dc-grow) {
    flex: 1;
  }
  .dc-card :global(.dc-row svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .dc-card :global(button.dc-row:hover:not(:disabled)) {
    background: var(--hover);
  }
  .dc-card :global(.dc-row:disabled) {
    cursor: default;
    color: var(--text-dim);
  }
  .dc-card :global(.dc-static) {
    cursor: default;
  }
  .dc-card :global(.dc-dim) {
    color: var(--text-dim);
  }
  .dc-card :global(.dc-label) {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dc-card :global(.dc-end-btn) {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    flex: none;
    block-size: 28px;
    padding-inline: var(--sp-4);
    margin-inline-end: var(--sp-3);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .dc-card :global(.dc-end-btn:hover) {
    background: var(--hover);
    color: var(--text);
  }
  .dc-card :global(.dc-x),
  .dc-card :global(.dc-more) {
    flex: none;
    margin-inline-end: var(--sp-3);
    color: var(--text-dim);
  }
  .dc-card :global(.dc-end) {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .dc-card :global(.dc-warn) {
    color: var(--warning);
  }
  .dc-card :global(.dc-danger) {
    color: var(--danger);
  }
  .dc-card :global(.dc-stat) {
    display: inline-flex;
    gap: var(--sp-3);
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .dc-card :global(.dc-cap) {
    padding-block: var(--sp-3);
    padding-inline: var(--sp-6);
    color: var(--text-dim);
    font-size: var(--fs-s);
    margin: 0;
  }
  .dc-card :global(.dc-note) {
    margin: 0;
    padding-block: var(--sp-3);
    padding-inline: var(--sp-6);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  /* Touch: every control is a full finger-sized target (44 px, Apple's HIG). */
  @media (pointer: coarse) {
    .dc-card :global(.dc-row),
    .dc-card :global(.dc-end-btn),
    .dc-card :global(.dc-x),
    .dc-card :global(.dc-more) {
      min-block-size: 44px;
    }
    .dc-card :global(.dc-x),
    .dc-card :global(.dc-more) {
      min-inline-size: 44px;
    }
  }
  .dc-action {
    padding-inline: var(--sp-5);
  }
</style>
