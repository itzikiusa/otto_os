<script lang="ts">
  // Notes tab — list, add, edit, delete internal notes for the selected story.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProductNote, NewNoteReq } from './types';

  // ── Load notes when story is selected / changes ────────────────────────────
  $effect(() => {
    product.selectedId;
    if (product.selectedId) {
      void product.loadNotes();
    }
  });

  // ── Add note form ──────────────────────────────────────────────────────────
  let addOpen = $state(false);
  let newBody = $state('');
  let newSection = $state('');
  let addWorking = $state(false);

  function openAdd(): void {
    addOpen = true;
    newBody = '';
    newSection = '';
  }
  function closeAdd(): void {
    addOpen = false;
  }

  // ── Inline edit state ──────────────────────────────────────────────────────
  let editingId = $state<string | null>(null);
  let editBody = $state('');

  function startEdit(n: ProductNote): void {
    editingId = n.id;
    editBody = n.body;
  }
  function cancelEdit(): void {
    editingId = null;
  }

  // ── Busy flags ─────────────────────────────────────────────────────────────
  let savingId = $state<string | null>(null);
  let deletingId = $state<string | null>(null);

  // ── Actions ─────────────────────────────────────────────────────────────────

  async function addNote(): Promise<void> {
    const body = newBody.trim();
    if (!body) return;
    addWorking = true;
    try {
      const req: NewNoteReq = {
        body,
        section: newSection.trim() || null,
      };
      await product.addNote(req);
      addOpen = false;
      toasts.success('Note added');
    } catch (e) {
      toasts.error('Could not add note', product.errMsg(e));
    } finally {
      addWorking = false;
    }
  }

  async function saveEdit(nid: string): Promise<void> {
    const body = editBody.trim();
    if (!body) return;
    savingId = nid;
    try {
      await product.updateNote(nid, { body });
      editingId = null;
    } catch (e) {
      toasts.error('Could not save note', product.errMsg(e));
    } finally {
      savingId = null;
    }
  }

  async function deleteNote(n: ProductNote): Promise<void> {
    const preview = n.body.length > 80 ? n.body.slice(0, 80) + '…' : n.body;
    if (!(await confirmer.ask(`Delete this note?\n\n"${preview}"`, { title: 'Delete note', confirmLabel: 'Delete', danger: true }))) return;
    deletingId = n.id;
    try {
      await product.deleteNote(n.id);
    } catch (e) {
      toasts.error('Could not delete note', product.errMsg(e));
    } finally {
      deletingId = null;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function fmtDate(s: string): string {
    try { return new Date(s).toLocaleString(); } catch { return s; }
  }
</script>

{#if !product.selectedId}
  <div class="muted">No story selected.</div>
{:else}
  <div class="ntab">

    <!-- ── Toolbar ──────────────────────────────────────────────────────────── -->
    <div class="toolbar">
      <span class="grow"></span>
      <button class="action-btn accent-btn" onclick={openAdd}>+ Add note</button>
    </div>

    <!-- ── Notes list ───────────────────────────────────────────────────────── -->
    {#if product.loadingNotes}
      <div class="muted">Loading notes…</div>
    {:else if product.notes.length === 0}
      <div class="muted">No notes yet. Click "+ Add note" to capture a thought.</div>
    {:else}
      <div class="n-list">
        {#each product.notes as n (n.id)}
          <div class="n-card">

            <!-- Card header: section chip + meta + actions -->
            <div class="n-header">
              <div class="n-meta-row">
                {#if n.section}
                  <span class="section-chip">{n.section}</span>
                {/if}
                <span class="n-meta">{fmtDate(n.created_at)}</span>
                <span class="n-author dim">{n.author_id}</span>
              </div>
              {#if editingId !== n.id}
                <div class="n-actions">
                  <button
                    class="na-btn"
                    onclick={() => startEdit(n)}
                    disabled={savingId === n.id || deletingId === n.id}
                    title="Edit"
                  >Edit</button>
                  <button
                    class="na-btn danger-btn"
                    onclick={() => deleteNote(n)}
                    disabled={savingId === n.id || deletingId === n.id}
                    title="Delete"
                  >
                    {deletingId === n.id ? '…' : 'Delete'}
                  </button>
                </div>
              {/if}
            </div>

            <!-- Body or inline edit -->
            {#if editingId === n.id}
              <div class="edit-wrap">
                <textarea
                  class="edit-text"
                  bind:value={editBody}
                  rows={4}
                  placeholder="Note body (markdown)"
                  disabled={savingId === n.id}
                ></textarea>
                <div class="edit-actions">
                  <button
                    class="action-btn accent-btn"
                    onclick={() => saveEdit(n.id)}
                    disabled={savingId === n.id || !editBody.trim()}
                  >
                    {savingId === n.id ? 'Saving…' : 'Save'}
                  </button>
                  <button
                    class="action-btn"
                    onclick={cancelEdit}
                    disabled={savingId === n.id}
                  >Cancel</button>
                </div>
              </div>
            {:else}
              <div class="n-body md-body">{@html renderMarkdown(n.body)}</div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

    <!-- ── Add note modal ────────────────────────────────────────────────────── -->
    {#if addOpen}
      <div class="modal-backdrop" role="presentation" onclick={closeAdd}>
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_no_noninteractive_element_interactions -->
        <div class="modal-box" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
          <div class="modal-head">
            <span class="modal-title">Add Note</span>
            <button class="modal-close" onclick={closeAdd} aria-label="Close">✕</button>
          </div>
          <div class="modal-body">
            <label class="form-label">Note <span class="req">*</span>
              <textarea
                class="form-textarea"
                bind:value={newBody}
                rows={5}
                placeholder="Write a note… Markdown supported."
                disabled={addWorking}
              ></textarea>
            </label>
            <label class="form-label">Section (optional)
              <input
                class="form-input"
                bind:value={newSection}
                placeholder="e.g. scope, edge-cases, decisions"
                disabled={addWorking}
              />
            </label>
          </div>
          <div class="modal-footer">
            <button
              class="action-btn accent-btn"
              onclick={addNote}
              disabled={addWorking || !newBody.trim()}
            >
              {addWorking ? 'Adding…' : 'Add note'}
            </button>
            <button class="action-btn" onclick={closeAdd} disabled={addWorking}>Cancel</button>
          </div>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .muted {
    padding: 24px 0;
    font-size: 13px;
    color: var(--text-dim);
    font-style: italic;
  }

  .ntab {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 860px;
    width: 100%;
    position: relative;
  }

  /* ── Toolbar ─────────────────────────────────────────────────────── */
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border);
  }
  .grow {
    flex: 1;
  }

  /* ── Action buttons ──────────────────────────────────────────────── */
  .action-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
    transition: background 100ms, border-color 100ms, color 100ms;
  }
  .action-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .action-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .accent-btn {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    font-weight: 600;
  }
  .accent-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent);
  }

  /* ── Notes list ──────────────────────────────────────────────────── */
  .n-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .n-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    background: var(--surface-raised, var(--surface));
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 100ms;
  }
  .n-card:hover {
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
  }

  .n-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .n-meta-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    flex: 1;
  }
  .section-chip {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .n-meta {
    font-size: 11px;
    color: var(--text-dim);
  }
  .n-author {
    font-size: 10.5px;
    color: var(--text-dim);
    opacity: 0.7;
  }
  .dim {
    opacity: 0.6;
  }

  .n-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .na-btn {
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    white-space: nowrap;
    transition: background 80ms, color 80ms;
  }
  .na-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .na-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .danger-btn:hover:not(:disabled) {
    border-color: #ef4444;
    color: #b91c1c;
    background: color-mix(in srgb, #ef4444 10%, transparent);
  }

  /* ── Note body (markdown rendered) ───────────────────────────────── */
  .n-body.md-body {
    font-size: 13px;
    line-height: 1.65;
    color: var(--text);
  }
  .n-body :global(h1),
  .n-body :global(h2),
  .n-body :global(h3),
  .n-body :global(h4) {
    margin: 0.9em 0 0.3em;
    font-weight: 700;
    color: var(--text);
  }
  .n-body :global(h1) { font-size: 1.25em; }
  .n-body :global(h2) { font-size: 1.1em; }
  .n-body :global(h3) { font-size: 1em; }
  .n-body :global(p)  { margin: 0 0 0.6em; }
  .n-body :global(ul),
  .n-body :global(ol) {
    padding-left: 1.4em;
    margin: 0 0 0.6em;
  }
  .n-body :global(li) { margin-bottom: 0.2em; }
  .n-body :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.87em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    padding: 1px 5px;
    border-radius: 3px;
  }
  .n-body :global(pre) {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    overflow-x: auto;
    margin: 0 0 0.6em;
  }
  .n-body :global(pre code) { background: none; padding: 0; }
  .n-body :global(blockquote) {
    border-left: 3px solid var(--border);
    padding-left: 10px;
    color: var(--text-dim);
    margin: 0 0 0.6em;
    font-style: italic;
  }
  .n-body :global(a) { color: var(--accent); text-decoration: none; }
  .n-body :global(a:hover) { text-decoration: underline; }

  /* ── Inline edit ─────────────────────────────────────────────────── */
  .edit-wrap {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .edit-text {
    width: 100%;
    font-size: 12.5px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 6px 8px;
    resize: vertical;
    font-family: inherit;
    line-height: 1.4;
  }
  .edit-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* ── Add note modal ──────────────────────────────────────────────── */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, #000 45%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
  }
  .modal-box {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    width: 480px;
    max-width: 94vw;
    display: flex;
    flex-direction: column;
    box-shadow: 0 12px 32px color-mix(in srgb, #000 40%, transparent);
  }
  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 10px;
    border-bottom: 1px solid var(--border);
  }
  .modal-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
  }
  .modal-close {
    background: none;
    border: none;
    color: var(--text-dim);
    font-size: 14px;
    cursor: pointer;
    padding: 2px 6px;
    line-height: 1;
    border-radius: var(--radius-s);
  }
  .modal-close:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .modal-body {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .modal-footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px 14px;
    border-top: 1px solid var(--border);
  }
  .form-label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .req {
    color: #ef4444;
    font-weight: 700;
  }
  .form-textarea,
  .form-input {
    width: 100%;
    font-size: 12.5px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 6px 8px;
    font-family: inherit;
    resize: vertical;
    line-height: 1.4;
  }
  .form-input {
    resize: none;
  }
</style>
