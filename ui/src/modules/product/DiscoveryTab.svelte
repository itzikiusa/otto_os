<script lang="ts">
  // Discovery tab — lists discovery swarm runs for the current story, lets the
  // user expand a run to read the report + per-task summaries + board messages,
  // and provides a "Run Discovery" button (with team picker) for repeat runs.
  import { product } from '../../lib/stores/product.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { runStatus } from '../../lib/status';
  import { loadErrorText } from '../../lib/loadError';
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
  let listSequence = 0;
  let detailSequence = 0;

  // ── Swarms ────────────────────────────────────────────────────────────────
  $effect(() => {
    const wsId = ws.currentId;
    if (wsId) void swarm.ensureSwarms(wsId);
  });

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    ++detailSequence;
    expandedId = null;
    expandedDetail = null;
    void loadRuns();
  });

  async function loadRuns(): Promise<void> {
    const sequence = ++listSequence;
    const storyId = product.selectedId;
    loading = true;
    loadError = null;
    try {
      const result = await product.listDiscoveryRuns();
      if (sequence === listSequence && storyId === product.selectedId) runs = result;
    } catch (e) {
      if (sequence === listSequence && storyId === product.selectedId) loadError = loadErrorText(e);
    } finally {
      if (sequence === listSequence && storyId === product.selectedId) loading = false;
    }
  }

  async function toggleRun(id: string): Promise<void> {
    const sequence = ++detailSequence;
    const storyId = product.selectedId;
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
      const result = await product.getDiscoveryRun(id);
      if (sequence === detailSequence && storyId === product.selectedId) expandedDetail = result;
    } catch (e) {
      if (sequence === detailSequence && storyId === product.selectedId) expandError = loadErrorText(e);
    } finally {
      if (sequence === detailSequence && storyId === product.selectedId) expandLoading = false;
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
      { title: 'Run discovery', confirmLabel: 'Run discovery', danger: false },
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
      class="btn small primary"
      onclick={runDiscovery}
      disabled={running}
      title="Launch a new discovery swarm run for this story"
    >
      <Icon name="zap" size={12} /> {running ? 'Starting…' : 'Run discovery'}
    </button>

    <button
      class="btn small"
      onclick={loadRuns}
      disabled={loading}
      title="Reload discovery runs"
    >
      <Icon name="refresh" size={12} /> {loading ? 'Refreshing…' : 'Refresh'}
    </button>
  </div>

  <!-- ── Run list ───────────────────────────────────────────────────── -->
  {#if (loading || loadError) && runs.length === 0}
    <LoadState what="discovery runs" loading={loading} error={loadError} empty onretry={() => void loadRuns()} />
  {:else if runs.length === 0}
    <div class="empty-state">
      <p>No discovery runs yet.</p>
      <p>Run discovery to have a swarm analyse this story and report its findings here.</p>
    </div>
  {:else}
    <div class="run-list">
      {#each runs as summary (summary.run.id)}
        {@const isOpen = expandedId === summary.run.id}
        <div class="run-card" class:open={isOpen}>
          <!-- ── Run header ──────────────────────────────────────────── -->
          <div class="run-header" role="button" tabindex="0" aria-expanded={isOpen}
            onclick={() => toggleRun(summary.run.id)}
            onkeydown={(e) => { if (e.target !== e.currentTarget) return; if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); void toggleRun(summary.run.id); } }}
          >
            <span class="coll-arrow" aria-hidden="true"><Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={11} /></span>
            <span class="status-badge"><StatusBadge status={runStatus(summary.derived_status)} /></span>
            <span class="run-date">{relDate(summary.run.created_at)}</span>
            <span class="run-progress">
              {summary.done_count}/{summary.task_count} tasks
            </span>
            <button
              class="view-swarm-btn"
              onclick={(e) => { e.stopPropagation(); void viewInSwarm(summary); }}
              title="Open in Swarm"
            >
              Open in Swarm <Icon name="chevronRight" size={12} />
            </button>
          </div>

          <!-- ── Expanded detail ────────────────────────────────────── -->
          {#if isOpen}
            <div class="run-body">
              {#if expandLoading}
                <div class="muted inner-pad">Loading details…</div>
              {:else if expandError}
                <LoadState what="the run details" variant="compact" error={expandError} empty onretry={() => { const id = summary.run.id; expandedId = null; void toggleRun(id); }} />
              {:else if expandedDetail}
                <!-- Report markdown -->
                {#if expandedDetail.run.report_md}
                  <div class="report-section">
                    <div class="section-label">Discovery report</div>
                    <div class="md-body">{@html renderMarkdown(expandedDetail.run.report_md)}</div>
                  </div>
                {:else}
                  <div class="muted inner-pad">Report not yet available — check back once agents complete.</div>
                {/if}

                <!-- Per-task summaries -->
                {#if expandedDetail.task_summaries.length > 0}
                  <div class="tasks-section">
                    <div class="section-label">Task summaries</div>
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
                    <div class="section-label">Board messages</div>
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
                    class="btn small"
                    onclick={() => viewInSwarm(summary)}
                  >
                    Open in Swarm <Icon name="chevronRight" size={12} />
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-s);
    cursor: pointer;
  }

  /* ── States ──────────────────────────────────────────────────────── */
  .muted {
    color: var(--text-dim);
    font-size: var(--fs-m);
    font-style: italic;
  }
  .inner-pad {
    padding: 12px 14px;
  }
  .empty-state {
    padding: 40px 16px;
    text-align: center;
    color: var(--text-dim);
    font-size: var(--fs-m);
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
    font-size: var(--fs-s);
    transition: background 100ms;
    user-select: none;
  }
  .run-header:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .coll-arrow {
    display: inline-flex;
    align-items: center;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  /* Status badges */
  .status-badge {
    display: inline-flex;
    flex-shrink: 0;
  }
  .status-done {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent-text);
  }
  .status-running {
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
  }
  .status-error {
    background: color-mix(in srgb, var(--danger) 18%, transparent);
    color: var(--danger);
  }
  .status-other {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }

  .run-date {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .run-progress {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    margin-left: auto;
  }

  .view-swarm-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-m);
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
    border-radius: var(--radius-s);
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
    background: var(--status-warn);
  }
  .task-status-dot.status-error {
    background: var(--status-exited);
  }
  .task-status-dot.status-other {
    background: var(--text-dim);
  }
  .task-title {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
  }
  .task-summary-text {
    margin: 0 0 0 13px;
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .message-role {
    flex-shrink: 0;
    font-weight: 600;
    color: var(--text-dim);
    min-width: 50px;
    text-align: end;
    font-size: var(--fs-xs);
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
