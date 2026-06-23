<script lang="ts">
  // Discovery tab — lists discovery swarm runs for the current story, lets the
  // user expand a run to read the report + per-task summaries + board messages,
  // and provides a "Run Discovery" button (with team picker) for repeat runs.
  import { product } from '../../lib/stores/product.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { DiscoveryRunSummary, DiscoveryRunDetail } from './types';

  // ── State ─────────────────────────────────────────────────────────────────
  let runs = $state<DiscoveryRunSummary[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  // Expanded run: id → DiscoveryRunDetail (null = loading, undefined = not fetched)
  let expandedId = $state<string | null>(null);
  let expandedDetail = $state<DiscoveryRunDetail | null>(null);
  let expandLoading = $state(false);
  let expandError = $state<string | null>(null);

  // Run Discovery controls
  let targetSwarmId = $state('');
  let running = $state(false);

  // ── Swarms ────────────────────────────────────────────────────────────────
  $effect(() => {
    const wsId = ws.currentId;
    if (wsId && swarm.swarms.length === 0) {
      void swarm.loadSwarms(wsId);
    }
  });

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    void loadRuns();
  });

  async function loadRuns(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      runs = await product.listDiscoveryRuns();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function toggleRun(id: string): Promise<void> {
    if (expandedId === id) {
      expandedId = null;
      expandedDetail = null;
      return;
    }
    expandedId = id;
    expandedDetail = null;
    expandError = null;
    expandLoading = true;
    try {
      expandedDetail = await product.getDiscoveryRun(id);
    } catch (e) {
      expandError = e instanceof Error ? e.message : String(e);
    } finally {
      expandLoading = false;
    }
  }

  async function runDiscovery(): Promise<void> {
    if (running) return;
    const story = product.detail?.story;
    if (!story || !ws.currentId) return;

    const targetSwarm = swarm.swarms.find((s) => s.id === targetSwarmId);
    const teamName = targetSwarm ? `"${targetSwarm.name}"` : 'a swarm';

    const attCount = 0; // attachment count not tracked here; overview has the panel
    const ok = await confirmer.ask(
      `Run Discovery in ${teamName}? This will START the swarm and send the story info${attCount > 0 ? ` + ${attCount} attachments` : ''} as discovery context.`,
      { title: 'Run Discovery', confirmLabel: 'Run Discovery' },
    );
    if (!ok) return;

    running = true;
    try {
      await product.discover(targetSwarmId ? { swarm_id: targetSwarmId } : {});
      toasts.success('Discovery started', 'The swarm is now analysing the story.');
      await loadRuns();
    } catch (e) {
      toasts.error('Discovery failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  async function viewInSwarm(summary: DiscoveryRunSummary): Promise<void> {
    if (!ws.currentId) return;
    try {
      await swarm.openProject(ws.currentId, summary.run.swarm_id, summary.run.project_id);
      router.go('swarm');
    } catch (e) {
      toasts.error('Could not open swarm', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Helpers ───────────────────────────────────────────────────────────────
  function statusColor(s: string): string {
    switch (s) {
      case 'done': return 'status-done';
      case 'error': return 'status-error';
      case 'running': return 'status-running';
      default: return 'status-other';
    }
  }

  function relDate(iso: string): string {
    try {
      const diff = Date.now() - new Date(iso).getTime();
      const s = Math.floor(diff / 1000);
      if (s < 60) return 'just now';
      const m = Math.floor(s / 60);
      if (m < 60) return `${m}m ago`;
      const h = Math.floor(m / 60);
      if (h < 24) return `${h}h ago`;
      const d = Math.floor(h / 24);
      if (d < 30) return `${d}d ago`;
      return new Date(iso).toLocaleDateString();
    } catch {
      return iso;
    }
  }
</script>

<div class="discovery-tab">
  <!-- ── Toolbar ─────────────────────────────────────────────────────── -->
  <div class="toolbar">
    <span class="toolbar-title">Discovery Runs</span>
    <span class="grow"></span>

    <!-- Team picker — mirrors PlanTab pattern -->
    {#if swarm.swarms.length > 1}
      <select class="picker" bind:value={targetSwarmId} title="Which swarm runs the discovery">
        <option value="">First swarm</option>
        {#each swarm.swarms as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
      </select>
    {/if}

    <button
      class="toolbar-btn primary"
      onclick={runDiscovery}
      disabled={running}
      title="Launch a new discovery swarm run for this story"
    >
      {running ? 'Starting…' : '⚡ Run Discovery'}
    </button>

    <button
      class="toolbar-btn"
      onclick={loadRuns}
      disabled={loading}
      title="Reload discovery runs"
    >
      {loading ? 'Loading…' : 'Refresh'}
    </button>
  </div>

  <!-- ── Run list ───────────────────────────────────────────────────── -->
  {#if loading && runs.length === 0}
    <div class="muted">Loading…</div>
  {:else if loadError}
    <div class="error-msg">Could not load discovery runs: {loadError}</div>
  {:else if runs.length === 0}
    <div class="empty-state">
      <p>No discovery runs yet.</p>
      <p>Click <strong>Run Discovery</strong> to analyse this story with a swarm.</p>
    </div>
  {:else}
    <div class="run-list">
      {#each runs as summary (summary.run.id)}
        {@const isOpen = expandedId === summary.run.id}
        <div class="run-card" class:open={isOpen}>
          <!-- ── Run header ──────────────────────────────────────────── -->
          <div class="run-header" role="button" tabindex="0"
            onclick={() => toggleRun(summary.run.id)}
            onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') toggleRun(summary.run.id); }}
          >
            <span class="coll-arrow">{isOpen ? '▼' : '▶'}</span>
            <span class="status-badge {statusColor(summary.derived_status)}">
              {summary.derived_status}
            </span>
            <span class="run-date">{relDate(summary.run.created_at)}</span>
            <span class="run-progress">
              {summary.done_count}/{summary.task_count} tasks
            </span>
            <button
              class="view-swarm-btn"
              onclick={(e) => { e.stopPropagation(); void viewInSwarm(summary); }}
              title="Open in Swarm"
            >
              View in Swarm →
            </button>
          </div>

          <!-- ── Expanded detail ────────────────────────────────────── -->
          {#if isOpen}
            <div class="run-body">
              {#if expandLoading}
                <div class="muted inner-pad">Loading details…</div>
              {:else if expandError}
                <div class="error-msg inner-pad">Could not load: {expandError}</div>
              {:else if expandedDetail}
                <!-- Report markdown -->
                {#if expandedDetail.run.report_md}
                  <div class="report-section">
                    <div class="section-label">Discovery Report</div>
                    <div class="md-body">{@html renderMarkdown(expandedDetail.run.report_md)}</div>
                  </div>
                {:else}
                  <div class="muted inner-pad">Report not yet available — check back once agents complete.</div>
                {/if}

                <!-- Per-task summaries -->
                {#if expandedDetail.task_summaries.length > 0}
                  <div class="tasks-section">
                    <div class="section-label">Task Summaries</div>
                    <div class="task-list">
                      {#each expandedDetail.tasks as task (task.id)}
                        {@const taskSummaryEntry = expandedDetail.task_summaries.find(([tid]) => tid === task.id)}
                        {@const taskSummary = taskSummaryEntry ? taskSummaryEntry[1] : null}
                        <div class="task-item">
                          <div class="task-row">
                            <span class="task-status-dot {statusColor(task.status)}"></span>
                            <span class="task-title">{task.title}</span>
                          </div>
                          {#if taskSummary}
                            <p class="task-summary-text">{taskSummary}</p>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  </div>
                {/if}

                <!-- Board messages -->
                {#if expandedDetail.messages.length > 0}
                  <div class="messages-section">
                    <div class="section-label">Discovery Board Messages</div>
                    <div class="message-list">
                      {#each expandedDetail.messages as msg (msg.id)}
                        <div class="message-item">
                          <span class="message-role">{msg.kind}</span>
                          <span class="message-content">{msg.body}</span>
                        </div>
                      {/each}
                    </div>
                  </div>
                {/if}

                <!-- Footer: View in Swarm -->
                <div class="run-footer">
                  <button
                    class="toolbar-btn"
                    onclick={() => viewInSwarm(summary)}
                  >
                    View in Swarm →
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .discovery-tab {
    display: flex;
    flex-direction: column;
    gap: 0;
    height: 100%;
    min-height: 0;
  }

  /* ── Toolbar ─────────────────────────────────────────────────────── */
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 0 10px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--border);
    margin-bottom: 12px;
  }
  .toolbar-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .picker {
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
  }
  .toolbar-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms;
  }
  .toolbar-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .toolbar-btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .toolbar-btn.primary:hover:not(:disabled) {
    opacity: 0.88;
  }
  .toolbar-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ── States ──────────────────────────────────────────────────────── */
  .muted {
    color: var(--text-dim);
    font-size: 13px;
    font-style: italic;
  }
  .inner-pad {
    padding: 12px 14px;
  }
  .error-msg {
    color: #ef4444;
    font-size: 13px;
    padding: 8px 0;
  }
  .empty-state {
    padding: 40px 16px;
    text-align: center;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.6;
  }
  .empty-state p {
    margin: 4px 0;
  }

  /* ── Run list ─────────────────────────────────────────────────────── */
  .run-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  .run-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--surface);
  }
  .run-card.open {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }

  .run-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 9px 12px;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font-size: 12.5px;
    transition: background 100ms;
    user-select: none;
  }
  .run-header:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .coll-arrow {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  /* Status badges */
  .status-badge {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .status-done {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .status-running {
    background: color-mix(in srgb, #f59e0b 18%, transparent);
    color: #b45309;
  }
  .status-error {
    background: color-mix(in srgb, #ef4444 18%, transparent);
    color: #dc2626;
  }
  .status-other {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }

  .run-date {
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .run-progress {
    color: var(--text-dim);
    font-size: 11.5px;
    margin-left: auto;
  }

  .view-swarm-btn {
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    transition: background 100ms;
  }
  .view-swarm-btn:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  /* ── Run body (expanded) ──────────────────────────────────────────── */
  .run-body {
    border-top: 1px solid var(--border);
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-label {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin-bottom: 8px;
  }

  .report-section {
    display: flex;
    flex-direction: column;
  }

  .md-body {
    font-size: 13.5px;
    line-height: 1.6;
    color: var(--text);
    overflow-wrap: break-word;
  }
  :global(.md-body h1, .md-body h2, .md-body h3) {
    margin: 0.8em 0 0.3em;
    font-weight: 600;
  }
  :global(.md-body p) {
    margin: 0.4em 0;
  }
  :global(.md-body ul, .md-body ol) {
    padding-inline-start: 1.4em;
    margin: 0.4em 0;
  }
  :global(.md-body code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.9em;
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    border-radius: 3px;
    padding: 1px 4px;
  }

  /* ── Task list ────────────────────────────────────────────────────── */
  .tasks-section {
    display: flex;
    flex-direction: column;
  }
  .task-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .task-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .task-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .task-status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .task-status-dot.status-done {
    background: var(--accent);
  }
  .task-status-dot.status-running {
    background: #f59e0b;
  }
  .task-status-dot.status-error {
    background: #ef4444;
  }
  .task-status-dot.status-other {
    background: var(--text-dim);
  }
  .task-title {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text);
  }
  .task-summary-text {
    margin: 0 0 0 13px;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
  }

  /* ── Messages ─────────────────────────────────────────────────────── */
  .messages-section {
    display: flex;
    flex-direction: column;
  }
  .message-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 300px;
    overflow-y: auto;
  }
  .message-item {
    display: flex;
    gap: 8px;
    font-size: 12px;
    line-height: 1.5;
  }
  .message-role {
    flex-shrink: 0;
    font-weight: 600;
    color: var(--text-dim);
    min-width: 50px;
    text-align: end;
    font-size: 11px;
    padding-top: 1px;
  }
  .message-content {
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: break-word;
    flex: 1;
    min-width: 0;
  }

  /* ── Footer ───────────────────────────────────────────────────────── */
  .run-footer {
    display: flex;
    justify-content: flex-end;
    padding-top: 4px;
    border-top: 1px solid var(--border);
  }
</style>
