<script lang="ts">
  // A review note on the Changes diff (or in the Review bar's list): where,
  // its state in words — Draft, Sent, Resolved, Outdated (the agent changed
  // the lines since) — the comment, and Edit / Resolve / Delete.
  import Icon from '../../lib/components/Icon.svelte';
  import DiffNoteEditor from './DiffNoteEditor.svelte';
  import { noteWhere, type ReviewNote } from './runtime/reviewNotes';

  interface Props {
    note: ReviewNote;
    /** Name the file too (the bar's list). */
    showPath?: boolean;
    onedit: (comment: string) => void;
    onresolve: (resolved: boolean) => void;
    ondelete: () => void;
  }
  let { note, showPath = false, onedit, onresolve, ondelete }: Props = $props();
  let editing = $state(false);
  const where = $derived(`${showPath ? `${note.path} · ` : ''}${noteWhere(note) || 'file'}`);
  const word = $derived(note.outdated && note.state !== 'resolved' ? 'Outdated' : note.state === 'draft' ? 'Draft' : note.state === 'sent' ? 'Sent' : 'Resolved');
</script>

{#if editing}
  <DiffNoteEditor {where} initial={note.comment} onsave={(c) => { onedit(c); editing = false; }} oncancel={() => (editing = false)} />
{:else}
  <div class="dnc" class:resolved={note.state === 'resolved'} data-testid="diff-note" data-state={word.toLowerCase()}>
    <div class="dnc-head">
      <span class="dnc-where mono ellipsis" dir="ltr">{where}</span>
      <span class="dnc-state {word.toLowerCase()}" data-testid="diff-note-state">{word}</span>
      <span class="dnc-grow"></span>
      {#if note.state !== 'resolved'}
        <button type="button" class="icon-btn" aria-label="Edit note on {where}" title="Edit" onclick={() => (editing = true)}><Icon name="edit" size={12} /></button>
      {/if}
      {#if note.state !== 'draft'}
        <button type="button" class="icon-btn" aria-label={note.state === 'resolved' ? `Reopen note on ${where}` : `Resolve note on ${where}`} title={note.state === 'resolved' ? 'Reopen' : 'Resolve'} data-testid="diff-note-resolve" onclick={() => onresolve(note.state !== 'resolved')}><Icon name={note.state === 'resolved' ? 'undo' : 'check'} size={12} /></button>
      {/if}
      <button type="button" class="icon-btn" aria-label="Delete note on {where}" title="Delete" data-testid="diff-note-delete" onclick={ondelete}><Icon name="trash" size={12} /></button>
    </div>
    <p class="dnc-text" dir="auto">{note.comment}</p>
  </div>
{/if}

<style>
  .dnc {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin-block: var(--sp-1);
    margin-inline: var(--sp-2);
    padding-block: var(--sp-1) var(--sp-2);
    padding-inline: var(--sp-3) var(--sp-1);
    border: 1px solid var(--border);
    border-inline-start: 3px solid var(--accent-solid);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-family: var(--font-ui);
    white-space: normal;
  }
  .dnc.resolved {
    opacity: 0.7;
    border-inline-start-color: var(--border-strong);
  }
  .dnc-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-inline-size: 0;
  }
  .dnc-where {
    min-inline-size: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .dnc-state {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .dnc-state.draft {
    color: var(--accent-text);
  }
  .dnc-state.sent {
    color: var(--text-dim);
  }
  .dnc-state.outdated {
    color: var(--warning);
  }
  .dnc-state.resolved {
    color: var(--success);
  }
  .dnc-grow {
    flex: 1;
  }
  .dnc-text {
    margin: 0;
    color: var(--text);
    font-size: var(--fs-m);
    line-height: 1.5;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  @media (pointer: coarse) {
    .dnc-head :global(.icon-btn) {
      inline-size: calc(var(--hit-min) + 8px);
      block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
