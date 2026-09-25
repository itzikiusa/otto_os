<script lang="ts">
  // Notes tab — list, add, edit, delete internal notes for the selected story.
  import { rel } from '../../lib/stores/now.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { confirmer } from '../../lib/confirm.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { ProductNote, NewNoteReq } from './types';

  // ── Load notes when story is selected / changes ────────────────────────────
  // A failed load shows inline with Retry — never as "No notes yet".
  let loadError = $state<string | null>(null);
  async function load(): Promise<void> {
    loadError = null;
    try {
      await product.loadNotes();
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }
  $effect(() => {
    product.selectedId;
    if (product.selectedId) {
      void load();
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
    try { return rel(s); } catch { return s; }
  }
</script>

{#if !product.selectedId}
  <div class="muted">No story selected.</div>
{:else}
  <div class="ntab">

    <!-- ── Toolbar ──────────────────────────────────────────────────────────── -->
    <div class="toolbar">
      <span class="grow"></span>
      <button class="btn small primary" onclick={openAdd}><Icon name="plus" size={12} /> Add note</button>
    </div>

    <!-- ── Notes list ───────────────────────────────────────────────────────── -->
    {#if (product.loadingNotes || loadError) && product.notes.length === 0}
      <LoadState what="notes" loading={product.loadingNotes} error={loadError} empty onretry={() => void load()} />
    {:else if product.notes.length === 0}
      <div class="muted">No notes yet. Use Add note to capture a thought.</div>
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
                <span class="n-meta" title={n.author_id ? `Author id: ${n.author_id}` : undefined}>{fmtDate(n.created_at)}</span>
              </div>
              {#if editingId !== n.id}
                <div class="n-actions">
                  <button
                    class="btn small ghost"
                    onclick={() => startEdit(n)}
                    disabled={savingId === n.id || deletingId === n.id}
                    title="Edit"
                  >Edit</button>
                  <button
                    class="btn small danger"
                    onclick={() => deleteNote(n)}
                    disabled={savingId === n.id || deletingId === n.id}
                    title="Delete"
                  >
                    {deletingId === n.id ? 'Deleting…' : 'Delete'}
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
                    class="btn small primary"
                    onclick={() => saveEdit(n.id)}
                    disabled={savingId === n.id || !editBody.trim()}
                  >
                    {savingId === n.id ? 'Saving…' : 'Save'}
                  </button>
                  <button
                    class="btn small ghost"
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
      <Modal title="Add note" width={480} onclose={closeAdd}>
        <div class="nt-add-body">
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
        {#snippet footer()}
          <button class="btn" onclick={closeAdd} disabled={addWorking}>Cancel</button>
          <button
            class="btn primary"
            onclick={addNote}
            disabled={addWorking || !newBody.trim()}
          >
            {addWorking ? 'Adding…' : 'Add note'}
          </button>
        {/snippet}
      </Modal>
    {/if}
  </div>
{/if}

<style>
  .muted {
    padding: 24px 0;
    font-size: var(--fs-m);
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
    background: var(--surface);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent-text);
  }
  .n-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  .n-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  /* ── Note body (markdown rendered) ───────────────────────────────── */
  .n-body.md-body {
    font-size: var(--fs-m);
    line-height: 1.65;
    color: var(--text);
  }
  .n-body :global(h1),
  .n-body :global(h2),
  .n-body :global(h3),
  .n-body :global(h4) {
    margin: 0.9em 0 0.3em;
    font-weight: 600;
    color: var(--text);
  }
  .n-body :global(h1) { font-size: 1.25em; }
  .n-body :global(h2) { font-size: 1.1em; }
  .n-body :global(h3) { font-size: 1em; }
  .n-body :global(p)  { margin: 0 0 0.6em; }
  .n-body :global(ul),
  .n-body :global(ol) {
    padding-inline-start: 1.4em;
    margin: 0 0 0.6em;
  }
  .n-body :global(li) { margin-bottom: 0.2em; }
  .n-body :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.87em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    padding: 1px 5px;
    border-radius: var(--radius-s);
  }
  .n-body :global(pre) {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    overflow-x: auto;
    margin: 0 0 0.6em;
  }
  .n-body :global(pre code) { background: none; padding: 0; }
  .n-body :global(blockquote) {
    border-inline-start: 3px solid var(--border);
    padding-inline-start: 10px;
    color: var(--text-dim);
    margin: 0 0 0.6em;
    font-style: italic;
  }
  .n-body :global(a) { color: var(--accent-text); text-decoration: none; }
  .n-body :global(a:hover) { text-decoration: underline; }

  /* ── Inline edit ─────────────────────────────────────────────────── */
  .edit-wrap {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .edit-text {
    width: 100%;
    font-size: var(--fs-s);
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
  .nt-add-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .form-label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .req {
    color: var(--danger);
    font-weight: 600;
  }
  .form-textarea,
  .form-input {
    width: 100%;
    font-size: var(--fs-s);
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
