<script lang="ts">
  // Notes rail: one row per DOM annotation ("mark") on the currently-open
  // page, newest last. A thin renderer over `browser.annotations` — mutation
  // (edit comment / delete / send-to-session) goes through the `browser`
  // store, never the API module directly (matches the scheduledTasks /
  // runWithOtto convention). Live rows arrive via the `browser_annotation_added`
  // WS event, which the store applies in place — no polling here.

  import { tick } from 'svelte';
  import type { BrowserAnnotation } from '../../lib/api/types';
  import { focusAppContent } from '../../lib/announce.svelte';
  import { toastError } from '../../lib/toastError';
  import { browser } from '../../lib/stores/browser.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import SendToSession from './SendToSession.svelte';

  let {
    annotations,
    liveMode = false,
    onclose,
    onfocusmark,
  }: {
    annotations: BrowserAnnotation[];
    /** True when rendered beside a live tab (tailors the empty-state hint). */
    liveMode?: boolean;
    /** Optional close callback to collapse the rail. */
    onclose?: () => void;
    /** Jump to the marked element in the page. */
    onfocusmark?: (selector: string) => void;
  } = $props();

  let railEl = $state<HTMLDivElement | null>(null);
  let editingId = $state<string | null>(null);
  let editText = $state('');
  let savingEdit = $state(false);

  function startEdit(a: BrowserAnnotation): void {
    editingId = a.id;
    editText = a.comment;
  }

  async function saveEdit(id: string): Promise<void> {
    if (savingEdit) return;
    savingEdit = true;
    try {
      await browser.updateAnnotationComment(id, editText);
      editingId = null;
    } catch (e) {
      toastError('Couldn’t update note', e);
    } finally {
      savingEdit = false;
    }
  }

  function cancelEdit(): void {
    editingId = null;
  }

  async function remove(id: string): Promise<boolean> {
    const mark = annotations.find(a => a.id === id);
    if (!mark || !await confirmer.ask(`Delete this mark and its note? “${mark.text.slice(0, 180)}”`, { title: 'Delete mark', confirmLabel: 'Delete mark', danger: true })) return false;
    if (!annotations.some(a => a.id === id)) return false;
    try {
      await browser.deleteAnnotation(id);
      return true;
    } catch (e) {
      toastError('Couldn’t delete mark', e);
      return false;
    }
  }

  async function removeAndFocus(id: string, idx: number): Promise<void> {
    const deleted = await remove(id);
    if (!deleted) return;
    await tick();
    const btns = railEl?.querySelectorAll<HTMLButtonElement>('.row .actions button, .rail-close');
    if (btns && btns.length > 0) {
      btns[Math.min(idx, btns.length - 1)]?.focus();
    } else {
      void focusAppContent();
    }
  }
</script>

<div class="notes-rail" bind:this={railEl}>
  <div class="rail-head">
    <Icon name="note" size={13} />
    <span>Marks</span>
    <span class="count">{annotations.length}</span>
    {#if onclose}
      <button class="icon-btn rail-close" onclick={onclose} aria-label="Hide marks" title="Hide marks">
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>

  {#if annotations.length === 0}
    <p class="empty">
      {#if liveMode}
        No marks yet. Turn on the target picker in the toolbar, then click an element on the page to mark it.
      {:else}
        No marks yet. Turn on Mark passage above the page, then click a passage to mark it.
      {/if}
    </p>
  {:else}
    <ul class="list">
      {#each annotations as a, idx (a.id)}
        <li class="row">
          <span class="swatch" style:background={a.color || 'var(--warning)'}></span>
          <div class="body">
            {#if onfocusmark}
              <button
                type="button"
                class="excerpt excerpt-btn"
                onclick={() => onfocusmark?.(a.selector)}
                title="Jump to marked passage"
              >{a.text}</button>
            {:else}
              <p class="excerpt">{a.text}</p>
            {/if}
            {#if editingId === a.id}
              <div class="edit">
                <textarea dir="auto"
                  bind:value={editText}
                  placeholder="Check this claim against the spec"
                  aria-label="Note for this mark"
                  rows="2"
                  spellcheck="false"
                  onkeydown={(e) => {
                    if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
                    else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void saveEdit(a.id); }
                  }}
                ></textarea>
                <div class="edit-actions">
                  <button class="btn small" onclick={cancelEdit} disabled={savingEdit}>Cancel</button>
                  <button class="btn small primary" onclick={() => saveEdit(a.id)} disabled={savingEdit} title="Save note (⌘↩)">
                    {savingEdit ? 'Saving…' : 'Save note'}
                  </button>
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
            <button class="icon-btn" title="Delete mark" aria-label="Delete mark" onclick={() => void removeAndFocus(a.id, idx)}>
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
  .excerpt-btn {
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .excerpt-btn:hover {
    color: var(--accent-text);
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
  .rail-close {
    width: 22px;
    height: 22px;
    flex-shrink: 0;
  }
  /* Phone / short landscape: rail collapses to compact vertical cap. */
  @media (max-width: 640px), (max-height: 500px) {
    .notes-rail {
      width: auto;
      max-height: 34%;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
  }
  @container (max-width: 640px) {
    .notes-rail {
      width: auto;
      max-height: 40%;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
  }
  @media (pointer: coarse), (max-width: 640px) {
    .actions :global(.icon-btn) { min-width: 36px; min-height: 36px; }
  }
</style>
