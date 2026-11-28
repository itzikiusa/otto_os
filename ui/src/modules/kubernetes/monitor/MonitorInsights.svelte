<script lang="ts">
  import { rel } from '../../../lib/stores/now.svelte';
  import { NO_WORKSPACE } from '../../../lib/labels';
  // Insights tab: the latest report of the workspace's "Kubernetes watchdog"
  // personal agent (found by the template marker in its persona), rendered
  // from Markdown through the shared allowlist sanitizer, plus run history
  // and a "Run now". Offers to create the agent from the template when the
  // workspace has none.
  import { loadErrorText } from '../../../lib/loadError';
  import { toastError } from '../../../lib/toastError';
  import { onDestroy, untrack } from 'svelte';
  import { renderMarkdownGfm } from '../../../lib/md';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { authedText } from '../../../lib/api/client';
  import { personalAgentsApi } from '../../../lib/api/personalAgents';
  import type { PersonalAgent, PersonalAgentRun } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ListDetail from '../../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import SideRow from '../../../lib/components/side/SideRow.svelte';
  import AgentEditSheet from '../../personal-agents/AgentEditSheet.svelte';
  import { K8S_WATCHDOG_MARK } from '../../personal-agents/templates';
  import { verdictOf } from './monitor-util';

  let agent = $state<PersonalAgent | null>(null);
  let runs = $state<PersonalAgentRun[]>([]);
  let selected = $state<PersonalAgentRun | null>(null);
  /** Phone: the runs are the page until one is opened. */
  let phoneOpen = $state(false);
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

  // lib/md: GFM + the allowlist sanitizer, never throws.
  const html = $derived(report ? renderMarkdownGfm(report, { breaks: true }) : '');
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
      if (version === loadVersion) error = loadErrorText(e);
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
      if (version === reportVersion) reportError = loadErrorText(e);
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
      if (version === loadVersion) toastError('Couldn’t run the insights', e);
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
    <LoadState what="the watchdog" variant="compact" {error} empty={true} onretry={() => void load()} />
  {:else if !ws.currentId}
    <EmptyState icon="shield" title={NO_WORKSPACE} body="Personal agents belong to a workspace — its watchdog reports show here." />
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
        <span class="avatar" aria-hidden="true"><ProviderIcon provider={agent.provider || 'claude'} size={20} /></span>
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

    <!-- Runs | report: the shared list pane (push on a phone, "‹ Runs"). -->
    <div class="body card">
      <ListDetail module="kubernetes" pane="insights" name="Runs" open={phoneOpen} onback={() => (phoneOpen = false)} empty={!runs.length}>
        {#snippet list()}
          <ListPane count={runs.length}>
            {#each runs as r (r.id)}
              <SideRow
                title={r.started_at ? rel(r.started_at) : 'never'}
                meta={r.summary || r.error || r.status}
                selected={selected?.id === r.id}
                tip={r.summary || r.error || r.status}
                onclick={() => { phoneOpen = true; void openRun(r); }}
              >
                {#snippet trailing()}<span class="st {r.status}">{r.status}</span>{/snippet}
              </SideRow>
            {/each}
          </ListPane>
        {/snippet}
      <article class="report">
        {#if reportLoading}
          <Skeleton rows={6} height={16} />
        {:else if reportError}
          <LoadState variant="compact" what="the report" error={reportError} empty onretry={() => void openRun(selected)} />
        {:else if html}
          <div class="md">{@html html}</div>
        {:else if selected}
          <div class="dim">This run has no report{selected.error ? `: ${selected.error}` : '.'}</div>
        {:else if !runs.length}
          <EmptyState icon="clock" title="No runs yet" body="Run now, or wait for the watchdog’s next scheduled run." />
        {:else}
          <div class="dim">Select a run.</div>
        {/if}
      </article>
      </ListDetail>
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
    font-weight: 600;
    letter-spacing: .06em;
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
  /* A bounded box: the runs pane and the report scroll inside it. */
  .body {
    display: flex;
    block-size: min(70vh, 720px);
    min-block-size: 300px;
    overflow: hidden;
  }
  /* The run's status as a word, coloured as before. */
  .st.done,
  .st.ok {
    color: var(--success);
  }
  .st.failed,
  .st.error {
    color: var(--danger);
  }
  .st.running {
    color: var(--accent-text);
  }
  .report {
    flex: 1;
    min-width: 0;
    padding: 14px 18px;
    overflow: auto;
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
    padding: 0 2px;
    border-radius: var(--radius-s);
  }
  .md :global(pre) {
    direction: ltr;
    text-align: start;
    background: var(--surface-2);
    padding: 8px 10px;
    border-radius: var(--radius-s);
    overflow: auto;
  }
  .md :global(table) {
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .md :global(td),
  .md :global(th) {
    border: 1px solid var(--border);
    padding: 2px 8px;
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
  }
</style>
