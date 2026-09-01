<script lang="ts">
  // Notes rail: one row per DOM annotation ("mark") on the currently-open
  // page, newest last. A thin renderer over `browser.annotations` — mutation
  // (edit comment / delete / send-to-session) goes through the `browser`
  // store, never the API module directly (matches the scheduledTasks /
  // runWithOtto convention). Live rows arrive via the `browser_annotation_added`
  // WS event, which the store applies in place — no polling here.

  import type { BrowserAnnotation } from '../../lib/api/types';
  import { browser } from '../../lib/stores/browser.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import SendToSession from './SendToSession.svelte';

  let { annotations }: { annotations: BrowserAnnotation[] } = $props();

  let editingId = $state<string | null>(null);
  let editText = $state('');

  function startEdit(a: BrowserAnnotation): void {
    editingId = a.id;
    editText = a.comment;
  }

  async function saveEdit(id: string): Promise<void> {
    try {
      await browser.updateAnnotationComment(id, editText);
      editingId = null;
    } catch (e) {
      toasts.error('Failed to update note', e instanceof Error ? e.message : undefined);
    }
  }

  function cancelEdit(): void {
    editingId = null;
  }

  async function remove(id: string): Promise<void> {
    try {
      await browser.deleteAnnotation(id);
    } catch (e) {
      toasts.error('Failed to delete mark', e instanceof Error ? e.message : undefined);
    }
  }
</script>

<div class="notes-rail">
  <div class="rail-head">
    <Icon name="note" size={13} />
    <span>Marks</span>
    <span class="count">{annotations.length}</span>
  </div>

  {#if annotations.length === 0}
    <p class="empty">No marks yet. Enable "Mark element" and click something in the reader.</p>
  {:else}
    <ul class="list">
      {#each annotations as a (a.id)}
        <li class="row">
          <span class="swatch" style:background={a.color || 'yellow'}></span>
          <div class="body">
            <p class="excerpt">{a.text}</p>
            {#if editingId === a.id}
              <div class="edit">
                <textarea
                  bind:value={editText}
                  placeholder="Add a note"
                  rows="2"
                  spellcheck="false"
                ></textarea>
                <div class="edit-actions">
                  <button class="btn" onclick={cancelEdit}>Cancel</button>
                  <button class="btn primary" onclick={() => saveEdit(a.id)}>Save mark</button>
                </div>
              </div>
            {:else if a.comment}
              <p class="comment">{a.comment}</p>
            {/if}
          </div>
          <div class="actions">
            {#if editingId !== a.id}
              <button class="icon-btn" title="Edit note" onclick={() => startEdit(a)}>
                <Icon name="edit" size={12} />
              </button>
            {/if}
            <SendToSession annotationId={a.id} />
            <button class="icon-btn" title="Delete mark" onclick={() => remove(a.id)}>
              <Icon name="trash" size={12} />
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .notes-rail {
    width: 260px;
    flex-shrink: 0;
    border-left: 1px solid var(--border);
    overflow-y: auto;
    padding: 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .rail-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--text-dim);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0 0.15rem;
  }
  .count {
    margin-inline-start: auto;
    color: var(--text-dim);
  }
  .empty {
    color: var(--text-dim);
    font-size: 0.8rem;
    padding: 0.5rem 0.15rem;
    line-height: 1.5;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .row {
    display: flex;
    gap: 0.4rem;
    padding: 0.5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-top: 0.35rem;
    flex-shrink: 0;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .excerpt {
    margin: 0;
    font-size: 0.82rem;
    color: var(--text);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  .comment {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-dim);
    white-space: pre-wrap;
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .edit textarea {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0.35rem 0.5rem;
    font: inherit;
    font-size: 0.8rem;
    resize: vertical;
  }
  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.3rem;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    flex-shrink: 0;
  }
  .icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 0.75rem;
    padding: 0.25rem 0.55rem;
    cursor: pointer;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--accent-contrast, #fff);
    border-color: var(--accent);
  }
</style>
