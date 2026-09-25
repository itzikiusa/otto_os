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
    <p class="empty">
      No marks yet. Turn on Mark passage above the page, then click a passage to mark it.
    </p>
  {:else}
    <ul class="list">
      {#each annotations as a (a.id)}
        <li class="row">
          <span class="swatch" style:background={a.color || 'var(--warning)'}></span>
          <div class="body">
            <p class="excerpt">{a.text}</p>
            {#if editingId === a.id}
              <div class="edit">
                <textarea
                  bind:value={editText}
                  placeholder="Add a note"
                  aria-label="Note for this mark"
                  rows="2"
                  spellcheck="false"
                  onkeydown={(e) => {
                    if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
                    else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void saveEdit(a.id); }
                  }}
                ></textarea>
                <div class="edit-actions">
                  <button class="btn small" onclick={cancelEdit}>Cancel</button>
                  <button class="btn small primary" onclick={() => saveEdit(a.id)} title="Save note (⌘↩)">Save note</button>
                </div>
              </div>
            {:else if a.comment}
              <p class="comment">{a.comment}</p>
            {/if}
          </div>
          <div class="actions">
            {#if editingId !== a.id}
              <button class="icon-btn" title="Edit note" aria-label="Edit note" onclick={() => startEdit(a)}>
                <Icon name="edit" size={12} />
              </button>
            {/if}
            <SendToSession annotationId={a.id} />
            <button class="icon-btn" title="Delete mark" aria-label="Delete mark" onclick={() => remove(a.id)}>
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
    border-inline-start: 1px solid var(--border);
    overflow-y: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rail-head {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-weight: 600;
    padding: 0 2px;
  }
  .count {
    margin-inline-start: auto;
    color: var(--text-dim);
    font-weight: 400;
  }
  .empty {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    padding: 8px 2px;
    line-height: 1.5;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-top: 6px;
    flex-shrink: 0;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .excerpt {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  .comment {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: pre-wrap;
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .edit textarea {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
    font: inherit;
    font-size: var(--fs-s);
    resize: vertical;
  }
  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 4px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex-shrink: 0;
  }
  .actions :global(.icon-btn) {
    width: 24px;
    height: 24px;
  }
  /* Phone: the rail drops under the page (BrowserView stacks .body). */
  @media (max-width: 640px) {
    .notes-rail {
      width: auto;
      max-height: 40%;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
  }
</style>
