<script lang="ts">
  // The Changes panel's one review line: "Review · N notes" with Send to
  // agent (one message to the focused thread at once; the notes read Sent when
  // the daemon took it), Add to message (the notes in the draft, to add a
  // sentence first) in ⋯-less form as the quiet second action, Clear, and
  // every note (any file) one tap away.
  import { plural } from '../../lib/plural';
  import DiffNoteCard from './DiffNoteCard.svelte';
  import type { ReviewNote } from './runtime/reviewNotes';

  interface Props {
    notes: readonly ReviewNote[];
    onsend: () => void;
    /** Put the notes in the thread's draft instead (absent: no draft to add to). */
    ondraft?: () => void;
    sending?: boolean;
    onclear: () => void;
    onedit: (id: string, comment: string) => void;
    onresolve: (id: string, resolved: boolean) => void;
    ondelete: (id: string) => void;
  }
  let { notes, onsend, ondraft, sending = false, onclear, onedit, onresolve, ondelete }: Props = $props();
  let open = $state(false);
  const drafts = $derived(notes.filter((n) => n.state === 'draft').length);
</script>

{#if notes.length}
  <div class="drb" data-testid="diff-review-bar">
    <div class="drb-line">
      <button type="button" class="drb-toggle" aria-expanded={open} onclick={() => (open = !open)}>
        <span class="drb-title">Review</span>
        <span class="drb-count">{drafts ? plural(drafts, 'note') : `${plural(notes.length, 'note')} sent`}</span>
      </button>
      <span class="drb-grow"></span>
      <button type="button" class="btn small ghost" data-testid="diff-review-clear" onclick={onclear}>Clear</button>
      {#if ondraft}<button type="button" class="btn small ghost" data-testid="diff-review-draft" disabled={!drafts || sending} title="Put the notes in this thread’s message, to add a sentence before sending" onclick={ondraft}>Add to message</button>{/if}
      <button type="button" class="btn small primary" data-testid="diff-review-send" disabled={!drafts || sending} title="Send the notes to this thread’s agent now (queued while it works)" onclick={onsend}>{sending ? 'Sending…' : 'Send to agent'}</button>
    </div>
    {#if open}
      <div class="drb-list">
        {#each notes as n (n.id)}
          <DiffNoteCard note={n} showPath onedit={(c) => onedit(n.id, c)} onresolve={(r) => onresolve(n.id, r)} ondelete={() => ondelete(n.id)} />
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .drb {
    flex-shrink: 0;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
  }
  .drb-line {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding-block: var(--sp-1);
    padding-inline: var(--sp-2);
  }
  .drb-toggle {
    display: inline-flex;
    align-items: baseline;
    gap: var(--sp-2);
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .drb-toggle:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .drb-title {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .drb-count {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .drb-grow {
    flex: 1;
  }
  .drb-list {
    max-block-size: 40vh;
    overflow-y: auto;
    padding-block-end: var(--sp-2);
  }
  @media (pointer: coarse) {
    .drb-line :global(.btn),
    .drb-toggle {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
