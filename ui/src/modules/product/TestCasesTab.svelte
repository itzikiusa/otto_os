<script lang="ts">
  // Test Cases tab — generate test cases, view them grouped by category, allow
  // per-case approve/request-changes/edit, bulk-select + bulk-approve, drag-to-
  // reorder (persists order_idx), approve a run (triggers skill learning), and
  // publish to Confluence.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type {
    ProductTestcaseRunDetail,
    ProductTestcaseRun,
    ProductTestcase,
    TestcaseSteps,
  } from './types';

  const PROVIDERS = ['claude', 'openai'] as const;
  const CATEGORIES = ['happy', 'validation', 'error', 'edge'] as const;
  type Category = (typeof CATEGORIES)[number];

  const CATEGORY_LABELS: Record<Category, string> = {
    happy: 'Happy Path',
    validation: 'Validation',
    error: 'Error Cases',
    edge: 'Edge Cases',
  };

  // Priority ordering for display within category (used as tiebreaker when
  // within a category group but no explicit order_idx reorder has been applied).
  const PRIORITY_ORDER: Record<string, number> = { high: 0, medium: 1, low: 2 };

  // ── Local UI state ──────────────────────────────────────────────────────────
  let provider = $state<string>('claude');
  let generating = $state(false);
  let approvingRun = $state(false);
  let bulkApproving = $state(false);
  let publishingRun = $state(false);
  let showPublishForm = $state(false);
  let publishSpaceKey = $state('');
  let publishParentId = $state('');

  // ── Bulk-select state ────────────────────────────────────────────────────────
  // Set of testcase ids that are currently checked.
  let selected = $state<Set<string>>(new Set());

  function toggleSelect(id: string): void {
    const next = new Set(selected);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    selected = next;
  }

  function selectAll(): void {
    selected = new Set(activeCases.map((c) => c.id));
  }

  function clearSelection(): void {
    selected = new Set();
  }

  // ── Drag-to-reorder state ────────────────────────────────────────────────────
  // Flat ordered list (mirrors activeCases but can be locally rearranged before
  // persisting).  Populated/reset whenever activeCases changes.
  let orderedIds = $state<string[]>([]);

  // Populate orderedIds from activeCases whenever the active run changes.
  // We keep a separate reactive list so local drags don't fight with store
  // refreshes until the user drops.
  $effect(() => {
    orderedIds = activeCases.map((c) => c.id);
  });

  let dragSrcId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);
  let savingOrder = $state(false);

  function onDragStart(id: string): void {
    dragSrcId = id;
  }

  function onDragOver(id: string, e: DragEvent): void {
    e.preventDefault();
    dragOverId = id;
  }

  function onDragLeave(): void {
    dragOverId = null;
  }

  function onDrop(targetId: string): void {
    if (!dragSrcId || dragSrcId === targetId) {
      dragSrcId = null;
      dragOverId = null;
      return;
    }
    // Reorder: move dragSrcId to be just before targetId.
    const next = orderedIds.filter((id) => id !== dragSrcId);
    const idx = next.indexOf(targetId);
    if (idx === -1) {
      next.push(dragSrcId);
    } else {
      next.splice(idx, 0, dragSrcId);
    }
    orderedIds = next;
    dragSrcId = null;
    dragOverId = null;
    // Persist immediately after drop.
    void persistOrder();
  }

  function onDragEnd(): void {
    dragSrcId = null;
    dragOverId = null;
  }

  async function persistOrder(): Promise<void> {
    if (!activeRun || savingOrder) return;
    savingOrder = true;
    try {
      await product.reorderTestcases(activeRun.id, orderedIds);
    } catch (e) {
      toasts.error('Could not save order', product.errMsg(e));
    } finally {
      savingOrder = false;
    }
  }

  // Which test-case run we're viewing (most recent by default).
  let activeRunId = $state<string | null>(null);

  // Per-case action state, keyed by testcase id.
  interface CaseAction {
    mode: 'idle' | 'approve' | 'changes' | 'edit';
    reviewNote: string;
    editTitle: string;
    editCategory: string;
    editPriority: string;
    editPreconditions: string; // newline-separated
    editSteps: string; // newline-separated
    editExpected: string;
    busy: boolean;
  }
  let caseActions = $state<Record<string, CaseAction>>({});

  // ── Polling ──────────────────────────────────────────────────────────────────
  let pollTimer = $state<ReturnType<typeof setInterval> | null>(null);
  const POLL_INTERVAL_MS = 3000;
  const POLL_MAX_MS = 120_000;
  let pollStartedAt = 0;
  let runsCountAtStart = 0;

  function clearPoll(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  async function pollTestcases(): Promise<void> {
    if (Date.now() - pollStartedAt > POLL_MAX_MS) {
      clearPoll();
      toasts.warn('Generate timed out', 'No new test run appeared within 2 minutes.');
      return;
    }
    try {
      await product.loadTestcases();
      if (product.testcaseRuns.length > runsCountAtStart) {
        clearPoll();
        // Auto-select the newest run.
        if (product.testcaseRuns.length > 0) {
          activeRunId = product.testcaseRuns[0].run.id;
        }
      }
    } catch (e) {
      console.error('[TestCasesTab] poll error', e);
    }
  }

  function startPolling(): void {
    clearPoll();
    runsCountAtStart = product.testcaseRuns.length;
    pollStartedAt = Date.now();
    void pollTestcases();
    pollTimer = setInterval(() => { void pollTestcases(); }, POLL_INTERVAL_MS);
  }

  // Reset on story change.
  $effect(() => {
    product.selectedId;
    activeRunId = null;
    caseActions = {};
    showPublishForm = false;
    selected = new Set();
    orderedIds = [];
    clearPoll();
    if (product.selectedId) {
      void product.loadTestcases().then(() => {
        if (product.testcaseRuns.length > 0 && !activeRunId) {
          activeRunId = product.testcaseRuns[0].run.id;
        }
      });
    }
    return () => { clearPoll(); };
  });

  // Subscribe to `product_changed { section: 'testcases' }` WS events.
  $effect(() => {
    const off = product.onSectionChange('testcases', (_status: string) => {
      void pollTestcases(); // immediate refresh; clears poll if new run appeared
    });
    return off;
  });

  // ── Derived ─────────────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);

  const activeRunDetail = $derived<ProductTestcaseRunDetail | null>(
    activeRunId
      ? (product.testcaseRuns.find((r) => r.run.id === activeRunId) ?? null)
      : (product.testcaseRuns[0] ?? null)
  );
  const activeRun = $derived<ProductTestcaseRun | null>(activeRunDetail?.run ?? null);
  const activeCases = $derived<ProductTestcase[]>(activeRunDetail?.cases ?? []);

  // Cases in the locally-applied display order (reflects drag-and-drop before
  // the next store reload, which will bring the persisted order_idx back).
  const orderedCases = $derived.by(() => {
    const byId = new Map(activeCases.map((c) => [c.id, c]));
    // orderedIds may be stale (set from a previous activeCases snapshot); fall
    // back to any cases not present in orderedIds appended at the end.
    const seen = new Set<string>();
    const result: ProductTestcase[] = [];
    for (const id of orderedIds) {
      const tc = byId.get(id);
      if (tc) {
        result.push(tc);
        seen.add(id);
      }
    }
    // Append cases that arrived after the local orderedIds was set.
    for (const c of activeCases) {
      if (!seen.has(c.id)) result.push(c);
    }
    return result;
  });

  // Cases grouped by category, ordered by the local drag-reordered list (then
  // priority as tiebreaker for cases not yet reordered by the user).
  const groupedCases = $derived.by(() => {
    const result: Record<Category, ProductTestcase[]> = {
      happy: [],
      validation: [],
      error: [],
      edge: [],
    };
    for (const c of orderedCases) {
      const cat = c.category as Category;
      if (cat in result) {
        result[cat].push(c);
      } else {
        // Unknown category — put in edge as fallback.
        result.edge.push(c);
      }
    }
    // Within each category the drag-order is already respected by orderedCases;
    // use priority as a secondary sort only among cases that share the same
    // effective order_idx (i.e. newly inserted rows not yet reordered).
    for (const cat of CATEGORIES) {
      result[cat].sort((a, b) => {
        const oa = orderedIds.indexOf(a.id);
        const ob = orderedIds.indexOf(b.id);
        if (oa !== ob) {
          // Both present — use the local orderedIds position.
          if (oa !== -1 && ob !== -1) return oa - ob;
          // One missing (new row) → push it after the known ones.
          if (oa === -1) return 1;
          if (ob === -1) return -1;
        }
        // Fallback: priority.
        const pa = PRIORITY_ORDER[a.priority] ?? 99;
        const pb = PRIORITY_ORDER[b.priority] ?? 99;
        return pa - pb;
      });
    }
    return result;
  });

  // ── Helpers ──────────────────────────────────────────────────────────────────

  function parseSteps(json: string): TestcaseSteps {
    try {
      return JSON.parse(json) as TestcaseSteps;
    } catch {
      return { preconditions: [], steps: [], expected: '' };
    }
  }

  function getAction(id: string): CaseAction {
    if (!caseActions[id]) {
      caseActions = {
        ...caseActions,
        [id]: {
          mode: 'idle',
          reviewNote: '',
          editTitle: '',
          editCategory: '',
          editPriority: '',
          editPreconditions: '',
          editSteps: '',
          editExpected: '',
          busy: false,
        },
      };
    }
    return caseActions[id];
  }

  function setAction(id: string, patch: Partial<CaseAction>): void {
    const current = getAction(id);
    caseActions = { ...caseActions, [id]: { ...current, ...patch } };
  }

  function openEdit(tc: ProductTestcase): void {
    const steps = parseSteps(tc.steps_json);
    setAction(tc.id, {
      mode: 'edit',
      editTitle: tc.title,
      editCategory: tc.category,
      editPriority: tc.priority,
      editPreconditions: steps.preconditions.join('\n'),
      editSteps: steps.steps.join('\n'),
      editExpected: steps.expected,
      reviewNote: tc.review_note ?? '',
    });
  }

  function openChanges(tc: ProductTestcase): void {
    setAction(tc.id, {
      mode: 'changes',
      reviewNote: tc.review_note ?? '',
    });
  }

  function cancelAction(id: string): void {
    setAction(id, { mode: 'idle' });
  }

  async function approveCase(tc: ProductTestcase): Promise<void> {
    setAction(tc.id, { busy: true });
    try {
      await product.updateTestcase(tc.id, { status: 'approved' });
      setAction(tc.id, { mode: 'idle', busy: false });
      toasts.success('Case approved');
    } catch (e) {
      toasts.error('Could not approve', product.errMsg(e));
      setAction(tc.id, { busy: false });
    }
  }

  // ── Bulk approve selected cases ───────────────────────────────────────────
  async function bulkApproveSelected(): Promise<void> {
    if (!activeRun || selected.size === 0 || bulkApproving) return;
    bulkApproving = true;
    try {
      const result = await product.bulkApproveTestcases(activeRun.id, [...selected]);
      clearSelection();
      toasts.success(
        `${result.approved} case${result.approved !== 1 ? 's' : ''} approved`,
      );
    } catch (e) {
      toasts.error('Bulk approve failed', product.errMsg(e));
    } finally {
      bulkApproving = false;
    }
  }

  async function submitChanges(tc: ProductTestcase): Promise<void> {
    const action = getAction(tc.id);
    if (!action.reviewNote.trim()) {
      toasts.warn('Note required', 'Please add a review note before requesting changes.');
      return;
    }
    setAction(tc.id, { busy: true });
    try {
      await product.updateTestcase(tc.id, {
        status: 'changes_requested',
        review_note: action.reviewNote.trim(),
      });
      setAction(tc.id, { mode: 'idle', busy: false });
      toasts.info('Changes requested');
    } catch (e) {
      toasts.error('Could not update', product.errMsg(e));
      setAction(tc.id, { busy: false });
    }
  }

  async function submitEdit(tc: ProductTestcase): Promise<void> {
    const action = getAction(tc.id);
    setAction(tc.id, { busy: true });
    const steps: TestcaseSteps = {
      preconditions: action.editPreconditions
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean),
      steps: action.editSteps
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean),
      expected: action.editExpected.trim(),
    };
    try {
      await product.updateTestcase(tc.id, {
        title: action.editTitle.trim() || null,
        category: action.editCategory || null,
        priority: action.editPriority || null,
        steps,
        review_note: action.reviewNote.trim() || null,
      });
      setAction(tc.id, { mode: 'idle', busy: false });
      toasts.success('Test case updated');
    } catch (e) {
      toasts.error('Could not update', product.errMsg(e));
      setAction(tc.id, { busy: false });
    }
  }

  // ── Generate ─────────────────────────────────────────────────────────────────

  async function generate(): Promise<void> {
    if (generating) return;
    generating = true;
    try {
      await product.generateTests({ provider: provider || null });
      toasts.info('Test generation triggered', 'Waiting for a new run to appear…');
      startPolling();
    } catch (e) {
      toasts.error('Generate failed', product.errMsg(e));
    } finally {
      generating = false;
    }
  }

  // ── Approve run ───────────────────────────────────────────────────────────────

  async function approveRun(): Promise<void> {
    if (!activeRun || approvingRun) return;
    approvingRun = true;
    try {
      await product.approveRun(activeRun.id);
      const approvedCount = activeCases.filter((c) => c.status === 'approved').length;
      toasts.success(
        'Run approved',
        `${approvedCount} case${approvedCount !== 1 ? 's' : ''} approved — skill learning kicked off.`,
      );
    } catch (e) {
      toasts.error('Could not approve run', product.errMsg(e));
    } finally {
      approvingRun = false;
    }
  }

  // ── Publish ───────────────────────────────────────────────────────────────────

  async function publishTests(): Promise<void> {
    if (!activeRun || publishingRun) return;
    publishingRun = true;
    try {
      await product.publishTests(activeRun.id, {
        space_key: publishSpaceKey.trim() || null,
        parent_id: publishParentId.trim() || null,
      });
      showPublishForm = false;
      // Refresh to get confluence_url.
      await product.loadTestcases();
      const updatedRun = product.testcaseRuns.find((r) => r.run.id === activeRun.id);
      const url = updatedRun?.run.confluence_url ?? activeRun.confluence_url;
      if (url) {
        toasts.success('Published to Confluence', url);
      } else {
        toasts.success('Published to Confluence');
      }
    } catch (e) {
      toasts.error('Publish failed', product.errMsg(e));
    } finally {
      publishingRun = false;
    }
  }

  // ── Status helpers ────────────────────────────────────────────────────────────

  function statusClass(status: string): string {
    switch (status) {
      case 'approved': return 'pill-approved';
      case 'changes_requested': return 'pill-changes';
      case 'rejected': return 'pill-rejected';
      case 'draft': return 'pill-draft';
      case 'published': return 'pill-published';
      default: return 'pill-draft';
    }
  }

  function statusLabel(status: string): string {
    switch (status) {
      case 'approved': return 'Approved';
      case 'changes_requested': return 'Changes needed';
      case 'rejected': return 'Rejected';
      case 'draft': return 'Draft';
      case 'published': return 'Published';
      default: return status;
    }
  }

  function priorityClass(p: string): string {
    switch (p) {
      case 'high': return 'pri-high';
      case 'medium': return 'pri-medium';
      case 'low': return 'pri-low';
      default: return 'pri-medium';
    }
  }

  function fmtDate(s: string): string {
    try { return new Date(s).toLocaleString(); } catch { return s; }
  }
</script>

{#if !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="tc-tab">

    <!-- ── Generate panel ───────────────────────────────────────────────────── -->
    <section class="card gen-panel">
      <div class="gen-row">
        <div class="provider-wrap">
          <label class="field-label" for="tc-provider-sel">Provider</label>
          <select
            id="tc-provider-sel"
            class="sel"
            bind:value={provider}
            disabled={generating}
          >
            {#each PROVIDERS as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </div>

        <button
          class="action-btn primary"
          onclick={generate}
          disabled={generating || pollTimer !== null}
        >
          {#if generating}
            Triggering…
          {:else if pollTimer !== null}
            Polling for result…
          {:else}
            Generate test cases
          {/if}
        </button>

        {#if pollTimer !== null}
          <span class="polling-indicator">checking every 3s…</span>
        {/if}
      </div>
    </section>

    <!-- ── Run selector ──────────────────────────────────────────────────────── -->
    {#if product.testcaseRuns.length > 1}
      <div class="run-selector-row">
        <span class="field-label">Run</span>
        <select
          class="sel"
          bind:value={activeRunId}
        >
          {#each product.testcaseRuns as rd (rd.run.id)}
            <option value={rd.run.id}>
              {fmtDate(rd.run.created_at)} · {rd.run.status} · {rd.cases.length} cases
            </option>
          {/each}
        </select>
      </div>
    {/if}

    <!-- ── Active run area ────────────────────────────────────────────────────── -->
    {#if product.loadingTestcases && product.testcaseRuns.length === 0}
      <div class="muted">Loading test cases…</div>
    {:else if activeRunDetail && activeRun}
      <!-- Run header -->
      <section class="card run-header">
        <div class="rh-row">
          <div class="rh-info">
            <span class="rh-date">{fmtDate(activeRun.created_at)}</span>
            <span class="pill {statusClass(activeRun.status)}">{statusLabel(activeRun.status)}</span>
            <span class="rh-count">{activeCases.length} case{activeCases.length !== 1 ? 's' : ''}</span>
          </div>

          <div class="rh-actions">
            <!-- Approve run -->
            {#if activeRun.status !== 'approved' && activeRun.status !== 'published'}
              <button
                class="action-btn primary"
                onclick={approveRun}
                disabled={approvingRun}
                title="Approve this run — triggers skill self-improvement from the reviewed cases"
              >
                {approvingRun ? 'Approving…' : 'Approve run'}
              </button>
            {/if}

            <!-- Publish / Confluence link -->
            {#if activeRun.confluence_url}
              <a
                class="confluence-link"
                href={activeRun.confluence_url}
                target="_blank"
                rel="noopener noreferrer"
              >
                View in Confluence
              </a>
            {:else}
              <button
                class="action-btn"
                onclick={() => (showPublishForm = !showPublishForm)}
                disabled={publishingRun}
              >
                Publish to Confluence
              </button>
            {/if}
          </div>
        </div>

        <!-- Publish form -->
        {#if showPublishForm}
          <div class="publish-form">
            <div class="pf-row">
              <label class="field-label" for="pf-space">Space key</label>
              <input
                id="pf-space"
                class="text-input"
                type="text"
                placeholder="e.g. TEAM (optional)"
                bind:value={publishSpaceKey}
                disabled={publishingRun}
              />
            </div>
            <div class="pf-row">
              <label class="field-label" for="pf-parent">Parent page ID</label>
              <input
                id="pf-parent"
                class="text-input"
                type="text"
                placeholder="Confluence page ID (optional)"
                bind:value={publishParentId}
                disabled={publishingRun}
              />
            </div>
            <div class="pf-actions">
              <button
                class="action-btn primary"
                onclick={publishTests}
                disabled={publishingRun}
              >
                {publishingRun ? 'Publishing…' : 'Publish'}
              </button>
              <button
                class="action-btn"
                onclick={() => (showPublishForm = false)}
                disabled={publishingRun}
              >
                Cancel
              </button>
            </div>
          </div>
        {/if}
      </section>

      <!-- ── Bulk-select toolbar ──────────────────────────────────────────────── -->
      {#if activeCases.length > 0}
        <div class="bulk-toolbar">
          <label class="bulk-check-all">
            <input
              type="checkbox"
              checked={selected.size === activeCases.length && activeCases.length > 0}
              indeterminate={selected.size > 0 && selected.size < activeCases.length}
              onchange={() => (selected.size === activeCases.length ? clearSelection() : selectAll())}
            />
            <span class="field-label">
              {selected.size > 0 ? `${selected.size} selected` : 'Select all'}
            </span>
          </label>

          {#if selected.size > 0}
            <button
              class="action-btn primary"
              onclick={bulkApproveSelected}
              disabled={bulkApproving}
              title="Approve all selected draft cases"
            >
              {bulkApproving ? 'Approving…' : `Approve ${selected.size}`}
            </button>
            <button
              class="action-btn"
              onclick={clearSelection}
              disabled={bulkApproving}
            >
              Clear
            </button>
          {/if}

          {#if savingOrder}
            <span class="field-label save-indicator">Saving order…</span>
          {/if}
        </div>
      {/if}

      <!-- ── Cases grouped by category ──────────────────────────────────────── -->
      {#each CATEGORIES as cat (cat)}
        {@const cases = groupedCases[cat]}
        {#if cases.length > 0}
          <section class="category-section">
            <div class="cat-header">
              <span class="cat-label cat-{cat}">{CATEGORY_LABELS[cat]}</span>
              <span class="cat-count">{cases.length}</span>
            </div>

            <div class="cases-list" role="list">
              {#each cases as tc (tc.id)}
                {@const action = getAction(tc.id)}
                {@const steps = parseSteps(tc.steps_json)}

                <div
                  class="case-card card"
                  class:case-approved={tc.status === 'approved'}
                  class:case-selected={selected.has(tc.id)}
                  class:drag-over={dragOverId === tc.id}
                  role="listitem"
                  draggable="true"
                  ondragstart={() => onDragStart(tc.id)}
                  ondragover={(e) => onDragOver(tc.id, e)}
                  ondragleave={onDragLeave}
                  ondrop={() => onDrop(tc.id)}
                  ondragend={onDragEnd}
                >
                  <!-- Case header -->
                  <div class="case-header">
                    <div class="case-title-row">
                      <!-- Drag handle + checkbox (always visible) -->
                      <span class="drag-handle" title="Drag to reorder">&#8942;&#8942;</span>
                      <input
                        type="checkbox"
                        class="case-checkbox"
                        checked={selected.has(tc.id)}
                        onchange={() => toggleSelect(tc.id)}
                        onclick={(e) => e.stopPropagation()}
                        aria-label="Select {tc.title}"
                      />
                      <span class="case-title">{tc.title}</span>
                      <span class="priority-badge {priorityClass(tc.priority)}">{tc.priority}</span>
                      <span class="pill {statusClass(tc.status)}">{statusLabel(tc.status)}</span>
                    </div>

                    <!-- Actions row (shown when idle) -->
                    {#if action.mode === 'idle'}
                      <div class="case-actions">
                        <button
                          class="case-btn approve"
                          onclick={() => approveCase(tc)}
                          disabled={action.busy || tc.status === 'approved'}
                          title="Approve this test case"
                        >
                          {tc.status === 'approved' ? 'Approved' : 'Approve'}
                        </button>
                        <button
                          class="case-btn changes"
                          onclick={() => openChanges(tc)}
                          disabled={action.busy}
                          title="Request changes"
                        >
                          Request changes
                        </button>
                        <button
                          class="case-btn edit"
                          onclick={() => openEdit(tc)}
                          disabled={action.busy}
                          title="Edit this test case"
                        >
                          Edit
                        </button>
                      </div>
                    {/if}
                  </div>

                  <!-- Review note (display) -->
                  {#if tc.review_note && action.mode === 'idle'}
                    <div class="review-note">
                      <span class="rn-label">Note:</span>
                      <span class="rn-body">{tc.review_note}</span>
                    </div>
                  {/if}

                  <!-- Steps (display, idle mode) -->
                  {#if action.mode === 'idle'}
                    <div class="steps-section">
                      {#if steps.preconditions.length > 0}
                        <div class="steps-group">
                          <div class="steps-label">Preconditions</div>
                          <ul class="steps-list">
                            {#each steps.preconditions as pre}
                              <li>{pre}</li>
                            {/each}
                          </ul>
                        </div>
                      {/if}

                      {#if steps.steps.length > 0}
                        <div class="steps-group">
                          <div class="steps-label">Steps</div>
                          <ol class="steps-list">
                            {#each steps.steps as step}
                              <li>{step}</li>
                            {/each}
                          </ol>
                        </div>
                      {/if}

                      {#if steps.expected}
                        <div class="steps-group">
                          <div class="steps-label">Expected result</div>
                          <div class="expected-body">{steps.expected}</div>
                        </div>
                      {/if}
                    </div>
                  {/if}

                  <!-- Request changes form -->
                  {#if action.mode === 'changes'}
                    <div class="inline-form">
                      <div class="if-label">Review note (required)</div>
                      <textarea
                        class="text-area"
                        rows="3"
                        placeholder="Describe what needs to change…"
                        bind:value={caseActions[tc.id].reviewNote}
                        disabled={action.busy}
                      ></textarea>
                      <div class="if-actions">
                        <button
                          class="action-btn primary"
                          onclick={() => submitChanges(tc)}
                          disabled={action.busy}
                        >
                          {action.busy ? 'Saving…' : 'Submit'}
                        </button>
                        <button
                          class="action-btn"
                          onclick={() => cancelAction(tc.id)}
                          disabled={action.busy}
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                  {/if}

                  <!-- Edit form -->
                  {#if action.mode === 'edit'}
                    <div class="inline-form">
                      <div class="edit-grid">
                        <div class="edit-field">
                          <label class="field-label" for="ef-title-{tc.id}">Title</label>
                          <input
                            id="ef-title-{tc.id}"
                            class="text-input"
                            type="text"
                            bind:value={caseActions[tc.id].editTitle}
                            disabled={action.busy}
                          />
                        </div>
                        <div class="edit-field edit-row">
                          <div class="edit-field-sm">
                            <label class="field-label" for="ef-cat-{tc.id}">Category</label>
                            <select
                              id="ef-cat-{tc.id}"
                              class="sel"
                              bind:value={caseActions[tc.id].editCategory}
                              disabled={action.busy}
                            >
                              {#each CATEGORIES as c (c)}
                                <option value={c}>{CATEGORY_LABELS[c]}</option>
                              {/each}
                            </select>
                          </div>
                          <div class="edit-field-sm">
                            <label class="field-label" for="ef-pri-{tc.id}">Priority</label>
                            <select
                              id="ef-pri-{tc.id}"
                              class="sel"
                              bind:value={caseActions[tc.id].editPriority}
                              disabled={action.busy}
                            >
                              <option value="high">High</option>
                              <option value="medium">Medium</option>
                              <option value="low">Low</option>
                            </select>
                          </div>
                        </div>
                        <div class="edit-field">
                          <label class="field-label" for="ef-pre-{tc.id}">Preconditions (one per line)</label>
                          <textarea
                            id="ef-pre-{tc.id}"
                            class="text-area"
                            rows="3"
                            bind:value={caseActions[tc.id].editPreconditions}
                            disabled={action.busy}
                            placeholder="One precondition per line"
                          ></textarea>
                        </div>
                        <div class="edit-field">
                          <label class="field-label" for="ef-steps-{tc.id}">Steps (one per line)</label>
                          <textarea
                            id="ef-steps-{tc.id}"
                            class="text-area"
                            rows="4"
                            bind:value={caseActions[tc.id].editSteps}
                            disabled={action.busy}
                            placeholder="One step per line"
                          ></textarea>
                        </div>
                        <div class="edit-field">
                          <label class="field-label" for="ef-exp-{tc.id}">Expected result</label>
                          <textarea
                            id="ef-exp-{tc.id}"
                            class="text-area"
                            rows="2"
                            bind:value={caseActions[tc.id].editExpected}
                            disabled={action.busy}
                            placeholder="Expected outcome"
                          ></textarea>
                        </div>
                        <div class="edit-field">
                          <label class="field-label" for="ef-note-{tc.id}">Review note (optional)</label>
                          <input
                            id="ef-note-{tc.id}"
                            class="text-input"
                            type="text"
                            bind:value={caseActions[tc.id].reviewNote}
                            disabled={action.busy}
                            placeholder="Optional note"
                          />
                        </div>
                      </div>
                      <div class="if-actions">
                        <button
                          class="action-btn primary"
                          onclick={() => submitEdit(tc)}
                          disabled={action.busy}
                        >
                          {action.busy ? 'Saving…' : 'Save changes'}
                        </button>
                        <button
                          class="action-btn"
                          onclick={() => cancelAction(tc.id)}
                          disabled={action.busy}
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          </section>
        {/if}
      {/each}

      {#if activeCases.length === 0}
        <div class="muted">No test cases in this run.</div>
      {/if}

    {:else if !product.loadingTestcases}
      <div class="muted">No test cases yet. Click "Generate test cases" above.</div>
    {:else}
      <div class="muted">Loading…</div>
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
  .tc-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: min(900px, 92vw);
    width: 100%;
  }

  /* ── Card ─────────────────────────────────────────────────────── */
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    background: var(--surface-raised, var(--surface));
  }

  /* ── Generate panel ──────────────────────────────────────────── */
  .gen-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .provider-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .sel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
  }
  .polling-indicator {
    font-size: 11.5px;
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Run selector ────────────────────────────────────────────── */
  .run-selector-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* ── Buttons ─────────────────────────────────────────────────── */
  .action-btn {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms, color 110ms, opacity 110ms;
  }
  .action-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .action-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .action-btn.primary {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    font-weight: 600;
  }
  .action-btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }

  /* ── Run header ──────────────────────────────────────────────── */
  .run-header {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rh-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .rh-info {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }
  .rh-date {
    font-size: 12px;
    color: var(--text-dim);
  }
  .rh-count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .rh-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .confluence-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
    transition: background 110ms;
  }
  .confluence-link:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    text-decoration: none;
  }

  /* ── Publish form ────────────────────────────────────────────── */
  .publish-form {
    border-top: 1px solid var(--border);
    padding-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pf-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pf-actions {
    display: flex;
    gap: 8px;
  }
  .text-input {
    flex: 1;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 12px;
    max-width: 300px;
  }
  .text-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .text-area {
    width: 100%;
    box-sizing: border-box;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 12.5px;
    line-height: 1.5;
    resize: vertical;
    font-family: inherit;
  }
  .text-area:focus {
    outline: none;
    border-color: var(--accent);
  }

  /* ── Category sections ───────────────────────────────────────── */
  .category-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cat-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cat-label {
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    padding: 3px 10px;
    border-radius: 999px;
  }
  .cat-happy {
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
    color: var(--status-working);
  }
  .cat-validation {
    background: color-mix(in srgb, #60a5fa 16%, transparent);
    color: #2563eb;
  }
  .cat-error {
    background: color-mix(in srgb, #ef4444 16%, transparent);
    color: #b91c1c;
  }
  .cat-edge {
    background: color-mix(in srgb, #a78bfa 16%, transparent);
    color: #7c3aed;
  }
  .cat-count {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .cases-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* ── Bulk-select toolbar ─────────────────────────────────────────────── */
  .bulk-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-raised, var(--surface));
  }
  .bulk-check-all {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    user-select: none;
  }
  .save-indicator {
    font-style: italic;
    opacity: 0.7;
  }

  /* ── Case card ───────────────────────────────────────────────── */
  .case-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    cursor: default;
  }
  .case-card[draggable='true'] {
    cursor: grab;
  }
  .case-card.case-approved {
    border-color: color-mix(in srgb, var(--status-working) 30%, var(--border));
    background: color-mix(in srgb, var(--status-working) 4%, var(--surface-raised, var(--surface)));
  }
  .case-card.case-selected {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 5%, var(--surface-raised, var(--surface)));
  }
  .case-card.drag-over {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 25%, transparent);
  }

  /* ── Drag handle + checkbox ──────────────────────────────────────────── */
  .drag-handle {
    flex-shrink: 0;
    font-size: 13px;
    line-height: 1;
    color: var(--text-dim);
    opacity: 0.45;
    letter-spacing: -3px;
    cursor: grab;
    padding-right: 2px;
    user-select: none;
    transition: opacity 90ms;
  }
  .case-card:hover .drag-handle {
    opacity: 0.8;
  }
  .case-checkbox {
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .case-header {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .case-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .case-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
    flex: 1;
    min-width: 0;
    line-height: 1.3;
  }

  /* ── Priority badge ──────────────────────────────────────────── */
  .priority-badge {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 2px 7px;
    border-radius: 999px;
  }
  .pri-high {
    background: color-mix(in srgb, #ef4444 16%, transparent);
    color: #b91c1c;
  }
  .pri-medium {
    background: color-mix(in srgb, #f59e0b 16%, transparent);
    color: #b45309;
  }
  .pri-low {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }

  /* ── Status pills ────────────────────────────────────────────── */
  .pill {
    flex-shrink: 0;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .pill-approved {
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
    color: var(--status-working);
  }
  .pill-changes {
    background: color-mix(in srgb, #f59e0b 16%, transparent);
    color: #b45309;
  }
  .pill-rejected {
    background: color-mix(in srgb, #ef4444 16%, transparent);
    color: #b91c1c;
  }
  .pill-draft {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .pill-published {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }

  /* ── Case action buttons ─────────────────────────────────────── */
  .case-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .case-btn {
    height: 24px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    transition: background 90ms, border-color 90ms, color 90ms, opacity 90ms;
    white-space: nowrap;
  }
  .case-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .case-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .case-btn.approve {
    border-color: color-mix(in srgb, var(--status-working) 50%, var(--border));
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 8%, transparent);
  }
  .case-btn.approve:hover:not(:disabled) {
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .case-btn.changes {
    border-color: color-mix(in srgb, #f59e0b 50%, var(--border));
    color: #b45309;
    background: color-mix(in srgb, #f59e0b 8%, transparent);
  }
  .case-btn.changes:hover:not(:disabled) {
    background: color-mix(in srgb, #f59e0b 16%, transparent);
  }
  .case-btn.edit {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .case-btn.edit:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  /* ── Review note ─────────────────────────────────────────────── */
  .review-note {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    font-size: 12px;
    background: color-mix(in srgb, #f59e0b 8%, transparent);
    border: 1px solid color-mix(in srgb, #f59e0b 25%, transparent);
    border-radius: var(--radius-s);
    padding: 6px 10px;
  }
  .rn-label {
    font-weight: 700;
    color: #b45309;
    flex-shrink: 0;
  }
  .rn-body {
    color: var(--text);
    line-height: 1.4;
    font-style: italic;
  }

  /* ── Steps section ───────────────────────────────────────────── */
  .steps-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .steps-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .steps-label {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .steps-list {
    margin: 0;
    padding-left: 20px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .steps-list li {
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text);
  }
  .expected-body {
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text);
    background: color-mix(in srgb, var(--status-working) 6%, transparent);
    border-left: 3px solid var(--status-working);
    padding: 6px 10px;
    border-radius: 0 var(--radius-s) var(--radius-s) 0;
  }

  /* ── Inline forms ────────────────────────────────────────────── */
  .inline-form {
    border-top: 1px solid var(--border);
    padding-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .if-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .if-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }

  /* ── Edit form grid ──────────────────────────────────────────── */
  .edit-grid {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .edit-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .edit-row {
    display: flex;
    flex-direction: row;
    gap: 12px;
    align-items: flex-end;
  }
  .edit-field-sm {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 120px;
  }
</style>
