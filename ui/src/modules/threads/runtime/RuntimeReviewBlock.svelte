<script lang="ts">
  // A review sent from the Changes panel, in the timeline: one compact
  // "Review · N notes" block that opens to each note (file, lines, the quoted
  // code, the comment) instead of the message's plain-text scaffold, which is
  // what the agent reads (reviewNotes.ts `sentReview`).
  import Icon from '../../../lib/components/Icon.svelte';
  import type { SentReviewNote } from './reviewNotes';

  interface Props {
    label: string;
    notes: readonly SentReviewNote[];
  }
  let { label, notes }: Props = $props();
</script>

<details class="rv" data-testid="runtime-review-block">
  <summary class="rv-sum"><Icon name="comment" size={13} /><span>{label}</span><Icon name="chevronDown" size={12} /></summary>
  <ol class="rv-list">
    {#each notes as n, i (i)}
      <li class="rv-note">
        <p class="rv-where mono" dir="ltr"><span class="ellipsis">{n.path}</span>{#if n.where}<span class="rv-lines">{n.where}</span>{/if}</p>
        {#if n.quote.length}<pre class="rv-quote mono" dir="ltr">{n.quote.join('\n')}</pre>{/if}
        <p class="rv-comment" dir="auto">{n.comment}</p>
      </li>
    {/each}
  </ol>
</details>

<style>
  .rv {
    align-self: flex-end;
    max-inline-size: min(100%, 560px);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .rv-sum {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-block-size: 32px;
    padding-inline: var(--sp-3);
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    list-style: none;
  }
  .rv-sum::-webkit-details-marker {
    display: none;
  }
  .rv[open] .rv-sum :global(svg:last-child) {
    rotate: 180deg;
  }
  .rv-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    margin: 0;
    padding-block: 0 var(--sp-3);
    padding-inline: var(--sp-3);
    list-style: none;
  }
  .rv-note {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding-block-start: var(--sp-2);
    border-block-start: 1px solid var(--border);
  }
  .rv-where {
    display: flex;
    gap: var(--sp-2);
    margin: 0;
    min-inline-size: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rv-lines {
    flex-shrink: 0;
  }
  .rv-quote {
    margin: 0;
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .rv-comment {
    margin: 0;
    color: var(--text);
    font-size: var(--fs-m);
    white-space: pre-wrap;
  }
  @media (pointer: coarse) {
    .rv-sum {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
