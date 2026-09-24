<script lang="ts">
  // Skills Lab → Skills → Evals / Usage for one skill, from data Otto already
  // keeps (no new API):
  //   Evals — Skills Evaluator runs whose source skill is this one: the latest
  //           score, a score trend, and the run list. "New evaluation" goes to
  //           the Evaluator tab (its form picks the skill).
  //   Usage — where the skill is installed (each copy), what Otto feature
  //           loads it (by category), and Otto's own activity on it in this
  //           workspace (reviews, evaluations, golden tasks). Otto doesn't
  //           record when an agent invokes a skill mid-session, and says so.
  import type { GoldenTask, SkillEval, SkillReview } from '../../lib/api/types';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import { skillReviewApi } from '../../lib/api/skillReview';
  import { rel } from '../../lib/stores/now.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Sparkline from '../../lib/components/Sparkline.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { sourceLabel, type SkillGroup } from './skillGroups';

  interface Props {
    group: SkillGroup;
    view: 'evals' | 'usage';
    wsId: string;
    onevaluate: () => void;
    onreview: () => void;
  }
  let { group, view, wsId, onevaluate, onreview }: Props = $props();

  let evals = $state<SkillEval[] | null>(null);
  let reviews = $state<SkillReview[] | null>(null);
  let golden = $state<GoldenTask[] | null>(null);
  let error = $state<string | null>(null);
  let loadedFor = '';

  async function load(ws: string): Promise<void> {
    error = null;
    evals = reviews = golden = null;
    const [e, r, g] = await Promise.allSettled([
      skillsEvalApi.list(ws),
      skillReviewApi.list(ws),
      skillsEvalApi.listGolden(ws),
    ]);
    evals = e.status === 'fulfilled' ? e.value : [];
    reviews = r.status === 'fulfilled' ? r.value : [];
    golden = g.status === 'fulfilled' ? g.value : [];
    if (e.status === 'rejected' && r.status === 'rejected') {
      error = e.reason instanceof Error ? e.reason.message : String(e.reason);
    }
  }
  $effect(() => {
    if (!wsId || loadedFor === wsId) return;
    loadedFor = wsId;
    void load(wsId);
  });

  const myEvals = $derived((evals ?? []).filter((x) => x.source_skill === group.name || x.dim_skill === group.name));
  const myReviews = $derived((reviews ?? []).filter((x) => x.skill_name === group.name));
  const myGolden = $derived((golden ?? []).filter((x) => x.skill === group.name));

  function scoreOf(e: SkillEval): number | null {
    const s = e.composite_score ?? e.best_score;
    return s == null || !Number.isFinite(s) ? null : s;
  }
  // Oldest → newest for the trend.
  const chrono = $derived([...myEvals].sort((a, b) => a.created_at.localeCompare(b.created_at)));
  const latest = $derived(chrono.at(-1) ?? null);
  const latestScore = $derived(latest ? scoreOf(latest) : null);
  const passCount = $derived(myEvals.filter((e) => e.status === 'done').length);
  const failCount = $derived(myEvals.filter((e) => e.status === 'error').length);

  function statusLabel(s: string): string {
    return s === 'done' ? 'Passed' : s === 'error' ? 'Failed' : s === 'running' ? 'Running' : s === 'cancelled' ? 'Cancelled' : s;
  }
  function statusTone(s: string): string {
    return s === 'done' ? 'success' : s === 'error' ? 'danger' : s === 'running' ? 'info' : 'neutral';
  }
  function fmtScore(v: number): string {
    return `${Math.round(v)}`;
  }

  const lastActivity = $derived.by(() => {
    const ts = [
      ...myEvals.map((e) => e.created_at),
      ...myReviews.map((r) => r.updated_at || r.created_at),
      ...myGolden.map((g) => g.updated_at || g.created_at),
    ].filter(Boolean);
    return ts.length ? ts.sort().at(-1)! : null;
  });

  /** What in Otto loads a skill of this category. */
  const loadedBy = $derived.by(() => {
    switch (group.category) {
      case 'review':
        return 'Offered as a lens in Code Review and PR review workflows.';
      case 'product':
        return 'Used by Product analysis (story refinement, plans).';
      case 'insights':
        return 'Runs the Insights reports.';
      case 'swarm':
        return 'Loaded by Agent Swarm role agents.';
      default:
        return 'Materialized into agent sessions from the library; agents load it by name when it fits the task.';
    }
  });

  function location(source: string): string {
    switch (source) {
      case 'library':
        return 'Otto library · <data dir>/library/skills/' + group.name;
      case 'bundled':
        return 'Compiled into Otto (read-only catalog)';
      case 'claude':
        return '~/.claude/skills/' + group.name;
      case 'codex':
        return '~/.codex/skills/' + group.name;
      case 'agy':
        return '~/.agy/skills/' + group.name;
      default:
        return `~/.${source}/skills/${group.name}`;
    }
  }
</script>

{#if error}
  <div class="inline-error" role="alert">
    <Icon name="warning" size={14} />
    <div><strong>Couldn't load activity for this skill.</strong> <span class="dim">{error}</span></div>
    <button class="btn small" onclick={() => load(wsId)}>Retry</button>
  </div>
{:else if !wsId}
  <EmptyState title="No workspace selected" body="Evaluations and reviews belong to a workspace. Pick one in the sidebar to see this skill's activity." icon="folder" />
{:else if evals == null}
  <p class="dim" role="status">Loading activity for {group.name}…</p>
{:else if view === 'evals'}
  {#if myEvals.length === 0}
    <EmptyState
      icon="target"
      title="Not evaluated yet"
      body="The Evaluator runs this skill on a real task, scores the result and suggests improvements — so you know a change helped before you ship it."
      actionLabel="New evaluation"
      actionIcon="play"
      onaction={onevaluate}
    />
  {:else}
    <div class="stats">
      <div class="card stat">
        <div class="stat-label">Latest score</div>
        <div class="stat-value">{latestScore != null ? fmtScore(latestScore) : '—'}<span class="dim unit">{latestScore != null ? ' / 100' : ''}</span></div>
        {#if chrono.filter((e) => scoreOf(e) != null).length >= 2}
          <Sparkline values={chrono.map(scoreOf)} labels={chrono.map((e) => new Date(e.created_at).toLocaleDateString())} name="Evaluation score" format={fmtScore} width={140} height={26} />
        {/if}
      </div>
      <div class="card stat">
        <div class="stat-label">Runs</div>
        <div class="stat-value">{myEvals.length}</div>
        <div class="stat-sub"><span class="ok-text">{passCount} passed</span>{#if failCount} · <span class="bad-text">{failCount} failed</span>{/if}</div>
      </div>
      <div class="card stat">
        <div class="stat-label">Last run</div>
        <div class="stat-value small" title={latest ? new Date(latest.created_at).toLocaleString() : ''}>{latest ? rel(latest.created_at) : '—'}</div>
        {#if latest}<div class="stat-sub"><span class="chip tone-{statusTone(latest.status)}">{statusLabel(latest.status)}</span></div>{/if}
      </div>
    </div>
    <div class="list-head">
      <h3 class="section-title">Runs</h3>
      <button class="btn small" onclick={onevaluate}><Icon name="play" size={12} /> New evaluation</button>
    </div>
    <ul class="rows">
      {#each [...chrono].reverse() as e (e.id)}
        <li class="rowi">
          <span class="chip tone-{statusTone(e.status)}">{statusLabel(e.status)}</span>
          <span class="grow ellipsis" title={e.task}>{e.task || e.summary || 'Evaluation'}</span>
          <span class="dim mono">{e.impl_cli}</span>
          <span class="score">{scoreOf(e) != null ? fmtScore(scoreOf(e)!) : '—'}</span>
          <span class="dim when" title={new Date(e.created_at).toLocaleString()}>{rel(e.created_at)}</span>
        </li>
      {/each}
    </ul>
  {/if}
{:else}
  <section class="block">
    <h3 class="section-title">Installed in</h3>
    <ul class="rows">
      {#each group.variants as v (v.source)}
        <li class="rowi">
          <span class="src-icon">
            {#if v.source === 'library'}<Icon name="book" size={14} />{:else if v.source === 'bundled'}<Icon name="box" size={14} />{:else}<ProviderIcon provider={v.source} size={14} />{/if}
          </span>
          <span class="src-name">{sourceLabel(v.source)}</span>
          <span class="grow dim mono ellipsis" dir="ltr" title={location(v.source)}>{location(v.source)}</span>
          {#if group.driftedSources.includes(v.source)}<span class="chip tone-warning">Drifted</span>{/if}
          {#if v.source === 'bundled'}
            <span class="chip">v{v.bundledVersion}{v.bundledState === 'not_installed' ? ' · not installed' : ''}</span>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
  <section class="block">
    <h3 class="section-title">Loaded by</h3>
    <p class="para">{loadedBy}</p>
  </section>
  <section class="block">
    <div class="list-head">
      <h3 class="section-title">Otto activity in this workspace</h3>
      {#if lastActivity}<span class="dim" title={new Date(lastActivity).toLocaleString()}>Last {rel(lastActivity)}</span>{/if}
    </div>
    <div class="stats">
      <button class="card stat link" onclick={onreview}>
        <div class="stat-label">Reviews</div>
        <div class="stat-value">{myReviews.length}</div>
        <div class="stat-sub dim">{myReviews[0] ? `Latest: ${myReviews[0].static_report?.verdict ?? myReviews[0].status}` : 'Run a multi-agent review'}</div>
      </button>
      <button class="card stat link" onclick={onevaluate}>
        <div class="stat-label">Evaluations</div>
        <div class="stat-value">{myEvals.length}</div>
        <div class="stat-sub dim">{latest ? `Last ${rel(latest.created_at)}` : 'Score it on a real task'}</div>
      </button>
      <div class="card stat">
        <div class="stat-label">Golden tasks</div>
        <div class="stat-value">{myGolden.length}</div>
        <div class="stat-sub dim">{myGolden.length ? 'Regression cases pinned to it' : 'None pinned to this skill'}</div>
      </div>
    </div>
    <p class="dim note"><Icon name="info" size={12} /> Otto doesn't record when an agent loads a skill during a session, so there's no "last used by an agent" yet — these counts are Otto's own reviews and evaluations of it.</p>
  </section>
{/if}

<style>
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 8px;
    margin-bottom: 16px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    min-width: 0;
    text-align: start;
    color: var(--text);
    font: inherit;
  }
  .stat.link {
    cursor: pointer;
  }
  .stat.link:hover {
    background: var(--hover);
  }
  .stat-label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .stat-value {
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  .stat-value.small {
    font-size: var(--fs-l);
  }
  .unit {
    font-size: var(--fs-s);
    font-weight: 400;
  }
  .stat-sub {
    font-size: var(--fs-s);
  }
  .ok-text {
    color: var(--success);
  }
  .bad-text {
    color: var(--danger);
  }
  .list-head {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .list-head .section-title {
    margin: 0;
  }
  .block {
    margin-bottom: 20px;
  }
  .block > .section-title {
    margin: 0 0 8px;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .rowi {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 34px;
    padding: 4px 12px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .rowi + .rowi {
    border-top: 1px solid var(--border);
  }
  .src-icon {
    display: inline-flex;
    color: var(--text-dim);
  }
  .src-name {
    font-weight: 500;
    min-width: 80px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .score {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    min-width: 28px;
    text-align: end;
  }
  .when {
    min-width: 56px;
    text-align: end;
  }
  .para {
    margin: 0;
    font-size: var(--fs-m);
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: var(--fs-s);
    margin: 0;
    line-height: 1.45;
  }
  .note :global(svg) {
    margin-top: 2px;
    flex: none;
  }
  .chip.tone-success {
    color: var(--success);
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 35%, transparent);
  }
  .chip.tone-danger {
    color: var(--danger);
    background: var(--danger-soft);
    border-color: color-mix(in srgb, var(--danger) 35%, transparent);
  }
  .chip.tone-info {
    color: var(--info);
    background: var(--info-soft);
    border-color: color-mix(in srgb, var(--info) 35%, transparent);
  }
  .chip.tone-warning {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
  }
  .inline-error {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .inline-error > :global(svg) {
    color: var(--danger);
  }
  .inline-error > div {
    flex: 1;
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .rowi .mono {
      display: none;
    }
  }
</style>
