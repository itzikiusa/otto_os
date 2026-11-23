<script lang="ts">
  // The note box under a line (or a range) of the Changes diff: a comment for
  // the agent. ⌘↵ / Ctrl+↵ saves, Esc cancels. With nothing typed, the lines
  // can still go to the chat as plain context (the old "Add line to chat").
  import { untrack } from 'svelte';
  import { isMacPlatform } from '../../lib/keys';

  interface Props {
    /** "src/a.ts · new 12–14". */
    where: string;
    initial?: string;
    onsave: (comment: string) => void;
    oncancel: () => void;
    /** Add the lines to the chat as context, without a note. */
    onplain?: () => void;
  }
  let { where, initial = '', onsave, oncancel, onplain }: Props = $props();
  let text = $state(untrack(() => initial)); // the box starts from it, then it is the person's
  let box = $state<HTMLTextAreaElement | null>(null);
  $effect(() => box?.focus({ preventScroll: false }));
  const save = (): void => {
    if (text.trim()) onsave(text.trim());
  };
  const keys = isMacPlatform() ? '⌘↵' : 'Ctrl+↵';
</script>

<div class="dne" data-testid="diff-note-editor">
  <span class="dne-where mono" dir="ltr">{where}</span>
  <textarea
    bind:value={text}
    rows="3"
    dir="auto"
    bind:this={box}
    aria-label="Note on {where}"
    placeholder="Note for the agent on these lines"
    data-testid="diff-note-input"
    onkeydown={(e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        oncancel();
      } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && !e.isComposing) {
        e.preventDefault();
        save();
      }
    }}
  ></textarea>
  <div class="dne-actions">
    {#if onplain && !text.trim()}
      <button type="button" class="btn small ghost" data-testid="diff-note-plain" onclick={onplain}>Add lines to chat</button>
    {/if}
    <span class="dne-grow"></span>
    <button type="button" class="btn small ghost" onclick={oncancel}>Cancel</button>
    <button type="button" class="btn small primary" title="Save ({keys})" data-testid="diff-note-save" disabled={!text.trim()} onclick={save}>Save note</button>
  </div>
</div>

<style>
  .dne {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin-block: var(--sp-1);
    margin-inline: var(--sp-2);
    padding: var(--sp-2);
    border: 1px solid var(--accent-line);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-family: var(--font-ui);
    white-space: normal;
  }
  .dne-where {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .dne textarea {
    inline-size: 100%;
    min-block-size: 3lh;
    max-block-size: 40vh;
    resize: vertical;
    padding: var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.5;
  }
  .dne textarea:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -1px;
  }
  .dne-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }
  .dne-grow {
    flex: 1;
  }
  @media (pointer: coarse) {
    .dne-actions :global(.btn) {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
