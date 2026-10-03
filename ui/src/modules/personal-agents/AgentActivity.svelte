<script lang="ts">
  // Activity tab: what the agent is doing NOW (running run, its session's
  // state, approvals it waits on) and a merged timeline of its tool calls and
  // runs. Event-fed (personal_agent_activity / run updates / approval
  // changes); polls only while the event socket is down.
  import { onDestroy, untrack } from 'svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { liveQuery } from '../../lib/live';
  import type { Poller } from '../../lib/poll';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import type {
    PersonalAgentActivity,
    PersonalAgentActivityItem,
    PersonalAgentRun,
  } from '../../lib/api/types';

  interface Props {
    agentId: string;
  }
  let { agentId }: Props = $props();

  let data = $state<PersonalAgentActivity | null>(null);
  let error = $state<string | null>(null);

  async function load(): Promise<boolean> {
    try {
      data = await personalAgentsApi.activity(agentId);
      error = null;
      return true;
    } catch (e) {
      error = loadErrorText(e);
      return false;
    }
  }

  let poll: Poller | null = null;
  $effect(() => {
    const id = agentId;
    untrack(() => {
      poll?.stop();
      data = null;
      poll = liveQuery({
        run: () => load(),
        on: ['personal_agent_activity', 'personal_agent_run_updated', 'mcp_approval_changed'],
        match: (ev) => ev.agent_id === undefined || ev.agent_id === id,
        fallbackMs: 10_000,
        debounceMs: 400,
      });
    });
  });
  onDestroy(() => poll?.stop());

  type Entry =
    | { key: string; at: string; kind: 'item'; item: PersonalAgentActivityItem }
    | { key: string; at: string; kind: 'run'; run: PersonalAgentRun };

  const timeline = $derived.by<Entry[]>(() => {
    if (!data) return [];
    const out: Entry[] = [
      ...data.items
        // Waiting approvals have their own panel above.
        .filter((i) => i.kind !== 'approval_waiting')
        .map((item) => ({ key: `i${item.seq}`, at: item.at, kind: 'item' as const, item })),
      ...data.runs.map((run) => ({ key: `r${run.id}`, at: run.started_at, kind: 'run' as const, run })),
    ];
    return out.sort((a, b) => Date.parse(b.at) - Date.parse(a.at)).slice(0, 120);
  });
  const waiting = $derived((data?.approvals ?? []).filter((a) => a.status === 'pending'));

  const ITEM_ICON: Record<PersonalAgentActivityItem['kind'], IconName> = {
    tool_call: 'zap',
    blocked: 'lock',
    approval_required: 'hand',
    approval_waiting: 'clock',
  };

  function modeLabel(r: PersonalAgentRun): string {
    const m = r.mode ?? 'directed';
    return m === 'proactive' ? 'Proactive' : m === 'scheduled' ? 'Scheduled' : 'Directed';
  }
</script>

<LoadState what="this agent’s activity" loading={data === null && !error} error={data === null ? error : null} rows={4} onretry={() => void poll?.now()}>
  {#if data}
    <section class="pa-panel now" aria-labelledby="now-h">
      <h2 id="now-h">Now</h2>
      {#if data.now.run}
        {@const run = data.now.run}
        <div class="now-row">
          <StatusBadge status={runStatus(run.status)} variant="text" />
          <span class="chip">{modeLabel(run)}</span>
          {#if run.read_only}<span class="chip pa-ichip"><Icon name="lock" size={11} /> Read-only</span>{/if}
          <span class="meta">Started <RelTime iso={run.started_at} /></span>
          {#if data.now.session_status}<span class="meta">Session: {data.now.session_status}</span>{/if}
          {#if run.session_id}
            <button class="btn small" onclick={() => ws.navigateToSession(run.session_id ?? '')}>Watch session</button>
          {/if}
        </div>
      {:else}
        <p class="muted">Idle — no run in progress.</p>
      {/if}
      {#if waiting.length > 0}
        <h3>Waiting for you</h3>
        <ul class="list">
          {#each waiting as a (a.approval_id)}
            <li class="wait">
              <Icon name="hand" size={14} />
              <div class="grow">
                <strong>otto.{a.tool}</strong>
                {#if a.risk_label === 'sensitive'}<span class="chip pa-ichip pa-warn">Sensitive</span>{:else if a.risk_label === 'agent_rule'}<span class="chip">Agent rule</span>{/if}
                {#if a.detail}<p class="detail">{a.detail}</p>{/if}
              </div>
              <span class="meta"><RelTime iso={a.at} /></span>
            </li>
          {/each}
        </ul>
        <button class="link" onclick={() => router.go('mcp/activity')}>Review approvals</button>
      {/if}
    </section>

    <section class="pa-panel" aria-labelledby="tl-h">
      <h2 id="tl-h">Timeline</h2>
      {#if timeline.length === 0}
        <EmptyState icon="radar" title="Nothing yet" body="Tool calls, blocked actions, approvals and runs show up here as they happen." />
      {:else}
        <ol class="list timeline">
          {#each timeline as e (e.key)}
            {#if e.kind === 'item'}
              <li class="ev" class:blocked={e.item.kind === 'blocked'} class:ask={e.item.kind === 'approval_required'}>
                <Icon name={ITEM_ICON[e.item.kind]} size={13} />
                <span class="mono">otto.{e.item.tool}</span>
                <span class="detail grow">{e.item.kind === 'tool_call' ? 'called' : e.item.detail}</span>
                <span class="meta"><RelTime iso={e.at} /></span>
              </li>
            {:else}
              <li class="ev run">
                <Icon name="play" size={13} />
                <span>{modeLabel(e.run)} run</span>
                <StatusBadge status={runStatus(e.run.status)} variant="text" />
                {#if e.run.read_only}<span class="chip pa-ichip"><Icon name="lock" size={11} /> Read-only</span>{/if}
                <span class="detail grow" title={e.run.summary || e.run.error || undefined}>{e.run.summary || e.run.error || ''}</span>
                <span class="meta"><RelTime iso={e.at} /></span>
              </li>
            {/if}
          {/each}
        </ol>
        <p class="hint">Tool calls are a live view since the daemon started; runs keep their full history in Runs.</p>
      {/if}
    </section>
  {/if}
</LoadState>

<style>
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; margin-bottom: 12px; }
  .pa-panel h2 { margin: 0 0 8px; font-size: var(--fs-l); font-weight: 600; }
  .pa-panel h3 { margin: 12px 0 6px; font-size: var(--fs-m); font-weight: 600; }
  .now-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: var(--fs-s); }
  .muted, .hint { color: var(--text-dim); font-size: var(--fs-s); margin: 0; }
  .hint { margin-top: 8px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .wait { display: flex; align-items: flex-start; gap: 8px; padding: 6px 0; font-size: var(--fs-s); }
  .wait > :global(svg) { color: var(--warning); margin-top: 2px; flex-shrink: 0; }
  .grow { flex: 1; min-width: 0; }
  .detail { margin: 2px 0 0; color: var(--text-dim); font-size: var(--fs-s); overflow-wrap: anywhere; }
  .ev { display: flex; align-items: center; gap: 8px; font-size: var(--fs-s); padding: 4px 0; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
  .ev:last-child { border-bottom: 0; }
  .ev > :global(svg) { color: var(--text-dim); flex-shrink: 0; }
  .ev.blocked > :global(svg) { color: var(--danger); }
  .ev.ask > :global(svg) { color: var(--warning); }
  .ev .detail { margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mono { font-family: var(--font-mono); }
  .meta { color: var(--text-dim); font-size: var(--fs-s); white-space: nowrap; }
  .pa-ichip { display: inline-flex; align-items: center; gap: 4px; }
  .pa-warn { color: var(--warning); border-color: color-mix(in srgb, var(--warning) 35%, transparent); }
  .link { border: 0; background: none; padding: 0; margin-top: 6px; font: inherit; font-size: var(--fs-s); color: var(--accent-text); cursor: pointer; }
  .link:hover { text-decoration: underline; }
</style>
