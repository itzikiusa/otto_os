<script lang="ts">
  // Run-scoped agent view (embedded in the WF page's right sidebar "Agents" tab).
  // Shows ONLY the currently-selected run's sessions — every session each step
  // spawned, grouped by step, in run order. This includes a review_run step's
  // reviewer sessions AND its summarizer (the backend now surfaces them into
  // `nodes[*].sessions`), so a running review has full inline visibility.
  //
  // Each session expands to a live <Terminal> attached in place — no navigation
  // to the global Agents panel. Nothing here is a general/all-workflows list.
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { WorkflowRun } from '../../lib/api/types';

  interface Props {
    run: WorkflowRun;
    /** Resolve a node id to a friendly label. */
    nodeName?: (id: string) => string;
    /** A session id to auto-expand + scroll to (set when opened from a step). */
    focusSid?: string | null;
  }
  let { run, nodeName = (id) => id, focusSid = null }: Props = $props();

  // Only steps that actually spawned sessions, in run order.
  const groups = $derived(
    (run.nodes ?? [])
      .filter((n) => (n.sessions?.length ?? 0) > 0)
      .map((n) => ({ id: n.node_id, status: n.status, sessions: n.sessions ?? [] })),
  );
  const total = $derived(groups.reduce((a, g) => a + g.sessions.length, 0));
  const runActive = $derived(run.status === 'running' || run.status === 'pending');

  // Which session terminals are mounted (collapsed by default so we don't attach
  // dozens of PTYs at once). id-keyed, reset when the viewed run changes.
  let expanded = $state<Record<string, boolean>>({});
  let expandedRunId: string | null = null;
  $effect(() => {
    if (run.id !== expandedRunId) {
      expandedRunId = run.id;
      expanded = {};
      lastFocus = null;
    }
  });
  function toggle(sid: string): void {
    expanded[sid] = !expanded[sid];
  }

  // Auto-expand + reveal a session opened from a step's "Open session" button.
  let lastFocus: string | null = null;
  $effect(() => {
    const f = focusSid;
    if (f && f !== lastFocus) {
      lastFocus = f;
      expanded[f] = true;
      queueMicrotask(() =>
        document.querySelector(`[data-sess="${f}"]`)?.scrollIntoView({ block: 'nearest' }),
      );
    }
  });

  function sessOf(sid: string) {
    return ws.sessions.find((s) => s.id === sid) ?? null;
  }
  function sTitle(sid: string): string {
    return sessOf(sid)?.title || 'Session';
  }
  function sStatus(sid: string): string {
    return ws.statusMap[sid] ?? sessOf(sid)?.status ?? 'idle';
  }
  function shortId(id: string): string {
    return id.length > 6 ? id.slice(-6) : id;
  }
</script>

{#if total === 0}
  <div class="empty">
    No agent sessions for this run yet.{#if runActive}
      <br />They’ll appear here as steps spawn them.
    {/if}
  </div>
{:else}
  <div class="agents" data-testid="run-agents">
    {#each groups as g (g.id)}
      <div class="grp">
        <div class="grp-h">
          <span class="dot {g.status}"></span>
          <span class="grp-name" title={nodeName(g.id)}>{nodeName(g.id)}</span>
          <span class="grp-status">{g.status}</span>
          <span class="grow"></span>
          <span class="grp-count" title="{g.sessions.length} session(s)">{g.sessions.length}</span>
        </div>
        {#each g.sessions as sid (sid)}
          <div class="sess" data-sess={sid}>
            <button class="sess-h" onclick={() => toggle(sid)} title="Show live terminal">
              <Icon name={expanded[sid] ? 'chevronDown' : 'chevronRight'} size={12} />
              <span class="s-dot {sStatus(sid)}"></span>
              <span class="s-title">{sTitle(sid)}</span>
              <span class="s-status">{sStatus(sid)}</span>
              <span class="grow"></span>
              <code class="s-id" title={sid}>{shortId(sid)}</code>
            </button>
            {#if expanded[sid]}
              <div class="term">
                {#key sid}
                  <Terminal sessionId={sid} resumable />
                {/key}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/each}
  </div>
{/if}

<style>
  .empty {
    padding: 16px 12px;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .agents {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 8px;
    overflow: auto;
    min-height: 0;
    height: 100%;
  }
  .grp {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grp-h {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 3px 4px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .grp-name {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grp-status {
    text-transform: capitalize;
    font-weight: 500;
    letter-spacing: 0;
  }
  .grow {
    flex: 1;
  }
  .grp-count {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    padding: 0 6px;
    border-radius: 99px;
  }
  .sess {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .sess-h {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 8px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 11.5px;
    text-align: start;
  }
  .sess-h:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .s-title {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 50%;
  }
  .s-status {
    text-transform: capitalize;
    font-size: 10.5px;
  }
  .s-id {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-dim);
  }
  .term {
    height: 320px;
    border-top: 1px solid var(--border);
    display: flex;
    min-height: 0;
  }
  .dot,
  .s-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot.success,
  .dot.running,
  .s-dot.running,
  .s-dot.working {
    background: var(--status-working, #28c840);
  }
  .dot.error,
  .s-dot.exited {
    background: var(--status-exited);
  }
  .dot.pending,
  .dot.skipped,
  .s-dot.idle,
  .s-dot.reconnectable {
    background: var(--text-dim);
  }
</style>
