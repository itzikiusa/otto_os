<script lang="ts">
  // Insights tab: the latest report of the workspace's "Kubernetes watchdog"
  // personal agent (found by the template marker in its persona), rendered
  // from Markdown through the shared allowlist sanitizer, plus run history
  // and a "Run now". Offers to create the agent from the template when the
  // workspace has none.
  import { onDestroy, untrack } from 'svelte';
  import { marked } from 'marked';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { authedText } from '../../../lib/api/client';
  import { personalAgentsApi } from '../../../lib/api/personalAgents';
  import { sanitizeHtml } from '../../../lib/sanitize';
  import type { PersonalAgent, PersonalAgentRun } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import AgentEditSheet from '../../personal-agents/AgentEditSheet.svelte';
  import { K8S_WATCHDOG_MARK } from '../../personal-agents/templates';
  import { fmtAgo, verdictOf } from './monitor-util';

  let agent = $state<PersonalAgent | null>(null);
  let runs = $state<PersonalAgentRun[]>([]);
  let selected = $state<PersonalAgentRun | null>(null);
  let report = $state('');
  let loading = $state(true);
  let reportLoading = $state(false);
  let error = $state('');
  let creating = $state(false);
  let running = $state(false);
  let reportError = $state('');
  let loadVersion = 0;
  let reportVersion = 0;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  const html = $derived.by(() => {
    if (!report) return '';
    try {
      return sanitizeHtml(marked.parse(report, { async: false, gfm: true, breaks: true }) as string);
    } catch {
      return '';
    }
  });
  const verdict = $derived(verdictOf(report));

  async function load(): Promise<void> {
    const version = ++loadVersion;
    const wsId = ws.currentId;
    agent = null;
    runs = [];
    running = false;
    error = '';
    void openRun(null);
    if (!wsId) {
      loading = false;
      return;
    }
    loading = true;
    try {
      const all = await personalAgentsApi.list(wsId);
      if (version !== loadVersion) return;
      agent = all.find((a) => a.soul_md.includes(K8S_WATCHDOG_MARK)) ?? null;
      if (agent) {
        const nextRuns = await personalAgentsApi.runs(agent.id);
        if (version !== loadVersion) return;
        runs = nextRuns.slice(0, 12);
        await openRun(runs.find((r) => r.report_path) ?? runs[0] ?? null);
      }
    } catch (e) {
      if (version === loadVersion) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (version === loadVersion) loading = false;
    }
  }

  async function openRun(run: PersonalAgentRun | null): Promise<void> {
    const version = ++reportVersion;
    selected = run;
    report = '';
    reportError = '';
    reportLoading = false;
    if (!run?.report_path) return;
    reportLoading = true;
    try {
      const next = await authedText(personalAgentsApi.reportPath(run.id));
      if (version === reportVersion) report = next;
    } catch (e) {
      if (version === reportVersion) reportError = e instanceof Error ? e.message : String(e);
    } finally {
      if (version === reportVersion) reportLoading = false;
    }
  }

  $effect(() => {
    const id = ws.currentId;
    void id;
    untrack(() => {
      if (refreshTimer) clearTimeout(refreshTimer);
      void load();
    });
    return () => { loadVersion++; reportVersion++; };
  });

  onDestroy(() => {
    loadVersion++;
    reportVersion++;
    if (refreshTimer) clearTimeout(refreshTimer);
  });

  async function runNow(): Promise<void> {
    if (!agent || running) return;
    const version = loadVersion;
    running = true;
    try {
      await personalAgentsApi.run(agent.id);
      if (version !== loadVersion) return;
      toasts.success('Watchdog started', 'The report appears here when the run finishes.');
      refreshTimer = setTimeout(() => void load(), 4000);
    } catch (e) {
      if (version === loadVersion) toasts.error('Run failed', e instanceof Error ? e.message : String(e));
    } finally {
      if (version === loadVersion) running = false;
    }
  }

  function verdictClass(v: string | null): string {
    if (v === 'HEALTHY') return 'ok';
    if (v === 'DEGRADED') return 'warn';
    if (v === 'INCIDENT') return 'bad';
    return '';
  }
</script>

<div class="insights" data-testid="k8s-monitor-insights">
  {#if loading && !agent}
    <Skeleton rows={4} height={40} />
  {:else if error}
    <div class="error">{error} <button class="btn small" onclick={() => void load()}>Retry</button></div>
  {:else if !ws.currentId}
    <EmptyState icon="shield" title="Pick a workspace" body="Personal agents belong to a workspace; select one to see the watchdog's reports." />
  {:else if !agent}
    <EmptyState
      icon="shield"
      title="No Kubernetes watchdog yet"
      body="Create a personal agent from the “Kubernetes watchdog” template: every 15 minutes it calls k8s_health on each monitored cluster and writes a short report with a verdict. Delivery to Slack / Telegram / email works like any other agent."
      actionLabel="Create watchdog agent"
      onaction={() => (creating = true)}
    />
  {:else}
    <div class="head card">
      <div class="who">
        <span class="avatar">{agent.avatar || '🛡️'}</span>
        <div>
          <div class="name">{agent.name}</div>
          <div class="dim small">{agent.provider}{agent.model ? ` · ${agent.model}` : ''}{agent.enabled ? '' : ' · disabled'}</div>
        </div>
      </div>
      <div class="row">
        {#if verdict}<span class="verdict {verdictClass(verdict)}" data-testid="k8s-monitor-verdict">{verdict}</span>{/if}
        <button class="btn small" onclick={() => void runNow()} disabled={running}><Icon name="play" size={12} /> {running ? 'Starting…' : 'Run now'}</button>
        <button class="btn small ghost" onclick={() => router.go(`personal-agents/${encodeURIComponent(agent!.id)}`)}>Open agent</button>
      </div>
    </div>

    <div class="body">
      <aside class="runs card">
        <div class="section-title">Runs</div>
        {#if !runs.length}
          <div class="dim small">No runs yet.</div>
        {/if}
        {#each runs as r (r.id)}
          <button class="run" class:on={selected?.id === r.id} aria-pressed={selected?.id === r.id} title={r.summary || r.error || r.status} onclick={() => void openRun(r)}>
            <span class="st {r.status}"></span>
            <span class="when">{fmtAgo(r.started_at)}</span>
            <span class="sum dim">{r.summary || r.error || r.status}</span>
          </button>
        {/each}
      </aside>
      <article class="report card">
        {#if reportLoading}
          <Skeleton rows={6} height={16} />
        {:else if reportError}
          <div class="error" role="alert">Couldn't load the report: {reportError} <button class="btn small" onclick={() => void openRun(selected)}>Retry</button></div>
        {:else if html}
          <div class="md">{@html html}</div>
        {:else if selected}
          <div class="dim">This run has no report{selected.error ? `: ${selected.error}` : '.'}</div>
        {:else}
          <div class="dim">Select a run.</div>
        {/if}
      </article>
    </div>
  {/if}
</div>

{#if creating}
  <AgentEditSheet agent={null} template="k8s-watchdog" onclose={() => { creating = false; void load(); }} />
{/if}

<style>
  .insights {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    container-type: inline-size;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 14px;
  }
  .who {
    min-width: 0;
    overflow-wrap: anywhere;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .avatar {
    font-size: var(--fs-2xl);
  }
  .name {
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .row {
    flex-wrap: wrap;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .verdict {
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .verdict.ok {
    color: var(--success);
    border-color: color-mix(in srgb, var(--success) 40%, transparent);
  }
  .verdict.warn {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 40%, transparent);
  }
  .verdict.bad {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
  }
  .body {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    gap: 12px;
    min-height: 300px;
  }
  .runs {
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 70vh;
    overflow: auto;
  }
  .run {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: 6px;
    align-items: center;
    text-align: start;
    background: none;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 5px 6px;
    font-size: var(--fs-s);
    cursor: pointer;
    color: inherit;
  }
  .run.on {
    background: var(--surface-2);
    border-color: var(--border);
  }
  .st {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .st.done,
  .st.ok {
    background: var(--status-working);
  }
  .st.failed,
  .st.error {
    background: var(--status-exited);
  }
  .st.running {
    background: var(--accent);
  }
  .when {
    white-space: nowrap;
  }
  .sum {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .report {
    padding: 14px 18px;
    overflow: auto;
    max-height: 70vh;
  }
  .md :global(h1) {
    font-size: var(--fs-l);
    margin: 0 0 8px;
  }
  .md :global(h2) {
    font-size: var(--fs-m);
    margin: 14px 0 6px;
  }
  .md :global(p),
  .md :global(li) {
    font-size: var(--fs-m);
    line-height: 1.5;
  }
  .md :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--surface-2);
    padding: 0 3px;
    border-radius: 3px;
  }
  .md :global(pre) {
    direction: ltr;
    text-align: start;
    background: var(--surface-2);
    padding: 8px 10px;
    border-radius: 6px;
    overflow: auto;
  }
  .md :global(table) {
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .md :global(td),
  .md :global(th) {
    border: 1px solid var(--border);
    padding: 3px 8px;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .error {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  @container (max-width: 620px) {
    .head { flex-direction: column; align-items: stretch; }
    .who { width: 100%; }
    .body { grid-template-columns: minmax(0, 1fr); }
    .runs { max-height: 180px; }
    .report { max-height: none; }
  }
  @media (max-width: 760px) {
    .body {
      grid-template-columns: 1fr;
    }
  }
</style>
