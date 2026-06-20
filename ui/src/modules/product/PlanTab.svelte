<script lang="ts">
  // Plan / Tasks tab — generate (or regenerate) an implementation plan for the
  // story, render it as a task tree with 3-state checkboxes the PO can toggle,
  // and persist toggles in place. Modeled on RewriteTab's load/poll pattern.
  import { product } from '../../lib/stores/product.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { confirmer } from '../../lib/confirm.svelte';
  import { parsePlan, setItemStatus, type Status, type Task } from './plan_parse';
  import type { ProductStoryVersion } from './types';

  const PROVIDERS = ['claude', 'openai'] as const;

  // ── Local UI state ──────────────────────────────────────────────────────────
  let provider = $state<string>('claude');
  let generating = $state(false);
  let saving = $state(false);
  let showRaw = $state(false);

  // The loaded plan version + its markdown body (the source of truth we edit).
  let planVersion = $state<ProductStoryVersion | null>(null);
  let body = $state<string>('');
  let loading = $state(false);

  // Parsed tasks (re-derived whenever `body` changes).
  const tasks = $derived<Task[]>(parsePlan(body).tasks);

  // Overall rollup across all items.
  const totals = $derived.by(() => {
    let done = 0;
    let inProgress = 0;
    let total = 0;
    for (const t of tasks) {
      for (const it of t.items) {
        total++;
        if (it.status === 'done') done++;
        else if (it.status === 'in_progress') inProgress++;
      }
    }
    return { done, inProgress, total };
  });

  const renderedRaw = $derived(body ? renderMarkdown(body) : '');

  // ── Polling for a freshly generated plan ─────────────────────────────────────
  let pollTimer = $state<ReturnType<typeof setInterval> | null>(null);
  const POLL_INTERVAL_MS = 3000;
  const POLL_MAX_MS = 120_000;
  let pollStartedAt = 0;

  function clearPoll(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function latestPlan(): ProductStoryVersion | null {
    let best: ProductStoryVersion | null = null;
    for (const v of product.versions) {
      if (v.kind === 'plan') {
        if (!best || v.version_no > best.version_no) best = v;
      }
    }
    return best;
  }

  async function loadPlanBody(v: ProductStoryVersion): Promise<void> {
    loading = true;
    try {
      const full = await product.getVersion(v.id);
      planVersion = full;
      body = full.body_md ?? '';
    } catch (e) {
      toasts.error('Could not load plan', product.errMsg(e));
    } finally {
      loading = false;
    }
  }

  async function pollForPlan(): Promise<void> {
    if (Date.now() - pollStartedAt > POLL_MAX_MS) {
      clearPoll();
      toasts.warn('Plan timed out', 'No plan appeared within 2 minutes.');
      return;
    }
    try {
      await product.loadVersions();
      const plan = latestPlan();
      if (plan && (!planVersion || plan.version_no > planVersion.version_no)) {
        clearPoll();
        await loadPlanBody(plan);
        toasts.success('Plan ready', 'Implementation plan generated.');
      }
    } catch (e) {
      console.error('[PlanTab] poll error', e);
    }
  }

  function startPolling(): void {
    clearPoll();
    pollStartedAt = Date.now();
    void pollForPlan();
    pollTimer = setInterval(() => { void pollForPlan(); }, POLL_INTERVAL_MS);
  }

  // ── Initial load + reset on story change ─────────────────────────────────────
  $effect(() => {
    product.selectedId;
    planVersion = null;
    body = '';
    showRaw = false;
    clearPoll();
    if (product.selectedId) void initialLoad();
    return () => { clearPoll(); };
  });

  // Subscribe to `product_changed { section: 'plan' }` WS events for faster feedback.
  $effect(() => {
    const off = product.onSectionChange('plan', (_status: string) => {
      void pollForPlan(); // trigger an immediate poll that also clears timer if ready
    });
    return off;
  });

  const story = $derived(product.detail?.story ?? null);

  async function initialLoad(): Promise<void> {
    try {
      await product.loadVersions();
      const plan = latestPlan();
      if (plan) await loadPlanBody(plan);
    } catch (e) {
      console.error('[PlanTab] initialLoad error', e);
    }
  }

  // ── Actions ──────────────────────────────────────────────────────────────────

  async function generate(): Promise<void> {
    if (generating) return;
    generating = true;
    try {
      await product.generatePlan({ provider: provider || null });
      toasts.info('Plan triggered', 'Waiting for the plan to appear…');
      startPolling();
    } catch (e) {
      toasts.error('Plan generation failed', product.errMsg(e));
    } finally {
      generating = false;
    }
  }

  async function regenerate(): Promise<void> {
    const ok = await confirmer.ask(
      'Generate a new plan? This creates a new plan version; your current checkbox progress stays on the existing version but a fresh plan will be shown.',
      { title: 'Regenerate plan', confirmLabel: 'Regenerate', danger: false },
    );
    if (!ok) return;
    await generate();
  }

  async function refresh(): Promise<void> {
    try {
      await product.loadVersions();
      const plan = latestPlan();
      if (plan) await loadPlanBody(plan);
      toasts.info('Refreshed', 'Loaded the latest plan version.');
    } catch (e) {
      toasts.error('Refresh failed', product.errMsg(e));
    }
  }

  // Cycle a single item's status todo → in_progress → done → todo, optimistic
  // local re-parse, then persist the whole body in place. Debounced ~500ms so
  // rapid clicks don't fire a POST per click.
  const NEXT: Record<Status, Status> = {
    todo: 'in_progress',
    in_progress: 'done',
    done: 'todo',
  };

  // Debounce state for savePlan
  let saveDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  let savedTick = $state(false); // briefly true after a successful save

  function scheduleSave(latestBody: string): void {
    if (saveDebounceTimer !== null) clearTimeout(saveDebounceTimer);
    saveDebounceTimer = setTimeout(() => {
      saveDebounceTimer = null;
      saving = true;
      product.savePlan(latestBody).then(() => {
        savedTick = true;
        setTimeout(() => { savedTick = false; }, 1500);
      }).catch((e) => {
        toasts.error('Could not save progress', product.errMsg(e));
        // Best-effort reload to resync with the server.
        const plan = latestPlan();
        if (plan) void loadPlanBody(plan);
      }).finally(() => {
        saving = false;
      });
    }, 500);
  }

  // Cleanup debounce timer on unmount.
  $effect(() => {
    return () => {
      if (saveDebounceTimer !== null) clearTimeout(saveDebounceTimer);
    };
  });

  async function cycleItem(lineIndex: number, current: Status): Promise<void> {
    const next = NEXT[current];
    const updated = setItemStatus(body, lineIndex, next);
    if (updated === body) return; // unrecognized line — no-op
    body = updated; // optimistic; `tasks` re-derives automatically
    scheduleSave(body);
  }

  function statusLabel(s: Status): string {
    if (s === 'done') return 'done';
    if (s === 'in_progress') return 'in progress';
    return 'todo';
  }

  function boxGlyph(s: Status): string {
    if (s === 'done') return '✓';
    if (s === 'in_progress') return '~';
    return '';
  }

  function taskCount(t: Task): string {
    const done = t.items.filter((i) => i.status === 'done').length;
    return `${done}/${t.items.length}`;
  }

  // ── Send to Swarm ─────────────────────────────────────────────────────────
  // The flagship Product → Swarm hand-off: turn the story (+ its plan) into a
  // runnable swarm project and jump to the project's Kanban board.
  let sendingToSwarm = $state(false);

  // Existing linked swarm project, if this story was already sent (drives the
  // badge + flips the action to "Open in Swarm").
  const swarmLink = $derived(product.detail?.swarm_link ?? null);

  async function openLinkedSwarm(): Promise<void> {
    const link = swarmLink;
    const wsId = ws.currentId;
    if (!link || !wsId) return;
    await swarm.openProject(wsId, link.swarm_id, link.project_id);
    router.go('swarm');
  }

  async function sendToSwarm(): Promise<void> {
    if (sendingToSwarm || !ws.currentId) return;
    if (swarmLink) {
      await openLinkedSwarm();
      return;
    }
    const ok = await confirmer.ask(
      'Create a swarm project from this story and seed it with the plan tasks? You can then run the swarm to implement it.',
      { title: 'Send to Swarm', confirmLabel: 'Send to Swarm', danger: false },
    );
    if (!ok) return;
    sendingToSwarm = true;
    try {
      const resp = await product.sendToSwarm();
      toasts.success(
        'Sent to Swarm',
        `Project “${resp.project.name}” created with ${resp.tasks.length} task(s).`,
      );
      await swarm.openProject(ws.currentId, resp.swarm.id, resp.project.id);
      router.go('swarm');
    } catch (e) {
      toasts.error('Send to Swarm failed', product.errMsg(e));
    } finally {
      sendingToSwarm = false;
    }
  }
</script>

{#if !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="plan-tab">
    {#if loading}
      <div class="muted">Loading plan…</div>
    {:else if !planVersion}
      <!-- ── No plan yet: generate panel ─────────────────────────────────────── -->
      <section class="card gen-panel">
        <div class="gen-row">
          <div class="provider-wrap">
            <label class="field-label" for="plan-provider-sel">Provider</label>
            <select
              id="plan-provider-sel"
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
              Generating…
            {:else}
              Generate plan
            {/if}
          </button>
          {#if pollTimer !== null}
            <span class="polling-indicator">checking every 3s…</span>
          {/if}
          <button
            class="action-btn swarm"
            onclick={sendToSwarm}
            disabled={sendingToSwarm}
            title={swarmLink ? 'Open the linked swarm project' : 'Create a swarm project from this story (the swarm planner generates tasks)'}
          >
            {#if sendingToSwarm}Sending…{:else if swarmLink}Open in Swarm{:else}⚡ Send to Swarm{/if}
          </button>
        </div>
      </section>
      {#if swarmLink}
        <div class="muted">
          Linked to swarm project <strong>{swarmLink.project_name}</strong>.
        </div>
      {/if}
      {#if pollTimer === null && !swarmLink}
        <div class="muted">No implementation plan yet. Generate one to break the story into trackable tasks, or send straight to a swarm.</div>
      {/if}
    {:else}
      <!-- ── Plan exists: header + task tree ─────────────────────────────────── -->
      <section class="card plan-header">
        <div class="ph-row">
          <div class="ph-info">
            <span class="ph-label">Implementation Plan v{planVersion.version_no}</span>
            <span class="ph-date">{new Date(planVersion.created_at).toLocaleString()}</span>
          </div>
          <div class="ph-progress">
            <span class="prog-text">
              {totals.done}/{totals.total} done{totals.inProgress > 0 ? `, ${totals.inProgress} in progress` : ''}
            </span>
            <div class="prog-bar" aria-hidden="true">
              <div
                class="prog-fill"
                style="width: {totals.total ? Math.round((totals.done / totals.total) * 100) : 0}%"
              ></div>
            </div>
          </div>
          <div class="ph-actions">
            {#if saving}<span class="saving">saving…</span>{:else if savedTick}<span class="saved-tick">saved ✓</span>{/if}
            {#if swarmLink}
              <span class="swarm-badge" title="This story is linked to a swarm project">
                ⚡ {swarmLink.project_name}
              </span>
            {/if}
            <button class="action-btn" onclick={refresh} disabled={generating}>Refresh</button>
            <button class="action-btn" onclick={() => (showRaw = !showRaw)}>
              {showRaw ? 'Hide raw' : 'Raw'}
            </button>
            <button
              class="action-btn swarm"
              onclick={sendToSwarm}
              disabled={sendingToSwarm}
              title={swarmLink ? 'Open the linked swarm project' : 'Create a swarm project from this story'}
            >
              {#if sendingToSwarm}Sending…{:else if swarmLink}Open in Swarm{:else}⚡ Send to Swarm{/if}
            </button>
            <button class="action-btn primary" onclick={regenerate} disabled={generating || pollTimer !== null}>
              {pollTimer !== null ? 'Generating…' : 'Regenerate'}
            </button>
          </div>
        </div>
      </section>

      {#if tasks.length === 0}
        <div class="muted">The plan has no recognizable tasks. View the raw markdown to inspect it.</div>
      {/if}

      <div class="tasks">
        {#each tasks as task (task.lineIndex)}
          <section class="card task" class:done={task.status === 'done'}>
            <header class="task-head">
              <span class="task-status status-{task.status}">{statusLabel(task.status)}</span>
              <h3 class="task-title">{task.title}</h3>
              <span class="task-count">{taskCount(task)}</span>
            </header>
            <ul class="items">
              {#each task.items as item (item.lineIndex)}
                <li class="item status-{item.status}">
                  <button
                    class="checkbox status-{item.status}"
                    title="Mark {statusLabel(NEXT[item.status])}"
                    aria-label="Toggle: currently {statusLabel(item.status)}"
                    onclick={() => cycleItem(item.lineIndex, item.status)}
                    disabled={saving}
                  >{boxGlyph(item.status)}</button>
                  <span class="item-text">{item.text}</span>
                </li>
              {/each}
            </ul>
          </section>
        {/each}
      </div>

      {#if showRaw}
        <section class="card raw">
          <div class="md-body">{@html renderedRaw}</div>
        </section>
      {/if}
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
  .plan-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 1000px;
    width: 100%;
  }

  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    background: var(--surface-raised, var(--surface));
  }

  /* Generate panel */
  .gen-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .provider-wrap { display: flex; align-items: center; gap: 6px; }
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
  .polling-indicator { font-size: 11.5px; color: var(--text-dim); font-style: italic; }

  /* Plan header */
  .ph-row {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
  }
  .ph-info { display: flex; flex-direction: column; gap: 2px; min-width: 160px; }
  .ph-label { font-size: 13px; font-weight: 600; color: var(--text); }
  .ph-date { font-size: 11px; color: var(--text-dim); }
  .ph-progress { display: flex; flex-direction: column; gap: 4px; flex: 1; min-width: 160px; }
  .prog-text { font-size: 12px; color: var(--text-dim); }
  .prog-bar {
    height: 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    overflow: hidden;
  }
  .prog-fill {
    height: 100%;
    background: var(--accent);
    transition: width 160ms;
  }
  .ph-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-left: auto; }
  .saving { font-size: 11.5px; color: var(--text-dim); font-style: italic; }
  .saved-tick { font-size: 11.5px; color: var(--status-idle, #3a8c3a); font-weight: 600; }

  /* Buttons */
  .action-btn {
    height: 30px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms, color 110ms, opacity 110ms;
  }
  .action-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .action-btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .action-btn.primary {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    font-weight: 600;
  }
  .action-btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  /* Send to Swarm — a distinct accent so the cross-feature hand-off stands out. */
  .action-btn.swarm {
    border-color: #8b5cf6;
    color: #8b5cf6;
    background: color-mix(in srgb, #8b5cf6 10%, transparent);
    font-weight: 600;
  }
  .action-btn.swarm:hover:not(:disabled) {
    background: color-mix(in srgb, #8b5cf6 20%, transparent);
  }
  .swarm-badge {
    font-size: 11px;
    font-weight: 600;
    color: #8b5cf6;
    background: color-mix(in srgb, #8b5cf6 14%, transparent);
    padding: 3px 9px;
    border-radius: 999px;
    white-space: nowrap;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Tasks */
  .tasks { display: flex; flex-direction: column; gap: 10px; }
  .task { padding: 10px 14px; }
  .task.done { opacity: 0.75; }
  .task-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .task-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    margin: 0;
    flex: 1;
  }
  .task-count { font-size: 12px; color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .task-status {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 8px;
    border-radius: 999px;
    white-space: nowrap;
  }
  .status-todo { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }
  .status-in_progress { background: color-mix(in srgb, #f59e0b 18%, transparent); color: #b45309; }
  .status-done { background: color-mix(in srgb, #22c55e 18%, transparent); color: #15803d; }

  .items { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 5px; }
  .item { display: flex; align-items: flex-start; gap: 9px; font-size: 13px; line-height: 1.5; }
  .item.status-done .item-text { text-decoration: line-through; color: var(--text-dim); }
  .item-text { color: var(--text); padding-top: 1px; }
  .checkbox {
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    border-radius: 4px;
    border: 1.5px solid var(--border);
    background: var(--surface);
    color: #fff;
    font-size: 12px;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    transition: background 110ms, border-color 110ms;
  }
  .checkbox:disabled { cursor: not-allowed; opacity: 0.6; }
  .checkbox.status-done { background: #22c55e; border-color: #22c55e; }
  .checkbox.status-in_progress { background: #f59e0b; border-color: #f59e0b; color: #422006; font-weight: 700; }
  .checkbox.status-todo:hover:not(:disabled) { border-color: var(--accent); }

  /* Raw markdown */
  .raw { padding: 14px 16px; }
  .md-body { font-size: 13px; line-height: 1.6; color: var(--text); }
  .md-body :global(h3) { font-size: 1.05em; font-weight: 700; margin: 1em 0 0.4em; }
  .md-body :global(ul) { padding-left: 1.4em; margin: 0 0 0.6em; }
  .md-body :global(li) { margin-bottom: 0.2em; }
  .md-body :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.88em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    padding: 1px 5px;
    border-radius: 3px;
  }
</style>
