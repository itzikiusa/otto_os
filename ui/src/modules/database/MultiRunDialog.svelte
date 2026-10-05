<script lang="ts">
  import { scrollRegion } from './scroll-region';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  // "Run on…" — the SAME script on several targets (connections × databases)
  // and/or once per parameter value. Four stages in one sheet:
  //   setup    → pick targets, give each placeholder its value list, options;
  //   preview  → the daemon's plan: the FINAL statement per run (placeholders
  //              substituted, ClickHouse `ON CLUSTER` injected into DDL), write
  //              and production flags, per-target cluster override;
  //   confirm  → only when a run writes to a production / read-only target:
  //              every such target and its final statement, plus a typed phrase;
  //   results  → live status per run, n ok / n failed, Stop, per-run result.
  // The run executes in the daemon (`POST /db/multi-runs`), so closing the
  // sheet does not stop it; "Recent" reopens a run. Each run is also recorded
  // in its connection's ordinary History.
  import { onDestroy, untrack } from 'svelte';
  import { toastError } from '../../lib/toastError';
  import { plural } from '../../lib/plural';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { api, ApiError } from '../../lib/api/client';
  import { mergeMultiRunJob } from '../../lib/api/db-multirun-types';
  import { pollWhileVisible, type Poller } from '../../lib/poll';
  import { runStatus } from '../../lib/status';
  import { toasts } from '../../lib/toast.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type {
    Connection,
    DbClusterMode,
    DbMultiRunBrief,
    DbMultiRunItemDetail,
    DbMultiRunJob,
    DbMultiRunParam,
    DbMultiRunPlan,
    DbMultiRunSpec,
    DbParamType,
    DbStartMultiRunReq,
    SchemaNode,
  } from '../../lib/api/types';
  import { extractVars, type SplitMode } from './sql-util';
  import {
    buildTargets,
    clusterLine,
    confirmPhrase,
    parseValues,
    phraseMatches,
    scopeOptions,
    summaryText,
    targetKey,
  } from './multi-run';

  interface Props {
    /** The script to run (the whole buffer, or the selection). */
    statement: string;
    onclose: () => void;
  }
  let { statement, onclose }: Props = $props();

  type Stage = 'setup' | 'preview' | 'confirm' | 'results';
  let stage = $state<Stage>('setup');

  const DB_KINDS = ['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse'];
  const kind = $derived(database.selectedConn?.kind ?? null);
  const isClickhouse = $derived(kind === 'clickhouse');
  const splitMode = $derived<SplitMode>(kind === 'redis' ? 'line' : 'sql');
  /** Same-engine connections — one script, one dialect. */
  const candidates = $derived<Connection[]>(
    database.connections.filter((c) => c.kind === kind && DB_KINDS.includes(c.kind)),
  );

  // ── Targets ───────────────────────────────────────────────────────────────
  /** Picked connections, in pick order, and the scopes picked on each. */
  let order = $state<string[]>([]);
  let picks = $state<Record<string, string[]>>({});
  let scopes = $state<Record<string, { value: string; label: string }[] | 'loading' | { error: string }>>({});

  // Seed: the current connection + its active database.
  untrack(() => {
    const id = database.selectedConnId;
    if (id) {
      order = [id];
      picks = { [id]: database.activeDb ? [database.activeDb] : [] };
      void loadScopes(id);
    }
  });

  async function loadScopes(id: string): Promise<void> {
    scopes = { ...scopes, [id]: 'loading' };
    try {
      const root = await api.get<SchemaNode[]>(`/connections/${encodeURIComponent(id)}/db/schema`);
      scopes = { ...scopes, [id]: scopeOptions(root) };
    } catch (e) {
      scopes = { ...scopes, [id]: { error: e instanceof Error ? e.message : String(e) } };
    }
  }

  function toggleConn(id: string, on: boolean): void {
    if (on) {
      if (!order.includes(id)) order = [...order, id];
      picks = { ...picks, [id]: picks[id] ?? [] };
      if (!scopes[id]) void loadScopes(id);
    } else {
      order = order.filter((x) => x !== id);
      const { [id]: _drop, ...rest } = picks;
      picks = rest;
    }
    plan = null;
  }

  function toggleScope(id: string, value: string, on: boolean): void {
    const cur = picks[id] ?? [];
    picks = { ...picks, [id]: on ? [...cur, value] : cur.filter((v) => v !== value) };
    plan = null;
  }

  // ClickHouse: one default for every target; per-target overrides from the preview.
  let clusterMode = $state<DbClusterMode>('auto');
  let overrides = $state<Record<string, { mode: DbClusterMode; name?: string }>>({});

  const targets = $derived(buildTargets(order, picks, { mode: clusterMode }, overrides));

  // ── Parameters ────────────────────────────────────────────────────────────
  const placeholders = $derived(extractVars(statement, splitMode));
  interface ParamDraft {
    text: string;
    type: DbParamType;
    escape: boolean;
  }
  let drafts = $state<Record<string, ParamDraft>>(
    untrack(() => {
      const tabVars = database.tab.vars ?? {};
      const out: Record<string, ParamDraft> = {};
      for (const n of extractVars(statement, database.selectedConn?.kind === 'redis' ? 'line' : 'sql')) {
        const v = tabVars[n];
        out[n] = { text: v?.value ?? '', type: v?.type ?? 'string', escape: v?.escape ?? true };
      }
      return out;
    }),
  );
  const params = $derived<DbMultiRunParam[]>(
    placeholders.map((n) => {
      const d = drafts[n] ?? { text: '', type: 'string', escape: true };
      return { name: n, values: parseValues(d.text), type: d.type, escape: d.escape };
    }),
  );
  const missing = $derived(params.filter((p) => p.values.length === 0).map((p) => p.name));
  const combos = $derived(params.reduce((n, p) => n * Math.max(1, p.values.length), 1));
  const runCount = $derived(targets.length * combos);

  // ── Options ───────────────────────────────────────────────────────────────
  let concurrency = $state(1);
  let stopOnError = $state(true);
  let maxRows = $state(500);

  function spec(): DbMultiRunSpec {
    return {
      statement,
      targets,
      params,
      max_rows: maxRows,
      mask: database.tab.mask || undefined,
    };
  }

  // ── Preview ───────────────────────────────────────────────────────────────
  let plan = $state<DbMultiRunPlan | null>(null);
  let planning = $state(false);
  let planError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  /** A cluster override changed after the preview — re-plan before running. */
  let planStale = $state(false);

  const setupBlocker = $derived(
    targets.length === 0
      ? 'Pick at least one target'
      : missing.length > 0
        ? `Give ${missing.map((m) => `:${m}`).join(', ')} at least one value`
        : runCount > 200
          ? `${runCount} runs — the limit is 200`
          : null,
  );

  async function preview(): Promise<void> {
    if (setupBlocker || planning) return;
    planning = true;
    planError = null;
    try {
      plan = await api.post<DbMultiRunPlan>('/db/multi-run/plan', spec());
      notice = null;
      planStale = false;
      stage = 'preview';
    } catch (e) {
      planError = e instanceof Error ? e.message : String(e);
    } finally {
      planning = false;
    }
  }

  function setOverride(i: number, mode: DbClusterMode, name?: string): void {
    const t = targets[i];
    if (!t) return;
    overrides = { ...overrides, [targetKey(t.connection_id, t.node)]: { mode, name } };
    planStale = true;
  }

  // ── Confirm ───────────────────────────────────────────────────────────────
  let typed = $state('');
  const phrase = $derived(plan ? confirmPhrase(plan) : '');
  const guardedRuns = $derived(plan ? plan.runs.filter((r) => r.needs_confirm) : []);

  // ── Run + results ─────────────────────────────────────────────────────────
  // `$state.raw`: the job (≤200 items + targets) is only ever REPLACED by a
  // poll answer — a deep proxy re-wrapped the whole tree every 800 ms tick.
  let job = $state.raw<DbMultiRunJob | null>(null);
  let starting = $state(false);
  let startError = $state<string | null>(null);
  let poller: Poller | null = null;

  function proceed(): void {
    if (!plan) return;
    if (plan.needs_confirm) {
      typed = '';
      stage = 'confirm';
      return;
    }
    void start(false);
  }

  async function start(confirmWrite: boolean): Promise<void> {
    if (!plan || starting) return;
    starting = true;
    startError = null;
    const body: DbStartMultiRunReq = {
      ...spec(),
      concurrency,
      stop_on_error: stopOnError,
      confirm_write: confirmWrite,
      plan_hash: plan.plan_hash,
    };
    try {
      job = await api.post<DbMultiRunJob>('/db/multi-runs', body);
      stage = 'results';
      watch(job.id);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (e instanceof ApiError && msg.startsWith('plan_changed:')) {
        // The daemon re-planned and got different statements — show them.
        stage = 'setup';
        await preview();
        notice = 'The final statements changed since the preview — review them again.';
      } else if (e instanceof ApiError && msg.startsWith('write_blocked:')) {
        stage = 'confirm';
        startError = msg.replace(/^write_blocked:\s*/, '');
      } else {
        startError = msg;
      }
    } finally {
      starting = false;
    }
  }

  function watch(id: string): void {
    poller?.stop();
    // Fast while a run is young (most finish in seconds), then 2 s: a long
    // 200-target run no longer pulls the whole job ~75×/min for its lifetime.
    const since = Date.now();
    poller = pollWhileVisible(
      async (signal) => {
        // Only the runs that changed since the last answer (`?since=`), merged
        // by index — not all ≤200 items + targets every tick.
        const since = job && job.id === id && job.seq !== undefined ? `?since=${job.seq}` : '';
        const j = mergeMultiRunJob(job, await api.get<DbMultiRunJob>(`/db/multi-runs/${encodeURIComponent(id)}${since}`, signal));
        job = j;
        if (j.status !== 'running') poller?.stop();
      },
      {
        get ms() {
          return Date.now() - since > 30_000 ? 2000 : 800;
        },
        floorMs: 500,
        immediate: false,
      },
    );
  }

  async function stop(): Promise<void> {
    if (!job) return;
    try {
      job = await api.post<DbMultiRunJob>(`/db/multi-runs/${encodeURIComponent(job.id)}/cancel`);
      poller?.now();
    } catch (e) {
      toastError('Couldn’t stop the multi-run', e);
    }
  }

  let openItem = $state<number | null>(null);
  let detail = $state<DbMultiRunItemDetail | null>(null);
  let detailError = $state<string | null>(null);
  async function showItem(index: number): Promise<void> {
    if (!job) return;
    if (openItem === index) {
      openItem = null;
      return;
    }
    openItem = index;
    detail = null;
    detailError = null;
    try {
      detail = await api.get<DbMultiRunItemDetail>(
        `/db/multi-runs/${encodeURIComponent(job.id)}/items/${index}`,
      );
    } catch (e) {
      detailError = e instanceof Error ? e.message : String(e);
    }
  }

  function openInTab(text: string): void {
    database.newTab(text);
    onclose();
  }

  function cell(v: unknown): string {
    if (v === null || v === undefined) return 'NULL';
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  }

  // ── Recent runs (the job outlives the sheet) ──────────────────────────────
  let recent = $state<DbMultiRunBrief[]>([]);
  void api
    .get<DbMultiRunBrief[]>('/db/multi-runs')
    .then((r) => (recent = r))
    .catch(() => (recent = []));

  async function reopen(id: string): Promise<void> {
    try {
      job = await api.get<DbMultiRunJob>(`/db/multi-runs/${encodeURIComponent(id)}`);
      openItem = null;
      stage = 'results';
      if (job.status === 'running') watch(job.id);
    } catch (e) {
      toastError('Couldn’t open the multi-run', e);
    }
  }

  function close(): void {
    if (job?.status === 'running') {
      toasts.info('The multi-run keeps going', 'Reopen it from Run on… → Recent.');
    }
    onclose();
  }

  onDestroy(() => poller?.stop());

  const title = $derived(
    stage === 'results' ? 'Multi-run' : stage === 'confirm' ? 'Confirm production writes' : 'Run on multiple targets',
  );
  const scriptLines = $derived(statement.split('\n').length);
</script>

<Modal {title} width={920} onclose={close}>
  <div class="mr" data-testid="multi-run">
    {#if notice && stage !== 'results'}<p class="mr-notice" role="status">{notice}</p>{/if}
    {#if planError && stage === 'preview'}
      <div class="mr-error" role="alert"><Icon name="warning" size={13} /><span>{planError}</span></div>
    {/if}
    {#if stage === 'setup'}
      {#if recent.length > 0}
        <section class="mr-sec" aria-label="Recent multi-runs">
          <h3 class="mr-h">Recent</h3>
          <ul class="mr-recent">
            {#each recent.slice(0, 4) as r (r.id)}
              <li>
                <button class="mr-link" onclick={() => reopen(r.id)} title="Open this multi-run’s results">
                  <StatusBadge status={runStatus(r.status)} variant="text" />
                  <span class="mono mr-clip">{r.statement_preview}</span>
                  <span class="mr-dim">{summaryText(r.summary)}</span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      <section class="mr-sec" aria-label="Script">
        <h3 class="mr-h">Script <span class="mr-dim">· {plural(scriptLines, 'line')}</span></h3>
        <pre class="mr-code mono" use:scrollRegion={'Statement'}>{statement}</pre>
      </section>

      <section class="mr-sec" aria-label="Targets">
        <h3 class="mr-h">Targets <span class="mr-dim">· {targets.length} selected</span></h3>
        {#if candidates.length === 0}
          <p class="mr-dim">No {kind ?? ''} connections to choose from.</p>
        {:else}
          <ul class="mr-conns">
            {#each candidates as c (c.id)}
              {@const on = order.includes(c.id)}
              {@const sc = scopes[c.id]}
              <li class="mr-conn" class:on>
                <label class="mr-check">
                  <input type="checkbox" checked={on} onchange={(e) => toggleConn(c.id, e.currentTarget.checked)} />
                  <span class="mr-name">{c.name}</span>
                  <EnvBadge env={c.environment} readOnly={c.read_only} />
                </label>
                {#if on}
                  <div class="mr-scopes">
                    {#if sc === 'loading'}
                      <span class="mr-dim"><span class="spinner" style="--spinner-size: 11px" aria-hidden="true"></span> Loading databases…</span>
                    {:else if sc && 'error' in sc}
                      <span class="mr-err">Couldn’t list databases: {sc.error}</span>
                      <button class="btn small ghost" onclick={() => loadScopes(c.id)}>Retry</button>
                    {:else if sc && sc.length > 0}
                      {#each sc as s (s.value)}
                        <label class="mr-scope">
                          <input
                            type="checkbox"
                            checked={(picks[c.id] ?? []).includes(s.value)}
                            onchange={(e) => toggleScope(c.id, s.value, e.currentTarget.checked)}
                          />
                          <span class="mono">{s.label}</span>
                        </label>
                      {/each}
                    {/if}
                    {#if (picks[c.id] ?? []).length === 0}
                      <span class="mr-dim">No database picked — runs on the connection’s default database.</span>
                    {/if}
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if isClickhouse}
          <label class="mr-inline">
            <span>ON CLUSTER for DDL</span>
            <select class="input" bind:value={clusterMode} aria-label="ON CLUSTER for DDL">
              <option value="auto">Auto-detect per target</option>
              <option value="off">Off — never inject</option>
            </select>
            <span class="mr-dim">Only CREATE / ALTER / DROP / RENAME / TRUNCATE… — never INSERT or SELECT. You review every statement before it runs.</span>
          </label>
        {/if}
      </section>

      <section class="mr-sec" aria-label="Parameters">
        <h3 class="mr-h">Parameters</h3>
        {#if placeholders.length === 0}
          <p class="mr-dim">
            No placeholders. Add <span class="mono">:name</span>, <span class="mono">{'{name}'}</span> or
            <span class="mono">{'{{name}}'}</span> to the script to run it once per value.
          </p>
        {:else}
          <p class="mr-dim">One value runs once; several values run once each (comma-separated, or one per line).</p>
          {#each placeholders as n (n)}
            {#if drafts[n]}
            {@const count = parseValues(drafts[n].text).length}
            <div class="mr-param">
              <span class="mr-pname mono" title={n}>:{n}</span>
              <textarea
                class="input mr-pvals mono"
                rows={Math.min(4, Math.max(1, drafts[n].text.split('\n').length))}
                bind:value={drafts[n].text}
                placeholder="1, 2, 3"
                spellcheck="false"
                aria-label="Values for {n}"
                oninput={() => (plan = null)}
              ></textarea>
              <select class="input mr-ptype" bind:value={drafts[n].type} aria-label="Type for {n}" title="string = quoted literal · number = validated number · raw = verbatim">
                <option value="string">string</option>
                <option value="number">number</option>
                <option value="raw">raw</option>
              </select>
              {#if drafts[n].type === 'string'}
                <label class="mr-esc" title="Escape quotes inside the value">
                  <input type="checkbox" bind:checked={drafts[n].escape} /> esc
                </label>
              {/if}
              <span class="mr-dim mr-count">{plural(count, 'value')}</span>
            </div>
            {/if}
          {/each}
        {/if}
      </section>

      <section class="mr-sec mr-opts" aria-label="Options">
        <label class="mr-inline">
          <span>Run</span>
          <select class="input" bind:value={concurrency} aria-label="Concurrency">
            <option value={1}>one at a time</option>
            <option value={2}>2 in parallel</option>
            <option value={4}>4 in parallel</option>
            <option value={8}>8 in parallel</option>
          </select>
        </label>
        <label class="mr-inline">
          <input type="checkbox" bind:checked={stopOnError} />
          <span>Stop at the first failure</span>
        </label>
        <label class="mr-inline">
          <span>Rows kept per run</span>
          <select class="input" bind:value={maxRows} aria-label="Rows kept per run">
            <option value={100}>100</option>
            <option value={500}>500</option>
            <option value={1000}>1,000</option>
            <option value={5000}>5,000</option>
          </select>
        </label>
      </section>

      {#if planError}
        <div class="mr-error" role="alert">
          <Icon name="warning" size={13} />
          <span>{planError}</span>
          <button class="btn small ghost" onclick={preview}>Retry</button>
        </div>
      {/if}
    {:else if stage === 'preview' && plan}
      <p class="mr-lead">
        <strong>{plural(plan.runs.length, 'run')}</strong> on
        <strong>{plural(plan.targets.length, 'target')}</strong>
        {#if plan.write_count > 0}· <span class="mr-warn-text">{plural(plan.write_count, 'write')}/DDL</span>{/if}
        · {concurrency === 1 ? 'one at a time' : `${concurrency} in parallel`}
        · {stopOnError ? 'stops at the first failure' : 'continues past failures'}
      </p>
      {#each plan.warnings as w (w)}
        <p class="mr-warning"><Icon name="warning" size={12} /> {w}</p>
      {/each}

      {#if isClickhouse}
        <section class="mr-sec" aria-label="Clusters">
          <h3 class="mr-h">Clusters</h3>
          <ul class="mr-clusters">
            {#each plan.targets as t (t.index)}
              {@const c = t.cluster}
              {@const ov = overrides[targetKey(t.connection_id, t.node)]}
              <li class="mr-cl">
                <span class="mr-name">{t.label}</span>
                <EnvBadge env={t.environment} readOnly={t.read_only} />
                {#if c}
                  <span class="mr-dim" title={c.note ?? ''}>{clusterLine(c)}</span>
                  <select
                    class="input mr-clsel"
                    aria-label="ON CLUSTER for {t.label}"
                    value={ov?.mode === 'custom' ? `custom:${ov.name ?? ''}` : (ov?.mode ?? 'auto')}
                    onchange={(e) => {
                      const v = e.currentTarget.value;
                      if (v.startsWith('custom:')) setOverride(t.index, 'custom', v.slice(7));
                      else setOverride(t.index, v as DbClusterMode);
                    }}
                  >
                    <option value="auto">Auto{c.detected ? ` (${c.detected})` : ''}</option>
                    <option value="off">Off</option>
                    {#each c.candidates as name (name)}
                      <option value={`custom:${name}`}>ON CLUSTER {name}</option>
                    {/each}
                    {#if ov?.mode === 'custom' && ov.name && !c.candidates.includes(ov.name)}
                      <option value={`custom:${ov.name}`}>ON CLUSTER {ov.name}</option>
                    {/if}
                  </select>
                  <input
                    class="input mr-clname mono"
                    placeholder="other cluster…"
                    aria-label="Custom cluster for {t.label}"
                    onkeydown={(e) => {
                      if (e.key === 'Enter' && e.currentTarget.value.trim()) setOverride(t.index, 'custom', e.currentTarget.value.trim());
                    }}
                    onblur={(e) => {
                      if (e.currentTarget.value.trim()) setOverride(t.index, 'custom', e.currentTarget.value.trim());
                    }}
                  />
                {/if}
              </li>
            {/each}
          </ul>
          {#if planStale}
            <p class="mr-notice" role="status">Cluster choice changed — update the preview to see the final statements.</p>
            <button class="btn small" onclick={preview} disabled={planning}>
              <Icon name="refresh" size={12} /> Update preview
            </button>
          {/if}
        </section>
      {/if}

      <section class="mr-sec" aria-label="Final statements">
        <h3 class="mr-h">Final statements</h3>
        <ul class="mr-runs">
          {#each plan.runs as r (r.index)}
            {@const t = plan.targets[r.target]}
            <li>
              <details open={plan.runs.length <= 6}>
                <summary class="mr-run-sum">
                  <span class="mr-idx mono">#{r.index + 1}</span>
                  <span class="mr-name">{r.label}</span>
                  <EnvBadge env={t.environment} readOnly={t.read_only} />
                  {#if r.is_write}<Badge tone="warn" label="Write" />{/if}
                  {#if r.needs_confirm}<Badge tone="bad" label="Needs confirm" />{/if}
                  {#if r.on_cluster?.length}<Badge tone="info" label={`ON CLUSTER ×${r.on_cluster.length}`} />{/if}
                </summary>
                <pre class="mr-code mono" use:scrollRegion={`Statement for ${r.label}`}>{r.statement}</pre>
                {#each r.cluster_skipped ?? [] as s (s)}
                  <p class="mr-dim mr-small">Not rewritten: {s}</p>
                {/each}
              </details>
            </li>
          {/each}
        </ul>
      </section>
      {#if startError}<div class="mr-error" role="alert"><Icon name="warning" size={13} /><span>{startError}</span></div>{/if}
    {:else if stage === 'confirm' && plan}
      <div class="mr-danger" role="alert">
        <Icon name="warning" size={14} />
        <span>
          {plural(guardedRuns.length, 'run')} will WRITE / change the schema on production or
          read-only targets. This can modify or destroy data. Check every target and statement below.
        </span>
      </div>
      <ul class="mr-runs">
        {#each guardedRuns as r (r.index)}
          {@const t = plan.targets[r.target]}
          <li class="mr-guarded">
            <div class="mr-run-sum">
              <span class="mr-idx mono">#{r.index + 1}</span>
              <span class="mr-name">{r.label}</span>
              <EnvBadge env={t.environment} readOnly={t.read_only} />
            </div>
            <pre class="mr-code mono" use:scrollRegion={`Statement for ${r.label}`}>{r.statement}</pre>
          </li>
        {/each}
      </ul>
      <label class="mr-typed">
        <span>Type <strong class="mono">{phrase}</strong> to confirm</span>
        <input
          class="input mono"
          bind:value={typed}
          placeholder={phrase}
          spellcheck="false"
          aria-label="Type {phrase} to confirm"
          onkeydown={(e) => {
            if (e.key === 'Enter' && phraseMatches(typed, phrase)) void start(true);
          }}
        />
      </label>
      {#if startError}<div class="mr-error" role="alert"><Icon name="warning" size={13} /><span>{startError}</span></div>{/if}
    {:else if stage === 'results' && job}
      <div class="mr-status">
        <StatusBadge status={runStatus(job.status === 'done' ? (job.summary.failed > 0 ? 'failed' : 'done') : job.status)} />
        <span class="mr-lead">{summaryText(job.summary)}</span>
        <span class="mr-dim">of {job.summary.total}</span>
        <span class="mr-dim mr-clip mono" title={job.statement_preview}>{job.statement_preview}</span>
      </div>
      {#if job.summary.total > 0}
        <div
          class="mr-bar"
          role="progressbar"
          aria-label="Multi-run progress"
          aria-valuemin={0}
          aria-valuemax={job.summary.total}
          aria-valuenow={job.summary.ok + job.summary.failed + job.summary.skipped + job.summary.cancelled}
        >
          <span class="ok" style:inline-size="{(job.summary.ok / job.summary.total) * 100}%"></span>
          <span class="bad" style:inline-size="{(job.summary.failed / job.summary.total) * 100}%"></span>
        </div>
      {/if}
      <ul class="mr-results">
        {#each job.items as it (it.index)}
          {@const t = job.targets[it.target]}
          <li class="mr-res" class:open={openItem === it.index}>
            <button
              class="mr-res-row"
              onclick={() => showItem(it.index)}
              aria-expanded={openItem === it.index}
              title="Show this run’s statement and result"
            >
              <span class="mr-idx mono">#{it.index + 1}</span>
              <span class="mr-name">{it.label}</span>
              {#if t}<EnvBadge env={t.environment} readOnly={t.read_only} />{/if}
              <StatusBadge status={runStatus(it.status)} variant="text" />
              <span class="mr-dim mr-meta">
                {#if it.duration_ms != null}{it.duration_ms} ms{/if}
                {#if it.rows_affected != null} · {it.rows_affected} affected{:else if it.row_count != null} · {plural(it.row_count, 'row')}{/if}
              </span>
              {#if it.error}<span class="mr-err mr-clip" title={it.error}>{it.error}</span>
              {:else if it.message}<span class="mr-dim mr-clip" title={it.message}>{it.message}</span>{/if}
            </button>
            {#if openItem === it.index}
              <div class="mr-detail">
                {#if detailError}
                  <div class="mr-error" role="alert"><Icon name="warning" size={13} /><span>{detailError}</span></div>
                {:else if !detail}
                  <Skeleton rows={3} height={20} label="result details" />
                {:else}
                  <pre class="mr-code mono" use:scrollRegion={'Final statement'}>{detail.statement}</pre>
                  <div class="mr-detail-actions">
                    <button class="btn small ghost" onclick={() => openInTab(detail!.statement)} title="Open this final statement in a new query tab">
                      <Icon name="external" size={12} /> Open in a new tab
                    </button>
                  </div>
                  {#if detail.result && detail.result.columns.length > 0}
                    <div class="mr-table-wrap" use:scrollRegion={`Result of run ${it.index + 1}`}>
                      <table class="mr-table mono">
                        <thead>
                          <tr>{#each detail.result.columns as col (col.name)}<th>{col.name}</th>{/each}</tr>
                        </thead>
                        <tbody>
                          {#each detail.result.rows.slice(0, 200) as row, ri (ri)}
                            <tr>{#each row as v, ci (ci)}<td title={cell(v)}>{cell(v)}</td>{/each}</tr>
                          {/each}
                        </tbody>
                      </table>
                    </div>
                    {#if detail.result.rows.length > 200 || detail.result.truncated}
                      <p class="mr-dim mr-small">Showing the first {Math.min(200, detail.result.rows.length)} rows — open the statement in a tab for the full result.</p>
                    {/if}
                  {:else if it.result_dropped}
                    <p class="mr-dim mr-small">Rows not kept (this multi-run reached its memory budget) — the counts above are exact.</p>
                  {:else if it.status === 'ok'}
                    <p class="mr-dim mr-small">{detail.result?.message ?? 'Done — no rows returned.'}</p>
                  {/if}
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  {#snippet footer()}
    {#if stage === 'setup'}
      <button class="btn" onclick={close}>Cancel</button>
      <button
        class="btn primary"
        onclick={preview}
        disabled={!!setupBlocker || planning}
        title={setupBlocker ?? 'Show the final statement for every run before anything executes'}
      >
        {planning ? 'Preparing…' : `Preview ${plural(runCount, 'run')}`}
      </button>
    {:else if stage === 'preview' && plan}
      <button class="btn" onclick={() => (stage = 'setup')}>Back</button>
      <button
        class="btn primary"
        class:danger={plan.needs_confirm}
        onclick={proceed}
        disabled={starting || planning || planStale}
        title={planStale ? 'Update the preview first' : plan.needs_confirm ? 'Writes to production / read-only targets — you will confirm each one' : 'Run every statement shown'}
      >
        {starting ? 'Starting…' : plan.needs_confirm ? `Review ${plural(guardedRuns.length, 'guarded write')}…` : `Run ${plan.runs.length}`}
      </button>
    {:else if stage === 'confirm' && plan}
      <button class="btn" onclick={() => (stage = 'preview')}>Back</button>
      <button
        class="btn primary danger"
        onclick={() => start(true)}
        disabled={!phraseMatches(typed, phrase) || starting}
        title={phraseMatches(typed, phrase) ? 'Run all planned statements, including the writes above' : `Type ${phrase} first`}
      >
        {starting ? 'Starting…' : `Run ${plan.runs.length}`}
      </button>
    {:else if stage === 'results' && job}
      {#if job.status === 'running'}
        <button class="btn danger" onclick={stop} title="Stop dispatching runs and cancel the ones in flight">
          <Icon name="stop" size={12} /> Stop
        </button>
      {:else}
        <button class="btn" onclick={() => { job = null; openItem = null; stage = 'setup'; }}>New run</button>
      {/if}
      <button class="btn primary" onclick={close}>Close</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .mr {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-inline-size: 0;
  }
  .mr-sec {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-inline-size: 0;
  }
  .mr-h {
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .mr-dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .mr-small {
    margin: 2px 0 0;
    font-size: var(--fs-xs);
  }
  .mr-lead {
    margin: 0;
    font-size: var(--fs-m);
  }
  .mr-notice,
  .mr-warning {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--warning);
  }
  .mr-warn-text {
    color: var(--warning);
  }
  .mr-err {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .mr-error,
  .mr-danger {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
    line-height: 1.45;
  }
  .mr-code {
    margin: 0;
    padding: 8px 10px;
    max-block-size: 180px;
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .mr-conns,
  .mr-recent,
  .mr-runs,
  .mr-results,
  .mr-clusters {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mr-conns {
    max-block-size: 260px;
    overflow-y: auto;
  }
  .mr-conn {
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .mr-conn.on {
    border-color: var(--accent-solid);
    background: var(--accent-soft);
  }
  .mr-check,
  .mr-scope,
  .mr-inline,
  .mr-esc {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .mr-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }
  .mr-scopes {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    padding-block-start: 6px;
    padding-inline-start: 22px;
  }
  .mr-opts {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 10px 18px;
  }
  .mr-param {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    flex-wrap: wrap;
  }
  .mr-pname {
    flex: 0 0 120px;
    padding-block-start: 6px;
    font-size: var(--fs-s);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mr-pvals {
    flex: 1 1 260px;
    min-inline-size: 0;
    resize: vertical;
  }
  .mr-ptype {
    flex: 0 0 90px;
  }
  .mr-count {
    padding-block-start: 6px;
  }
  .mr-link,
  .mr-res-row {
    display: flex;
    align-items: center;
    gap: 8px;
    inline-size: 100%;
    min-inline-size: 0;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .mr-link:hover,
  .mr-res-row:hover {
    background: var(--hover);
  }
  .mr-link:focus-visible,
  .mr-res-row:focus-visible {
    outline: 2px solid var(--accent-solid);
    outline-offset: -2px;
  }
  .mr-clip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
    flex: 1 1 auto;
  }
  .mr-run-sum {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 4px 0;
    cursor: pointer;
    font-size: var(--fs-s);
  }
  .mr-idx {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .mr-cl {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .mr-clsel {
    max-inline-size: 220px;
  }
  .mr-clname {
    inline-size: 150px;
  }
  .mr-guarded {
    padding: 6px 8px;
    border: 1px solid var(--danger);
    border-radius: var(--radius-s);
  }
  .mr-typed {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .mr-status {
    display: flex;
    align-items: center;
    gap: 10px;
    min-inline-size: 0;
  }
  .mr-bar {
    display: flex;
    block-size: 6px;
    border-radius: var(--radius-s);
    background: var(--surface-3);
    overflow: hidden;
  }
  .mr-bar .ok {
    background: var(--success);
  }
  .mr-bar .bad {
    background: var(--danger);
  }
  .mr-results {
    max-block-size: 52vh;
    overflow-y: auto;
  }
  .mr-res.open {
    background: var(--surface-2);
    border-radius: var(--radius-s);
  }
  .mr-meta {
    white-space: nowrap;
  }
  .mr-detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 8px 10px;
  }
  .mr-detail-actions {
    display: flex;
    gap: 6px;
  }
  .mr-table-wrap {
    max-block-size: 260px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .mr-table {
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .mr-table th,
  .mr-table td {
    padding: 2px 8px;
    border-block-end: 1px solid var(--separator);
    text-align: start;
    white-space: nowrap;
    max-inline-size: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mr-table th {
    position: sticky;
    inset-block-start: 0;
    background: var(--surface);
    font-weight: 600;
  }
  @media (max-width: 640px) {
    .mr-pname {
      flex-basis: 100%;
    }
  }
</style>
