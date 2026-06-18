<script lang="ts">
  // Questions tab — filters by status/category; inline edit, answer/discard,
  // delete, add question; multi-select + post to Jira/Confluence.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProductQuestion, NewQuestionReq, UpdateQuestionReq } from './types';

  // ── Load questions when tab becomes active ─────────────────────────────────
  $effect(() => {
    // Track selectedId reactively so we reload on story change.
    product.selectedId;
    if (product.selectedId) {
      void product.loadQuestions();
    }
  });

  // ── Filter state ───────────────────────────────────────────────────────────
  type StatusFilter = 'all' | 'open' | 'posted' | 'answered' | 'discarded';
  type CatFilter = 'all' | 'scope' | 'data' | 'ux' | 'edge-case' | 'dependency' | 'other';

  let filterStatus = $state<StatusFilter>('all');
  let filterCat = $state<CatFilter>('all');

  const STATUS_OPTIONS: { value: StatusFilter; label: string }[] = [
    { value: 'all', label: 'All statuses' },
    { value: 'open', label: 'Open' },
    { value: 'posted', label: 'Posted' },
    { value: 'answered', label: 'Answered' },
    { value: 'discarded', label: 'Discarded' },
  ];

  const CAT_OPTIONS: { value: CatFilter; label: string }[] = [
    { value: 'all', label: 'All categories' },
    { value: 'scope', label: 'Scope' },
    { value: 'data', label: 'Data' },
    { value: 'ux', label: 'UX' },
    { value: 'edge-case', label: 'Edge Case' },
    { value: 'dependency', label: 'Dependency' },
    { value: 'other', label: 'Other' },
  ];

  // ── Derived filtered list ──────────────────────────────────────────────────
  const filtered = $derived(
    product.questions.filter((q) => {
      if (filterStatus !== 'all' && q.status !== filterStatus) return false;
      if (filterCat !== 'all' && q.category !== filterCat) return false;
      return true;
    }),
  );

  // ── Multi-select ───────────────────────────────────────────────────────────
  let selectedIds = $state<Set<string>>(new Set());

  function toggleSelect(id: string): void {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedIds = next;
  }

  function toggleSelectAll(): void {
    if (selectedIds.size === filtered.length && filtered.length > 0) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(filtered.map((q) => q.id));
    }
  }

  // Clear selection when story changes.
  $effect(() => {
    product.selectedId;
    selectedIds = new Set();
  });

  // ── Inline edit state ──────────────────────────────────────────────────────
  // editingId = which question is in edit mode; null = none.
  let editingId = $state<string | null>(null);
  let editText = $state('');
  let editRationale = $state('');
  let editCategory = $state('');

  function startEdit(q: ProductQuestion): void {
    editingId = q.id;
    editText = q.text;
    editRationale = q.rationale ?? '';
    editCategory = q.category ?? '';
  }

  function cancelEdit(): void {
    editingId = null;
  }

  // ── Answer modal state ─────────────────────────────────────────────────────
  let answeringId = $state<string | null>(null);
  let answerText = $state('');

  function startAnswer(q: ProductQuestion): void {
    answeringId = q.id;
    answerText = q.answer ?? '';
  }

  function cancelAnswer(): void {
    answeringId = null;
  }

  // ── Add question form ──────────────────────────────────────────────────────
  let addOpen = $state(false);
  let newText = $state('');
  let newRationale = $state('');
  let newCategory = $state('scope');
  let addWorking = $state(false);

  function openAdd(): void {
    addOpen = true;
    newText = '';
    newRationale = '';
    newCategory = 'scope';
  }
  function closeAdd(): void {
    addOpen = false;
  }

  // ── Busy flags ─────────────────────────────────────────────────────────────
  let savingId = $state<string | null>(null);
  let postingIds = $state(false);
  let deletingId = $state<string | null>(null);

  // ── Actions ─────────────────────────────────────────────────────────────────

  async function saveEdit(): Promise<void> {
    if (!editingId) return;
    const req: UpdateQuestionReq = {
      text: editText.trim() || undefined,
      rationale: editRationale.trim() || undefined,
      category: editCategory.trim() || undefined,
    };
    savingId = editingId;
    try {
      await product.updateQuestion(editingId, req);
      editingId = null;
    } catch (e) {
      toasts.error('Could not save question', product.errMsg(e));
    } finally {
      savingId = null;
    }
  }

  async function saveAnswer(): Promise<void> {
    if (!answeringId) return;
    const req: UpdateQuestionReq = {
      answer: answerText.trim() || undefined,
      status: 'answered',
    };
    savingId = answeringId;
    try {
      await product.updateQuestion(answeringId, req);
      answeringId = null;
    } catch (e) {
      toasts.error('Could not save answer', product.errMsg(e));
    } finally {
      savingId = null;
    }
  }

  async function discard(q: ProductQuestion): Promise<void> {
    savingId = q.id;
    try {
      await product.updateQuestion(q.id, { status: 'discarded' });
    } catch (e) {
      toasts.error('Could not discard question', product.errMsg(e));
    } finally {
      savingId = null;
    }
  }

  async function deleteQ(q: ProductQuestion): Promise<void> {
    if (!(await confirmer.ask(`Delete question?\n\n"${q.text}"`, { title: 'Delete question', confirmLabel: 'Delete', danger: true }))) return;
    deletingId = q.id;
    try {
      await product.deleteQuestion(q.id);
      selectedIds = new Set([...selectedIds].filter((id) => id !== q.id));
    } catch (e) {
      toasts.error('Could not delete question', product.errMsg(e));
    } finally {
      deletingId = null;
    }
  }

  async function addQuestion(): Promise<void> {
    const text = newText.trim();
    if (!text) return;
    addWorking = true;
    try {
      const req: NewQuestionReq = {
        text,
        rationale: newRationale.trim() || null,
        category: newCategory || null,
      };
      await product.addQuestion(req);
      addOpen = false;
      toasts.success('Question added');
    } catch (e) {
      toasts.error('Could not add question', product.errMsg(e));
    } finally {
      addWorking = false;
    }
  }

  async function postSelected(): Promise<void> {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    postingIds = true;
    try {
      await product.postQuestions({ ids });
      selectedIds = new Set();
      toasts.success(`Posted ${ids.length} question${ids.length !== 1 ? 's' : ''}`);
    } catch (e) {
      toasts.error('Post failed', product.errMsg(e));
    } finally {
      postingIds = false;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function statusClass(status: string): string {
    switch (status) {
      case 'open': return 'pill-open';
      case 'posted': return 'pill-posted';
      case 'answered': return 'pill-answered';
      case 'discarded': return 'pill-discarded';
      default: return 'pill-open';
    }
  }

  function catClass(cat: string): string {
    switch (cat) {
      case 'scope': return 'cat-scope';
      case 'data': return 'cat-data';
      case 'ux': return 'cat-ux';
      case 'edge-case': return 'cat-edge';
      case 'dependency': return 'cat-dep';
      default: return 'cat-other';
    }
  }

  function fmtDate(s: string): string {
    try { return new Date(s).toLocaleDateString(); } catch { return s; }
  }
</script>

{#if !product.selectedId}
  <div class="muted">No story selected.</div>
{:else}
  <div class="qtab">

    <!-- ── Toolbar ──────────────────────────────────────────────────────────── -->
    <div class="toolbar">
      <!-- Filters -->
      <select
        class="filter-sel"
        bind:value={filterStatus}
        aria-label="Filter by status"
      >
        {#each STATUS_OPTIONS as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>

      <select
        class="filter-sel"
        bind:value={filterCat}
        aria-label="Filter by category"
      >
        {#each CAT_OPTIONS as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>

      <span class="grow"></span>

      <!-- Post selected -->
      {#if selectedIds.size > 0}
        <button
          class="action-btn accent-btn"
          onclick={postSelected}
          disabled={postingIds}
        >
          {postingIds ? 'Posting…' : `Post ${selectedIds.size} to Jira / Confluence`}
        </button>
      {/if}

      <!-- Add question -->
      <button class="action-btn" onclick={openAdd}>+ Add question</button>
    </div>

    <!-- ── Loading state ────────────────────────────────────────────────────── -->
    {#if product.loadingQuestions}
      <div class="muted">Loading questions…</div>
    {:else if filtered.length === 0}
      <div class="muted">
        {product.questions.length === 0
          ? 'No questions yet. Click "+ Add question" to create one.'
          : 'No questions match the current filters.'}
      </div>
    {:else}
      <!-- Select-all row -->
      <div class="select-all-row">
        <label class="cb-label" title="Select / deselect all visible">
          <input
            type="checkbox"
            checked={selectedIds.size === filtered.length && filtered.length > 0}
            indeterminate={selectedIds.size > 0 && selectedIds.size < filtered.length}
            onchange={toggleSelectAll}
          />
          <span class="sel-count">
            {selectedIds.size > 0 ? `${selectedIds.size} selected` : `${filtered.length} question${filtered.length !== 1 ? 's' : ''}`}
          </span>
        </label>
      </div>

      <!-- ── Question list ─────────────────────────────────────────────────── -->
      <div class="q-list">
        {#each filtered as q (q.id)}
          <div class="q-card" class:dimmed={q.status === 'discarded'}>

            <!-- Checkbox + header row -->
            <div class="q-header">
              <input
                type="checkbox"
                class="q-cb"
                checked={selectedIds.has(q.id)}
                onchange={() => toggleSelect(q.id)}
                aria-label="Select question"
              />

              {#if editingId === q.id}
                <!-- ── Edit mode ─────────────────────────────────────────── -->
                <div class="edit-form">
                  <textarea
                    class="edit-text"
                    bind:value={editText}
                    rows={3}
                    placeholder="Question text"
                  ></textarea>
                  <input
                    class="edit-input"
                    bind:value={editRationale}
                    placeholder="Rationale (optional)"
                  />
                  <select class="edit-sel" bind:value={editCategory}>
                    <option value="scope">Scope</option>
                    <option value="data">Data</option>
                    <option value="ux">UX</option>
                    <option value="edge-case">Edge Case</option>
                    <option value="dependency">Dependency</option>
                    <option value="other">Other</option>
                  </select>
                  <div class="edit-actions">
                    <button
                      class="action-btn accent-btn"
                      onclick={saveEdit}
                      disabled={savingId === q.id}
                    >
                      {savingId === q.id ? 'Saving…' : 'Save'}
                    </button>
                    <button class="action-btn" onclick={cancelEdit} disabled={savingId === q.id}>
                      Cancel
                    </button>
                  </div>
                </div>
              {:else}
                <!-- ── Read mode ─────────────────────────────────────────── -->
                <div class="q-body">
                  <div class="q-top">
                    <span class="q-text">{q.text}</span>
                    <div class="q-chips">
                      <span class="cat-chip {catClass(q.category)}">{q.category}</span>
                      <span class="status-pill {statusClass(q.status)}">{q.status}</span>
                    </div>
                  </div>
                  {#if q.rationale}
                    <div class="q-rationale">{q.rationale}</div>
                  {/if}
                  {#if q.answer}
                    <div class="q-answer">
                      <span class="answer-label">Answer:</span>
                      {q.answer}
                    </div>
                  {/if}
                  {#if q.posted_ref}
                    <div class="q-ref">Posted: <span class="mono">{q.posted_ref}</span></div>
                  {/if}
                  <div class="q-meta">{fmtDate(q.created_at)}</div>
                </div>
              {/if}

              <!-- Per-question action buttons (only in read mode) -->
              {#if editingId !== q.id && answeringId !== q.id}
                <div class="q-actions">
                  <button
                    class="qa-btn"
                    onclick={() => startEdit(q)}
                    disabled={savingId === q.id || deletingId === q.id}
                    title="Edit"
                  >Edit</button>
                  <button
                    class="qa-btn"
                    onclick={() => startAnswer(q)}
                    disabled={savingId === q.id || deletingId === q.id}
                    title="Answer / add context"
                  >Answer</button>
                  {#if q.status !== 'discarded'}
                    <button
                      class="qa-btn warn-btn"
                      onclick={() => discard(q)}
                      disabled={savingId === q.id || deletingId === q.id}
                      title="Discard"
                    >Discard</button>
                  {/if}
                  <button
                    class="qa-btn danger-btn"
                    onclick={() => deleteQ(q)}
                    disabled={savingId === q.id || deletingId === q.id}
                    title="Delete"
                  >
                    {deletingId === q.id ? '…' : 'Delete'}
                  </button>
                </div>
              {/if}
            </div>

            <!-- ── Answer panel (inline) ─────────────────────────────────── -->
            {#if answeringId === q.id}
              <div class="answer-panel">
                <textarea
                  class="edit-text"
                  bind:value={answerText}
                  rows={3}
                  placeholder="Write answer or additional context…"
                ></textarea>
                <div class="edit-actions">
                  <button
                    class="action-btn accent-btn"
                    onclick={saveAnswer}
                    disabled={savingId === q.id}
                  >
                    {savingId === q.id ? 'Saving…' : 'Save answer'}
                  </button>
                  <button class="action-btn" onclick={cancelAnswer} disabled={savingId === q.id}>
                    Cancel
                  </button>
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

    <!-- ── Add question dialog ───────────────────────────────────────────── -->
    {#if addOpen}
      <div class="modal-backdrop" role="presentation" onclick={closeAdd}>
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_no_noninteractive_element_interactions -->
        <div class="modal-box" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
          <div class="modal-head">
            <span class="modal-title">Add Question</span>
            <button class="modal-close" onclick={closeAdd} aria-label="Close">✕</button>
          </div>
          <div class="modal-body">
            <label class="form-label">Question <span class="req">*</span>
              <textarea
                class="form-textarea"
                bind:value={newText}
                rows={3}
                placeholder="What needs clarification?"
                disabled={addWorking}
              ></textarea>
            </label>
            <label class="form-label">Rationale
              <input
                class="form-input"
                bind:value={newRationale}
                placeholder="Why is this question important?"
                disabled={addWorking}
              />
            </label>
            <label class="form-label">Category
              <select class="form-select" bind:value={newCategory} disabled={addWorking}>
                <option value="scope">Scope</option>
                <option value="data">Data</option>
                <option value="ux">UX</option>
                <option value="edge-case">Edge Case</option>
                <option value="dependency">Dependency</option>
                <option value="other">Other</option>
              </select>
            </label>
          </div>
          <div class="modal-footer">
            <button
              class="action-btn accent-btn"
              onclick={addQuestion}
              disabled={addWorking || !newText.trim()}
            >
              {addWorking ? 'Adding…' : 'Add question'}
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

  .qtab {
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
    flex-wrap: wrap;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border);
  }
  .grow {
    flex: 1;
    min-width: 8px;
  }
  .filter-sel {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
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

  /* ── Select-all row ──────────────────────────────────────────────── */
  .select-all-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .cb-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    user-select: none;
    font-size: 12px;
    color: var(--text-dim);
  }
  .cb-label input {
    accent-color: var(--accent);
    cursor: pointer;
  }
  .sel-count {
    font-size: 11.5px;
  }

  /* ── Question list ───────────────────────────────────────────────── */
  .q-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .q-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    background: var(--surface-raised, var(--surface));
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 100ms;
  }
  .q-card:hover {
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  }
  .q-card.dimmed {
    opacity: 0.55;
  }

  .q-header {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .q-cb {
    flex-shrink: 0;
    margin-top: 3px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .q-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .q-top {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    flex-wrap: wrap;
  }
  .q-text {
    flex: 1;
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
    line-height: 1.4;
    min-width: 0;
  }
  .q-chips {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }
  .q-rationale {
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.45;
    font-style: italic;
  }
  .q-answer {
    font-size: 12.5px;
    color: var(--text);
    line-height: 1.45;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--accent) 7%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--radius-s) var(--radius-s) 0;
  }
  .answer-label {
    font-weight: 700;
    color: var(--accent);
    margin-right: 4px;
  }
  .q-ref {
    font-size: 11px;
    color: var(--text-dim);
  }
  .q-meta {
    font-size: 10.5px;
    color: var(--text-dim);
  }

  /* ── Category chips ──────────────────────────────────────────────── */
  .cat-chip {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 7px;
    border-radius: 999px;
  }
  .cat-scope   { background: color-mix(in srgb, #3b82f6 18%, transparent); color: #60a5fa; }
  .cat-data    { background: color-mix(in srgb, #8b5cf6 18%, transparent); color: #a78bfa; }
  .cat-ux      { background: color-mix(in srgb, #ec4899 18%, transparent); color: #f472b6; }
  .cat-edge    { background: color-mix(in srgb, #f59e0b 18%, transparent); color: #fbbf24; }
  .cat-dep     { background: color-mix(in srgb, #10b981 18%, transparent); color: #34d399; }
  .cat-other   { background: color-mix(in srgb, var(--text-dim) 15%, transparent); color: var(--text-dim); }

  /* ── Status pills ────────────────────────────────────────────────── */
  .status-pill {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: 999px;
  }
  .pill-open      { background: color-mix(in srgb, var(--status-working) 18%, transparent); color: var(--status-working); }
  .pill-posted    { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .pill-answered  { background: color-mix(in srgb, #10b981 18%, transparent); color: #34d399; }
  .pill-discarded { background: color-mix(in srgb, var(--text-dim) 15%, transparent); color: var(--text-dim); }

  /* ── Per-question action buttons ─────────────────────────────────── */
  .q-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .qa-btn {
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
  .qa-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .qa-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .warn-btn:hover:not(:disabled) {
    border-color: #f59e0b;
    color: #b45309;
    background: color-mix(in srgb, #f59e0b 10%, transparent);
  }
  .danger-btn:hover:not(:disabled) {
    border-color: #ef4444;
    color: #b91c1c;
    background: color-mix(in srgb, #ef4444 10%, transparent);
  }

  /* ── Inline edit form ────────────────────────────────────────────── */
  .edit-form {
    flex: 1;
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
  .edit-input {
    width: 100%;
    font-size: 12.5px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 5px 8px;
    font-family: inherit;
  }
  .edit-sel {
    font-size: 12.5px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 4px 8px;
    align-self: flex-start;
  }
  .edit-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* ── Answer panel ────────────────────────────────────────────────── */
  .answer-panel {
    padding-left: 28px; /* align under text, past checkbox */
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* ── Add question modal ──────────────────────────────────────────── */
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
  .form-select {
    font-size: 12.5px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 5px 8px;
    align-self: flex-start;
  }

  .mono {
    font-family: var(--font-mono, monospace);
    font-size: 10.5px;
  }
</style>
