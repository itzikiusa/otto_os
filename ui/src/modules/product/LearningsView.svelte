<script lang="ts">
  // LearningsView — workspace-scoped knowledge base.
  // Patterns to follow (kind='pattern') vs Cases to avoid (kind='avoid').
  // Inactive (AI-suggested) learnings surface with an Accept button.
  import Icon from '../../lib/components/Icon.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { ProductLearning, NewLearningReq, UpdateLearningReq } from './types';

  // ── Filter (the view's own segmented control — no side rail) ─────────────────
  type LearningFilter = 'all' | 'pattern' | 'avoid';
  let filter = $state<LearningFilter>('all');
  const FILTERS: { value: LearningFilter; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'pattern', label: 'Patterns to follow' },
    { value: 'avoid', label: 'Cases to avoid' },
  ];

  // Ref shape parsed from refs_json.
  interface LearningRef {
    type: string;
    ref: string;
    label?: string;
  }

  // ── Local state ───────────────────────────────────────────────────────────────
  let loaded = $state(false);

  // Add-form
  let addOpen = $state(false);
  let addKind = $state<'pattern' | 'avoid'>('pattern');
  let addTitle = $state('');
  let addBody = $state('');
  let addTags = $state('');
  let addRefs = $state('');
  let adding = $state(false);

  // Edit state: id → draft fields
  let editingId = $state<string | null>(null);
  let editKind = $state<'pattern' | 'avoid'>('pattern');
  let editTitle = $state('');
  let editBody = $state('');
  let editTags = $state('');
  let editRefs = $state('');
  let editSaving = $state(false);

  // Per-card busy flags
  let acceptingId = $state<string | null>(null);
  let deletingId = $state<string | null>(null);
  let togglingId = $state<string | null>(null);


  // Load all learnings on mount / when workspace changes.
  $effect(() => {
    if (!loaded) {
      void loadAll();
    }
  });

  /** A failed load — inline with Retry, never as "No patterns yet". */
  let loadError = $state<string | null>(null);
  async function loadAll(): Promise<void> {
    loadError = null;
    try {
      await product.loadLearnings(); // load ALL (active + inactive)
      loaded = true;
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }

  // ── Derived ───────────────────────────────────────────────────────────────────

  const patterns = $derived(product.learnings.filter((l) => l.kind === 'pattern'));
  const avoids = $derived(product.learnings.filter((l) => l.kind === 'avoid'));

  const showPatterns = $derived(filter === 'all' || filter === 'pattern');
  const showAvoids = $derived(filter === 'all' || filter === 'avoid');

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function parseTags(tags: string): string[] {
    return tags
      ? tags
          .split(',')
          .map((t) => t.trim())
          .filter(Boolean)
      : [];
  }

  function parseRefs(refsJson: string): LearningRef[] {
    if (!refsJson) return [];
    try {
      const parsed = JSON.parse(refsJson);
      if (Array.isArray(parsed)) return parsed as LearningRef[];
    } catch {
      // ignore malformed
    }
    return [];
  }

  function refHref(r: LearningRef): string | null {
    if (r.type === 'url') return r.ref;
    if (r.type === 'jira') return r.ref.startsWith('http') ? r.ref : null;
    if (r.type === 'confluence') return r.ref.startsWith('http') ? r.ref : null;
    return null;
  }

  function refClass(type: string): string {
    switch (type) {
      case 'jira': return 'ref-jira';
      case 'confluence': return 'ref-confluence';
      case 'url': return 'ref-url';
      default: return 'ref-other';
    }
  }

  // ── CRUD actions ──────────────────────────────────────────────────────────────

  async function addLearning(): Promise<void> {
    if (!addTitle.trim() || adding) return;
    adding = true;
    try {
      const refsArr = addRefs.trim()
        ? (JSON.parse(addRefs) as unknown)
        : undefined;
      const req: NewLearningReq = {
        kind: addKind,
        title: addTitle.trim(),
        body: addBody.trim(),
        tags: addTags.trim() || null,
        refs: refsArr,
      };
      await product.addLearning(req);
      toasts.success('Learning added');
      addOpen = false;
      addTitle = '';
      addBody = '';
      addTags = '';
      addRefs = '';
      addKind = 'pattern';
    } catch (e) {
      toasts.error('Could not add learning', product.errMsg(e));
    } finally {
      adding = false;
    }
  }

  function startEdit(l: ProductLearning): void {
    editingId = l.id;
    editKind = l.kind as 'pattern' | 'avoid';
    editTitle = l.title;
    editBody = l.body;
    editTags = l.tags ?? '';
    // Pretty-print refs for editing
    try {
      const parsed = JSON.parse(l.refs_json || '[]');
      editRefs = JSON.stringify(parsed, null, 2);
    } catch {
      editRefs = l.refs_json ?? '';
    }
  }

  function cancelEdit(): void {
    editingId = null;
  }

  async function saveEdit(id: string): Promise<void> {
    if (editSaving) return;
    editSaving = true;
    try {
      let refsVal: unknown = undefined;
      if (editRefs.trim()) {
        refsVal = JSON.parse(editRefs) as unknown;
      }
      const req: UpdateLearningReq = {
        kind: editKind,
        title: editTitle.trim() || null,
        body: editBody.trim() || null,
        tags: editTags.trim() || null,
        refs: refsVal,
      };
      await product.updateLearning(id, req);
      toasts.success('Learning updated');
      editingId = null;
    } catch (e) {
      toasts.error('Could not save learning', product.errMsg(e));
    } finally {
      editSaving = false;
    }
  }

  async function toggleActive(l: ProductLearning): Promise<void> {
    if (togglingId) return;
    togglingId = l.id;
    try {
      await product.updateLearning(l.id, { active: !l.active });
    } catch (e) {
      toasts.error('Could not toggle active', product.errMsg(e));
    } finally {
      togglingId = null;
    }
  }

  async function accept(id: string): Promise<void> {
    if (acceptingId) return;
    acceptingId = id;
    try {
      await product.acceptLearning(id);
      toasts.success('Learning accepted — now active');
    } catch (e) {
      toasts.error('Could not accept learning', product.errMsg(e));
    } finally {
      acceptingId = null;
    }
  }

  /** Delete one learning — confirmed in the shared dialog, naming it (the old
   *  inline bar sat at the top of the page, far from the card it deleted). */
  async function askDelete(id: string): Promise<void> {
    if (deletingId) return;
    const l = product.learnings.find((x) => x.id === id);
    const ok = await confirmer.ask(
      `Delete the learning “${l?.title ?? 'untitled'}”? Future analyses stop using it. This can't be undone.`,
      { title: 'Delete learning', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    deletingId = id;
    try {
      await product.deleteLearning(deletingId);
      toasts.info('Learning deleted');
    } catch (e) {
      toasts.error('Could not delete learning', product.errMsg(e));
    } finally {
      deletingId = null;
    }
  }
</script>

<div class="learnings-view">
  <!-- ── Header ────────────────────────────────────────────────────────────── -->
  <div class="lv-header">
    <div class="lv-title-row">
      <div class="segmented" role="group" aria-label="Show learnings">
        {#each FILTERS as f (f.value)}
          <button class:active={filter === f.value} aria-pressed={filter === f.value} onclick={() => (filter = f.value)}>{f.label}</button>
        {/each}
      </div>
      <span class="lv-count">{product.learnings.length} learning{product.learnings.length !== 1 ? 's' : ''}</span>
      <span class="spacer"></span>
      <button
        class="btn small"
        onclick={() => (addOpen = !addOpen)}
        title="Add a new learning"
      >
        <Icon name="plus" size={13} />
        Add learning
      </button>
      <button
        class="icon-btn"
        onclick={loadAll}
        disabled={product.loadingLearnings}
        title="Refresh"
        aria-label="Refresh learnings"
      >
        <Icon name="refresh" size={13} />
      </button>
    </div>

  </div>

  <!-- ── Add form ───────────────────────────────────────────────────────────── -->
  {#if addOpen}
    <div class="form-card">
      <div class="form-head">Add learning</div>
      <div class="form-row">
        <label class="field-label" for="add-kind">Kind</label>
        <select id="add-kind" class="mini-select" bind:value={addKind}>
          <option value="pattern">Pattern to follow</option>
          <option value="avoid">Case to avoid</option>
        </select>
      </div>
      <div class="form-row col">
        <label class="field-label" for="add-title">Title *</label>
        <input id="add-title" class="field-input" type="text" bind:value={addTitle} placeholder="Short title" />
      </div>
      <div class="form-row col">
        <label class="field-label" for="add-body">Body</label>
        <textarea id="add-body" class="field-textarea" bind:value={addBody} placeholder="Detailed description (markdown)"></textarea>
      </div>
      <div class="form-row col">
        <label class="field-label" for="add-tags">Tags <span class="hint">(comma-separated)</span></label>
        <input id="add-tags" class="field-input" type="text" bind:value={addTags} placeholder="api, auth, performance" />
      </div>
      <div class="form-row col">
        <label class="field-label" for="add-refs">Refs <span class="hint">(JSON array of &#123;type,ref,label&#125;)</span></label>
        <textarea id="add-refs" class="field-textarea mono" bind:value={addRefs} rows={3} placeholder="JSON array, e.g. type/ref/label objects"></textarea>
      </div>
      <div class="form-actions">
        <button class="btn primary" onclick={addLearning} disabled={adding || !addTitle.trim()}>
          {adding ? 'Adding…' : 'Add'}
        </button>
        <button class="btn ghost" onclick={() => (addOpen = false)}>Cancel</button>
      </div>
    </div>
  {/if}

  <!-- ── First load / failed load (nothing to show yet) ────────────────────── -->
  {#if (loadError || (product.loadingLearnings && !loaded)) && product.learnings.length === 0}
    <LoadState what="learnings" loading={product.loadingLearnings} error={loadError} empty onretry={() => void loadAll()} />
  {:else}
  <!-- ── Two-column layout ─────────────────────────────────────────────────── -->
  <div class="two-col" class:single-col={!showPatterns || !showAvoids}>

    <!-- Patterns to follow -->
    {#if showPatterns}
    <div class="col-section">
      <div class="col-header pattern-header">
        <Icon name="check" size={13} />
        <span class="col-title">Patterns to follow</span>
        <span class="col-count">{patterns.length}</span>
      </div>

      {#if patterns.length === 0}
        <div class="empty-col">No patterns yet.</div>
      {:else}
        {#each patterns as l (l.id)}
          {@const refs = parseRefs(l.refs_json)}
          {@const tags = parseTags(l.tags)}
          <div class="learning-card" class:inactive={!l.active}>
            {#if !l.active}
              <div class="suggested-banner">
                <Icon name="zap" size={11} />
                AI-suggested · pending acceptance
              </div>
            {/if}

            {#if editingId === l.id}
              <!-- Edit form inline -->
              <div class="edit-form">
                <div class="form-row">
                  <label class="field-label" for="ek-{l.id}">Kind</label>
                  <select id="ek-{l.id}" class="mini-select" bind:value={editKind}>
                    <option value="pattern">Pattern to follow</option>
                    <option value="avoid">Case to avoid</option>
                  </select>
                </div>
                <div class="form-row col">
                  <label class="field-label" for="et-{l.id}">Title</label>
                  <input id="et-{l.id}" class="field-input" type="text" bind:value={editTitle} />
                </div>
                <div class="form-row col">
                  <label class="field-label" for="eb-{l.id}">Body</label>
                  <textarea id="eb-{l.id}" class="field-textarea" bind:value={editBody}></textarea>
                </div>
                <div class="form-row col">
                  <label class="field-label" for="etg-{l.id}">Tags</label>
                  <input id="etg-{l.id}" class="field-input" type="text" bind:value={editTags} />
                </div>
                <div class="form-row col">
                  <label class="field-label" for="er-{l.id}">Refs (JSON)</label>
                  <textarea id="er-{l.id}" class="field-textarea mono" rows={3} bind:value={editRefs}></textarea>
                </div>
                <div class="form-actions">
                  <button class="btn primary small" onclick={() => saveEdit(l.id)} disabled={editSaving}>{editSaving ? 'Saving…' : 'Save'}</button>
                  <button class="btn ghost small" onclick={cancelEdit}>Cancel</button>
                </div>
              </div>
            {:else}
              <!-- Card view -->
              <div class="card-header">
                <span class="card-title">{l.title}</span>
                <div class="card-actions">
                  {#if !l.active}
                    <button
                      class="btn accept-btn small"
                      onclick={() => accept(l.id)}
                      disabled={acceptingId === l.id}
                      title="Accept this suggested learning"
                    >
                      {acceptingId === l.id ? 'Accepting…' : 'Accept'}
                    </button>
                  {/if}
                  <label class="toggle-wrap" title={l.active ? 'Active — click to deactivate' : 'Inactive — click to activate'}>
                    <input
                      type="checkbox"
                      class="toggle-input"
                      checked={l.active}
                      disabled={togglingId === l.id}
                      onchange={() => toggleActive(l)}
                    />
                    <span class="toggle-label">{l.active ? 'Active' : 'Inactive'}</span>
                  </label>
                  <button class="icon-act" onclick={() => startEdit(l)} title="Edit" aria-label="Edit">
                    <Icon name="edit" size={12} />
                  </button>
                  <button class="icon-act danger" onclick={() => askDelete(l.id)} disabled={deletingId === l.id} title="Delete" aria-label="Delete">
                    <Icon name="trash" size={12} />
                  </button>
                </div>
              </div>

              {#if l.body}
                <div class="card-body md-body">{@html renderMarkdown(l.body)}</div>
              {/if}

              {#if tags.length > 0}
                <div class="tags-row">
                  {#each tags as tag}
                    <span class="tag-chip"><Icon name="tag" size={10} />{tag}</span>
                  {/each}
                </div>
              {/if}

              {#if refs.length > 0}
                <div class="refs-row">
                  {#each refs as r}
                    {@const href = refHref(r)}
                    {#if href}
                      <a class="ref-badge {refClass(r.type)}" href={href} target="_blank" rel="noopener noreferrer">
                        <Icon name="link" size={10} />
                        {r.label || r.ref}
                      </a>
                    {:else}
                      <span class="ref-badge {refClass(r.type)}">
                        {r.label || r.ref}
                      </span>
                    {/if}
                  {/each}
                </div>
              {/if}
            {/if}
          </div>
        {/each}
      {/if}
    </div>
    {/if}

    <!-- Cases to avoid -->
    {#if showAvoids}
    <div class="col-section">
      <div class="col-header avoid-header">
        <Icon name="x" size={13} />
        <span class="col-title">Cases to avoid</span>
        <span class="col-count">{avoids.length}</span>
      </div>

      {#if avoids.length === 0}
        <div class="empty-col">No cases to avoid yet.</div>
      {:else}
        {#each avoids as l (l.id)}
          {@const refs = parseRefs(l.refs_json)}
          {@const tags = parseTags(l.tags)}
          <div class="learning-card avoid-card" class:inactive={!l.active}>
            {#if !l.active}
              <div class="suggested-banner">
                <Icon name="zap" size={11} />
                AI-suggested · pending acceptance
              </div>
            {/if}

            {#if editingId === l.id}
              <!-- Edit form inline -->
              <div class="edit-form">
                <div class="form-row">
                  <label class="field-label" for="ek2-{l.id}">Kind</label>
                  <select id="ek2-{l.id}" class="mini-select" bind:value={editKind}>
                    <option value="pattern">Pattern to follow</option>
                    <option value="avoid">Case to avoid</option>
                  </select>
                </div>
                <div class="form-row col">
                  <label class="field-label" for="et2-{l.id}">Title</label>
                  <input id="et2-{l.id}" class="field-input" type="text" bind:value={editTitle} />
                </div>
                <div class="form-row col">
                  <label class="field-label" for="eb2-{l.id}">Body</label>
                  <textarea id="eb2-{l.id}" class="field-textarea" bind:value={editBody}></textarea>
                </div>
                <div class="form-row col">
                  <label class="field-label" for="etg2-{l.id}">Tags</label>
                  <input id="etg2-{l.id}" class="field-input" type="text" bind:value={editTags} />
                </div>
                <div class="form-row col">
                  <label class="field-label" for="er2-{l.id}">Refs (JSON)</label>
                  <textarea id="er2-{l.id}" class="field-textarea mono" rows={3} bind:value={editRefs}></textarea>
                </div>
                <div class="form-actions">
                  <button class="btn primary small" onclick={() => saveEdit(l.id)} disabled={editSaving}>{editSaving ? 'Saving…' : 'Save'}</button>
                  <button class="btn ghost small" onclick={cancelEdit}>Cancel</button>
                </div>
              </div>
            {:else}
              <!-- Card view -->
              <div class="card-header">
                <span class="card-title">{l.title}</span>
                <div class="card-actions">
                  {#if !l.active}
                    <button
                      class="btn accept-btn small"
                      onclick={() => accept(l.id)}
                      disabled={acceptingId === l.id}
                      title="Accept this suggested learning"
                    >
                      {acceptingId === l.id ? 'Accepting…' : 'Accept'}
                    </button>
                  {/if}
                  <label class="toggle-wrap" title={l.active ? 'Active — click to deactivate' : 'Inactive — click to activate'}>
                    <input
                      type="checkbox"
                      class="toggle-input"
                      checked={l.active}
                      disabled={togglingId === l.id}
                      onchange={() => toggleActive(l)}
                    />
                    <span class="toggle-label">{l.active ? 'Active' : 'Inactive'}</span>
                  </label>
                  <button class="icon-act" onclick={() => startEdit(l)} title="Edit" aria-label="Edit">
                    <Icon name="edit" size={12} />
                  </button>
                  <button class="icon-act danger" onclick={() => askDelete(l.id)} disabled={deletingId === l.id} title="Delete" aria-label="Delete">
                    <Icon name="trash" size={12} />
                  </button>
                </div>
              </div>

              {#if l.body}
                <div class="card-body md-body">{@html renderMarkdown(l.body)}</div>
              {/if}

              {#if tags.length > 0}
                <div class="tags-row">
                  {#each tags as tag}
                    <span class="tag-chip"><Icon name="tag" size={10} />{tag}</span>
                  {/each}
                </div>
              {/if}

              {#if refs.length > 0}
                <div class="refs-row">
                  {#each refs as r}
                    {@const href = refHref(r)}
                    {#if href}
                      <a class="ref-badge {refClass(r.type)}" href={href} target="_blank" rel="noopener noreferrer">
                        <Icon name="link" size={10} />
                        {r.label || r.ref}
                      </a>
                    {:else}
                      <span class="ref-badge {refClass(r.type)}">
                        {r.label || r.ref}
                      </span>
                    {/if}
                  {/each}
                </div>
              {/if}
            {/if}
          </div>
        {/each}
      {/if}
    </div>
    {/if}
  </div>
  {/if}
</div>

<style>
  .learnings-view {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: 100%;
    min-height: 0;
  }

  /* Header */
  .lv-header {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex-shrink: 0;
  }
  .lv-title-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .lv-title {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    color: var(--text);
  }
  .lv-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    padding: 2px 8px;
    border-radius: 999px;
  }
  .spacer { flex: 1; }
  .dim-sm { font-size: var(--fs-xs); color: var(--text-dim); }



  .accept-btn {
    border-color: var(--success);
    color: var(--success);
    background: color-mix(in srgb, var(--success) 12%, transparent);
    font-weight: 600;
  }
  .accept-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--success) 22%, transparent);
  }

  /* Confirm bar */
  .form-head {
    font-size: var(--fs-s);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .form-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .form-row.col {
    flex-direction: column;
    align-items: flex-start;
  }
  .field-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .hint { font-weight: 400; text-transform: none; font-size: var(--fs-xs); }
  .field-input {
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 5px 9px;
    box-sizing: border-box;
  }
  .field-input:focus { outline: none; border-color: var(--accent); }
  .field-textarea {
    width: 100%;
    min-height: 70px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 6px 9px;
    resize: vertical;
    box-sizing: border-box;
    font-family: inherit;
    line-height: 1.5;
  }
  .field-textarea:focus { outline: none; border-color: var(--accent); }
  .field-textarea.mono { font-family: var(--font-mono, monospace); font-size: var(--fs-xs); }
  .mini-select {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 3px 7px;
  }
  .form-actions {
    display: flex;
    gap: 8px;
  }

  /* Two-column layout */
  .two-col {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    align-items: start;
  }
  .two-col.single-col {
    grid-template-columns: 1fr;
  }
  @media (max-width: 720px) {
    .two-col { grid-template-columns: 1fr; }
  }

  /* Column sections */
  .col-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .col-header {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 8px 12px;
    border-radius: var(--radius-s);
    margin-bottom: 2px;
  }
  .pattern-header {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
    color: var(--accent-text);
  }
  .avoid-header {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 25%, transparent);
    color: var(--danger);
  }
  .col-title {
    font-size: var(--fs-s);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .col-count {
    font-size: var(--fs-xs);
    opacity: 0.7;
    background: color-mix(in srgb, currentColor 15%, transparent);
    padding: 1px 7px;
    border-radius: 999px;
  }
  .empty-col {
    font-size: var(--fs-s);
    color: var(--text-dim);
    padding: 16px 12px;
    text-align: center;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    font-style: italic;
  }

  /* Learning card */
  .learning-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 7px;
    transition: opacity 150ms;
  }
  .learning-card.inactive {
    opacity: 0.72;
    border-style: dashed;
    background: color-mix(in srgb, var(--text-dim) 4%, var(--surface));
  }
  .avoid-card {
    border-color: color-mix(in srgb, var(--danger) 22%, var(--border));
  }

  /* AI-suggested banner */
  .suggested-banner {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--warning) 30%, transparent);
    border-radius: var(--radius-s);
    padding: 3px 8px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* Card header */
  .card-header {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .card-title {
    flex: 1;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    line-height: 1.35;
  }
  .card-actions {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }

  /* Toggle */
  .toggle-wrap {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
    user-select: none;
  }
  .toggle-input {
    accent-color: var(--accent);
    cursor: pointer;
  }
  .toggle-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }

  /* Icon action buttons */
  .icon-act {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    transition: background 100ms, color 100ms;
  }
  .icon-act:hover { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text); }
  .icon-act.danger:hover { background: color-mix(in srgb, var(--danger) 15%, transparent); color: var(--danger); }
  .icon-act:disabled { opacity: 0.4; cursor: not-allowed; }

  /* Card body (markdown) */
  .card-body {
    font-size: var(--fs-s);
    line-height: 1.55;
    color: var(--text-dim);
  }

  /* Tags */
  .tags-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    padding: 2px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }

  /* Refs */
  .refs-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .ref-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 500;
    text-decoration: none;
    border: 1px solid transparent;
    transition: opacity 100ms;
  }
  .ref-badge:hover { opacity: 0.8; }
  /* Sources are kinds, not states: neutral chips (components.md §4 — no
     per-source hues); links read as links in --accent-text. */
  .ref-jira,
  .ref-confluence,
  .ref-url {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--accent-text);
  }
  .ref-other {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    border-color: var(--border);
    color: var(--text-dim);
  }

  /* Edit form (inline) */
  .edit-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* Markdown */
  .md-body :global(h1),
  .md-body :global(h2),
  .md-body :global(h3) {
    margin: 0.8em 0 0.3em;
    font-weight: 600;
    color: var(--text);
  }
  .md-body :global(p) { margin: 0 0 0.5em; }
  .md-body :global(ul),
  .md-body :global(ol) { padding-inline-start: 1.4em; margin: 0 0 0.5em; }
  .md-body :global(li) { margin-bottom: 0.15em; }
  .md-body :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.88em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    padding: 1px 4px;
    border-radius: var(--radius-s);
  }
  .md-body :global(a) { color: var(--accent-text); text-decoration: none; }
  .md-body :global(a:hover) { text-decoration: underline; }

  /* Responsive */
  @media (max-width: 600px) {
    .lv-title-row { flex-wrap: wrap; }
    .card-header { flex-direction: column; }
  }
</style>
