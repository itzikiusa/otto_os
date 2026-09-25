<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  // Usage dashboard (root-only): provider/day/session token rollups, system
  // CPU/RAM metrics, and the embedded-ClickHouse install/retention controls.
  // All data comes from the daemon's /usage/* endpoints (otto-usage engine).
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatBytes, formatCount } from '../../lib/metric-format';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { usage } from '../../lib/api/usage.svelte';
  import type { UsageBudgetConfig } from '../../lib/api/usage.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import { budgetBus } from '../../lib/events.svelte';
  // Work-graph attribution drilldown + cost forecast (B1).
  import AttributionDrilldown from './AttributionDrilldown.svelte';
  import CostForecastChip from './CostForecastChip.svelte';

  // Navigate to a session from the top-sessions table (click-through drill-down).
  function openSession(sessionId: string): void {
    // Only known Otto sessions (those with a title or kind) can be navigated to.
    ws.navigateToSession(sessionId);
  }

  const WINDOWS = [
    { days: 7, label: '7d' },
    { days: 30, label: '30d' },
    { days: 90, label: '90d' },
    { days: 180, label: '180d' },
  ];

  // Editable config (seeded from status, applied on Save).
  let retention = $state(180);
  let interval = $state(60);
  let chPath = $state('');
  let configOpen = $state(false);

  // Editable budget config — a local copy of usage.budgets.config, seeded when
  // budgets load and saved back on demand. Enforcement is opt-in (default off).
  let budgetCfg: UsageBudgetConfig = $state({
    enforce: false,
    block_on_exceed: false,
    window_days: 30,
    workspaces: [],
    providers: [],
  });
  let budgetsOpen = $state(false);
  let budgetsDirty = $state(false);

  // Live budget-exceeded banner — driven by the BudgetExceeded WS event via
  // budgetBus. The banner is dismissible; a "recovered" direction auto-clears
  // it. Null means no active alert.
  type BudgetAlert = {
    provider: string;
    spendUsd: number;
    capUsd: number;
    direction: string;
  };
  let budgetAlert: BudgetAlert | null = $state(null);
  // Track which tick we last processed to avoid re-applying the same event.
  let budgetAlertTick = $state(0);

  $effect(() => {
    const tick = budgetBus.tick;
    if (tick === 0 || tick === budgetAlertTick) return;
    budgetAlertTick = tick;
    if (budgetBus.direction === 'recovered') {
      budgetAlert = null;
    } else {
      budgetAlert = {
        provider: budgetBus.provider,
        spendUsd: budgetBus.spendUsd,
        capUsd: budgetBus.capUsd,
        direction: budgetBus.direction,
      };
    }
  });

  function dismissBudgetAlert(): void {
    budgetAlert = null;
  }

  // The Storage & retention panel renders at the TOP of the body and scrolls
  // into view when opened from the header gear (it used to render below the
  // sessions table, so the gear looked like it did nothing).
  let settingsEl: HTMLElement | undefined = $state();
  function toggleSettings(): void {
    configOpen = !configOpen;
    if (configOpen) queueMicrotask(() => settingsEl?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }));
  }

  /** Downloads land in the user's Downloads folder with no visible change on
   *  the page — say what was exported. */
  function exported(what: string, file: string): void {
    toasts.success(`Exported ${what}`, file);
  }
  function exportProviders(): void {
    usage.exportProvidersCsv();
    exported('providers as CSV', `otto-usage-providers-${usage.days}d.csv`);
  }
  function exportDaily(): void {
    usage.exportDailyCsv();
    exported('daily cost as CSV', `otto-usage-daily-${usage.days}d.csv`);
  }
  function exportSessions(): void {
    usage.exportSessionsCsv();
    exported('sessions as CSV', `otto-usage-sessions-${usage.days}d.csv`);
  }
  function exportSummary(): void {
    usage.exportSummaryJson();
    exported('the usage summary as JSON', `otto-usage-summary-${usage.days}d.json`);
  }

  /** Throw away unsaved budget edits and re-seed from the server copy. */
  function discardBudgets(): void {
    budgetsDirty = false;
    if (usage.budgets) budgetCfg = structuredClone($state.snapshot(usage.budgets.config));
    budgetsOpen = false;
  }

  onMount(() => {
    if (auth.isRoot) void usage.loadAll();
    return () => {
      // Tear down auto-refresh on unmount so we don't poll in the background.
      usage.setAutoRefresh(false);
    };
  });

  // Mirror server status into the editable fields whenever it refreshes.
  $effect(() => {
    const s = usage.status;
    if (s) {
      retention = s.retention_days;
      interval = s.metrics_interval_secs;
      chPath = s.binary ?? '';
    }
  });

  // Seed the editable budget config from the server, unless the user has made
  // local edits (don't clobber in-flight changes on a background refresh).
  $effect(() => {
    const b = usage.budgets;
    if (b && !budgetsDirty) {
      budgetCfg = structuredClone($state.snapshot(b.config));
    }
  });

  function addWsBudget(): void {
    budgetsDirty = true;
    budgetCfg.workspaces = [...budgetCfg.workspaces, { workspace_id: '', monthly_usd: 0 }];
  }
  function addProviderBudget(): void {
    budgetsDirty = true;
    budgetCfg.providers = [...budgetCfg.providers, { provider: '', monthly_usd: 0 }];
  }
  function removeWsBudget(i: number): void {
    budgetsDirty = true;
    budgetCfg.workspaces = budgetCfg.workspaces.filter((_, j) => j !== i);
  }
  function removeProviderBudget(i: number): void {
    budgetsDirty = true;
    budgetCfg.providers = budgetCfg.providers.filter((_, j) => j !== i);
  }
  async function saveBudgets(): Promise<void> {
    if (usage.savingBudgets) return;
    const submitted = JSON.stringify(budgetCfg);
    // Drop blank rows before saving (no key or no cap = nothing to enforce).
    const cfg: UsageBudgetConfig = {
      ...budgetCfg,
      window_days: budgetCfg.window_days || 30,
      workspaces: budgetCfg.workspaces.filter((b) => b.workspace_id && b.monthly_usd > 0),
      providers: budgetCfg.providers.filter((b) => b.provider && b.monthly_usd > 0),
    };
    const saved = await usage.saveBudgets(cfg);
    // The response acknowledges the submitted draft, never newer local edits.
    if (saved && JSON.stringify(budgetCfg) === submitted) budgetsDirty = false;
  }

  // Workspace name for a budget row's id (falls back to the id). The hidden
  // scratch workspace is not in `ws.workspaces`, so name it explicitly.
  function wsName(id: string): string {
    return (
      ws.workspaces.find((w) => w.id === id)?.name ??
      (id === SCRATCH_WORKSPACE_ID ? (ws.scratch?.name ?? 'Scratch') : id)
    );
  }
  // Provider choices for the budget editor (installed CLIs + any already used).
  const providerChoices = $derived.by(() => {
    const set = new Set<string>((usage.summary?.providers ?? []).map((p) => p.provider));
    for (const t of auth.meta?.providers ?? []) set.add(t);
    return [...set].filter(Boolean);
  });

  // Shared formatters (content.md → numbers): 1.2k / 3.4M, KB/MB/GB.
  const fmtNum = formatCount;
  /** "1 session" / "3 sessions". */
  function plural(n: number, one: string, many = one + 's'): string {
    return `${formatCount(n)} ${n === 1 ? one : many}`;
  }
  function fmtCost(n: number): string {
    if (n === 0) return '$0';
    if (n < 0.01) return '<$0.01';
    return '$' + n.toFixed(n < 100 ? 2 : 0);
  }
  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  function shortDay(iso: string): string {
    // "2026-06-16" → "Jun 16" (a bare "06-16" reads as either order).
    const m = iso.match(/^\d{4}-(\d{2})-(\d{2})/);
    if (!m) return iso;
    return `${MONTHS[parseInt(m[1], 10) - 1] ?? m[1]} ${parseInt(m[2], 10)}`;
  }
  function fmtLastActive(iso: string): string {
    // "2026-06-16 14:32:05.123" → "Jun 16, 14:32" (date matters: window is up to
    // 180d, so time-only is ambiguous). Format the stored value directly to avoid
    // a timezone shift.
    const m = iso.match(/(\d{4})-(\d{2})-(\d{2})[ T](\d{2}:\d{2})/);
    if (!m) return iso;
    const mon = MONTHS[parseInt(m[2], 10) - 1] ?? m[2];
    return `${mon} ${parseInt(m[3], 10)}, ${m[4]}`;
  }

  // ── Daily cost SVG chart ──────────────────────────────────────────────────
  // The chart is hand-rolled SVG (no dependencies). It renders cost on the
  // y-axis (labeled ticks), days on the x-axis (thinned for 30d/90d), gridlines
  // at each y-tick, and a per-point tooltip via <title> (shown on hover by the
  // browser). Works at 7d/30d/90d windows.

  // Chart viewport (inner drawing area, inside the axis labels).
  const SVG_W = 500;
  const SVG_H = 110;
  const AXIS_L = 52;  // left margin for y-axis labels
  const AXIS_B = 22;  // bottom margin for x-axis labels

  const dailyCosts = $derived((usage.summary?.daily ?? []).map((d) => d.cost_usd));
  const dailyMaxCost = $derived(Math.max(...dailyCosts, 0));
  const dailyDays = $derived(usage.summary?.daily ?? []);

  // Y-axis: 4 ticks from 0 to ceiling. Round the top tick to a "nice" value.
  // The step never drops below one cent: sub-cent steps all format as
  // "<$0.01", so a near-zero window used to print the same label three times.
  const yTicks = $derived.by(() => {
    const top = dailyMaxCost;
    const raw = Math.max(top / 3, 0.01);
    // Pick a magnitude step that gives readable labels.
    const mag = Math.pow(10, Math.floor(Math.log10(raw || 1)));
    const nice = Math.ceil(raw / mag) * mag;
    return [0, nice, nice * 2, nice * 3];
  });

  function svgY(cost: number): number {
    const plotH = SVG_H - AXIS_B;
    const max = yTicks[yTicks.length - 1] || 1;
    return plotH - (cost / max) * plotH;
  }
  // Bars sit at the CENTRE of equal bands: pinning the first/last bar to the
  // plot edges put half of the first bar over the y-axis labels and half of
  // the last one (and its date) past the right edge.
  function svgX(i: number, total: number): number {
    const plotW = SVG_W - AXIS_L;
    return AXIS_L + ((i + 0.5) / Math.max(1, total)) * plotW;
  }

  // X-axis: thin labels so they don't overlap (max ~8 visible).
  function showXLabel(i: number, total: number): boolean {
    if (total <= 8) return true;
    const step = Math.ceil(total / 8);
    return i % step === 0 || i === total - 1;
  }

  // ── Metric sparkline path builder ─────────────────────────────────────────
  function sparkPath(values: number[], max: number, w: number, h: number): string {
    if (values.length === 0) return '';
    const m = Math.max(max, 1);
    const step = values.length > 1 ? w / (values.length - 1) : 0;
    return values
      .map((v, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)},${(h - (v / m) * h).toFixed(1)}`)
      .join(' ');
  }
  const cpuSeries = $derived(usage.metrics.map((p) => p.cpu_pct));
  const memSeries = $derived(usage.metrics.map((p) => p.mem_pct));
  const latest = $derived(usage.metrics.at(-1) ?? null);

  // ── Token category breakdown (input · cache-write · cache-read · output) ────
  // One shared model so the headline, provider bars, daily chart, and session
  // rows all split tokens the same way and use the same colors (see legend).
  type TokenParts = {
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cache_write_tokens: number;
  };
  const TOKEN_CATS = [
    { label: 'Input', color: 'var(--cat-1)', pick: (o: TokenParts) => o.input_tokens },
    { label: 'Cache write', color: 'var(--cat-2)', pick: (o: TokenParts) => o.cache_write_tokens },
    { label: 'Cache read', color: 'var(--cat-3)', pick: (o: TokenParts) => o.cache_read_tokens },
    { label: 'Output', color: 'var(--cat-4)', pick: (o: TokenParts) => o.output_tokens },
  ] as const;

  type Seg = { label: string; color: string; v: number; pct: number };
  function tokenSegs(o: TokenParts): Seg[] {
    const total = o.input_tokens + o.output_tokens + o.cache_read_tokens + o.cache_write_tokens;
    return TOKEN_CATS.map((c) => {
      const v = c.pick(o);
      return { label: c.label, color: c.color, v, pct: total > 0 ? (v / total) * 100 : 0 };
    });
  }
  function breakdownTitle(o: TokenParts): string {
    return tokenSegs(o)
      .map((s) => `${s.label} ${s.v.toLocaleString()}`)
      .join(' · ');
  }
  // ── Per-feature (by-kind) labels ──────────────────────────────────────────
  // Friendly display names for the feature buckets the server emits. Unknown
  // values pass through capitalized.
  const FEATURE_LABELS: Record<string, string> = {
    review: 'Code review',
    product: 'Product AI',
    channel: 'Channels',
    agent: 'Ad-hoc agents',
    connection: 'Connections',
    external: 'External',
    swarm: 'Swarm',
  };
  function featureLabel(k: string): string {
    return FEATURE_LABELS[k] ?? (k ? k[0].toUpperCase() + k.slice(1) : 'Other');
  }

  // The summary carries the same four numbers under total_* names.
  function summaryParts(s: {
    total_input_tokens: number;
    total_output_tokens: number;
    total_cache_read_tokens: number;
    total_cache_write_tokens: number;
  }): TokenParts {
    return {
      input_tokens: s.total_input_tokens,
      output_tokens: s.total_output_tokens,
      cache_read_tokens: s.total_cache_read_tokens,
      cache_write_tokens: s.total_cache_write_tokens,
    };
  }
</script>

<div class="usage">
  <PageHeader
    title="Usage"
    subtitle="Tokens, estimated cost and system load"
  >
    {#snippet tabs()}
      {#if usage.status?.available}
        <div
          class="segmented"
          role="group"
          aria-label="Sessions to count"
          title="Otto: only sessions run inside Otto. All: every Claude/Codex session on this Mac."
        >
          <button aria-pressed={usage.ottoOnly} class:active={usage.ottoOnly} onclick={() => usage.setOttoOnly(true)}>
            Otto
          </button>
          <button aria-pressed={!usage.ottoOnly} class:active={!usage.ottoOnly} onclick={() => usage.setOttoOnly(false)}>
            All
          </button>
        </div>
        <div class="segmented" role="group" aria-label="Time window">
          {#each WINDOWS as w (w.days)}
            <button
              aria-pressed={usage.days === w.days}
              class:active={usage.days === w.days}
              title="Last {w.days} days"
              onclick={() => usage.setDays(w.days)}
            >
              {w.label}
            </button>
          {/each}
        </div>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if usage.status?.available}
        <button
          class="icon-btn"
          onclick={() => usage.loadAll()}
          disabled={usage.loading}
          title={usage.loading ? 'Refreshing…' : 'Refresh'}
          aria-label="Refresh"
          data-icon="refresh"
          data-label="Refresh"
        >
          <Icon name="refresh" size={14} />
        </button>
        <button
          class="btn small"
          class:on={usage.autoRefresh}
          aria-pressed={usage.autoRefresh}
          onclick={() => usage.setAutoRefresh(!usage.autoRefresh)}
          title={usage.autoRefresh ? 'Refreshing every 60 s — click to stop' : 'Refresh this page every 60 s'}
          data-icon="clock"
          data-label={usage.autoRefresh ? 'Stop auto-refresh' : 'Auto-refresh every 60 s'}
        >
          <Icon name="clock" size={12} />
          {usage.autoRefresh ? 'Live' : 'Auto-refresh'}
        </button>
        <button
          class="btn small"
          disabled={!usage.summary}
          onclick={exportSummary}
          title={usage.summary ? 'Download the full summary as JSON' : 'Nothing to export yet'}
          data-icon="download"
          data-label="Export summary as JSON"
          data-overflow="-1"
        >
          <Icon name="download" size={12} /> Export
        </button>
        <button
          class="icon-btn"
          class:on={configOpen}
          aria-pressed={configOpen}
          onclick={toggleSettings}
          title="Storage and retention settings"
          aria-label="Storage and retention settings"
          data-icon="gear"
          data-label="Storage and retention"
        >
          <Icon name="gear" size={14} />
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody width="full">
    <!-- Live budget banner (driven by the BudgetExceeded WS event).
         Dismissible; clears automatically on a "recovered" event. -->
    {#if budgetAlert}
      <div class="budget-banner" class:recovered={budgetAlert.direction === 'recovered'} role="status">
        <Icon name={budgetAlert.direction === 'recovered' ? 'check' : 'warning'} size={14} />
        {#if budgetAlert.direction === 'recovered'}
          <span>
            Back under budget: <strong>{budgetAlert.provider || 'workspace'}</strong>
            spend ({fmtCost(budgetAlert.spendUsd)}) is below its {fmtCost(budgetAlert.capUsd)} cap.
          </span>
        {:else}
          <span>
            Over budget: <strong>{budgetAlert.provider || 'workspace'}</strong>
            spent {fmtCost(budgetAlert.spendUsd)} of its {fmtCost(budgetAlert.capUsd)} cap.
          </span>
        {/if}
        <button class="icon-btn" onclick={dismissBudgetAlert} title="Dismiss" aria-label="Dismiss budget alert"><Icon name="x" size={14} /></button>
      </div>
    {/if}

    {#if !auth.isRoot}
      <EmptyState
        variant="page"
        icon="chart"
        title="Usage is root-only"
        body="Token, cost and system-load history is visible to the root account. Ask whoever set up this Otto for access."
      />
    {:else if !usage.status}
      <!-- Status unknown (loading, or /usage/status failed): never fall through to
           the "Install ClickHouse" prompt — a failed load is not "not installed". -->
      <LoadState
        what="usage"
        variant="page"
        rows={6}
        loading={usage.loading || !usage.statusError}
        error={usage.statusError}
        empty
        onretry={() => void usage.loadAll()}
      />
    {:else if !usage.status.available}
      <!-- Engine not installed: the page's one CTA installs it; pointing at an
           existing binary is the quiet secondary path. -->
      <EmptyState
        variant="page"
        icon="chart"
        title="Set up usage tracking"
        body="Otto keeps token, cost and system-load history in an embedded ClickHouse engine (clickhouse local — no server, no port). Install it once and Otto manages it from then on."
        actionLabel={usage.installing ? 'Installing…' : 'Install ClickHouse'}
        actionIcon="download"
        onaction={() => { if (!usage.installing) void usage.install(); }}
      >
        <div class="install-alt">
          <label for="ch-path">Or use a ClickHouse binary you already have</label>
          <div class="path-input">
            <PathField bind:value={chPath} files><input
              id="ch-path"
              class="input mono"
              placeholder="/usr/local/bin/clickhouse"
              bind:value={chPath}
              spellcheck="false"
            /></PathField>
            <button
              class="btn"
              disabled={usage.saving || chPath.trim() === ''}
              title={chPath.trim() === '' ? 'Enter the path to a clickhouse binary first' : 'Use this binary'}
              onclick={() => usage.saveConfig({ enabled: true, clickhouse_path: chPath.trim() })}
            >
              {usage.saving ? 'Saving…' : 'Use binary'}
            </button>
          </div>
          {#if usage.status?.binary}
            <span class="dim small">Detected: <span class="mono">{usage.status.binary}</span></span>
          {/if}
          {#if usage.status?.priced_as_of}
            <span class="dim small">
              Cost estimates use published rates as of {usage.status.priced_as_of}; unknown models are priced
              at the Opus tier and marked “est.”.
            </span>
          {/if}
        </div>
      </EmptyState>
    {:else}
      <div class="body">
        <!-- Engine config (opened from the header gear). Rendered first so
             opening it never lands off-screen below the sessions table. -->
        {#if configOpen}
          <section class="panel card" bind:this={settingsEl} aria-labelledby="usage-cfg-title">
            <div class="panel-head">
              <h3 id="usage-cfg-title">Storage and retention</h3>
              <button class="icon-btn" onclick={() => (configOpen = false)} title="Close" aria-label="Close storage and retention">
                <Icon name="x" size={14} />
              </button>
            </div>
            <div class="cfg-grid">
              <label for="cfg-retention">Keep history for (days)</label>
              <input id="cfg-retention" class="input" type="number" min="1" max="3650" bind:value={retention} />

              <label for="cfg-interval">Sample system load every (seconds)</label>
              <input id="cfg-interval" class="input" type="number" min="5" max="3600" bind:value={interval} />

              <label for="cfg-path">ClickHouse binary</label>
              <PathField bind:value={chPath} files><input id="cfg-path" class="input mono" bind:value={chPath} spellcheck="false" /></PathField>
            </div>
            <div class="cfg-actions">
              <button class="btn" disabled={usage.installing} onclick={() => usage.install()}>
                {usage.installing ? 'Updating…' : 'Update ClickHouse'}
              </button>
              <button
                class="btn primary"
                disabled={usage.saving}
                onclick={() =>
                  usage.saveConfig({
                    enabled: true,
                    retention_days: retention,
                    metrics_interval_secs: interval,
                    clickhouse_path: chPath.trim(),
                  })}
              >
                {usage.saving ? 'Saving…' : 'Save settings'}
              </button>
            </div>
            <div class="engine-meta">
              <span>ClickHouse <span class="mono">{usage.status.version ?? '—'}</span></span>
              <span class="ellip-any" title={usage.status.data_dir}>Data: <span class="mono">{usage.status.data_dir}</span></span>
              <span title="{usage.status.usage_rows.toLocaleString()} usage rows · {usage.status.metric_rows.toLocaleString()} metric rows">
                On disk: {formatBytes(usage.status.disk_bytes)}
                ({plural(usage.status.usage_rows, 'row')})
              </span>
              <span>Kept for {usage.status.retention_days} days</span>
              {#if usage.status.priced_as_of}
                <span title="Cost estimates use published rates as of this date. Unknown models fall back to the Opus tier.">
                  Priced as of {usage.status.priced_as_of}
                </span>
              {/if}
            </div>
          </section>
        {/if}

        {#if usage.summaryError}
          <!-- Summary failed: inline error (no data yet) or a stale-data bar.
               The budgets card below still renders on its own. -->
          <LoadState
            what="usage summary"
            loading={usage.loading}
            error={usage.summaryError}
            empty={!usage.summary}
            onretry={() => void usage.loadAll()}
          />
        {:else if !usage.summary && usage.loading}
          <LoadState what="usage summary" loading empty rows={4} />
        {/if}

        <!-- Stat cards: one sans figure per tile (tabular-nums), label above. -->
        {#if usage.summary}
          <div class="cards">
            <div class="stat card">
              <span class="stat-label">Total tokens</span>
              <span class="stat-value" title={usage.summary.total_tokens.toLocaleString()}>{fmtNum(usage.summary.total_tokens)}</span>
              <div class="seg-bar" title={breakdownTitle(summaryParts(usage.summary))}>
                {#each tokenSegs(summaryParts(usage.summary)) as s (s.label)}
                  {#if s.pct > 0}<div style="width: {s.pct}%; background: {s.color}"></div>{/if}
                {/each}
              </div>
            </div>
            <div class="stat card">
              <span class="stat-label">Estimated cost</span>
              <span class="stat-value">{fmtCost(usage.summary.total_cost_usd)}</span>
              <span class="stat-sub">
                in the last {usage.summary.days} days
                <!-- Projected cost of the next agent run on the most-used
                     provider (hidden when there's no history to base it on). -->
                {#if usage.summary.providers.length > 0}
                  <CostForecastChip
                    feature="agent"
                    provider={usage.summary.providers[0].provider}
                  />
                {/if}
              </span>
            </div>
            <div class="stat card">
              <span class="stat-label">Activity</span>
              <span class="stat-value">{fmtNum(usage.summary.total_events)}</span>
              <span class="stat-sub">{usage.summary.total_events === 1 ? 'event' : 'events'} recorded</span>
            </div>
            <div class="stat card">
              <span class="stat-label">Providers</span>
              <span class="stat-value">{usage.summary.providers.length}</span>
              <span class="stat-sub">across {plural(usage.summary.sessions.length, 'session')}</span>
            </div>
          </div>
        {/if}

        <!-- Token breakdown: input · cache-write · cache-read · output -->
        {#if usage.summary && usage.summary.total_tokens > 0}
          {@const parts = summaryParts(usage.summary)}
          <section class="panel card" aria-labelledby="usage-bd-title">
            <div class="panel-head">
              <h3 id="usage-bd-title">Token breakdown</h3>
              <div class="legend">
                {#each tokenSegs(parts) as s (s.label)}
                  <span class="lg"><i style="background: {s.color}"></i>{s.label}</span>
                {/each}
              </div>
            </div>
            <div class="seg-bar big" title={breakdownTitle(parts)}>
              {#each tokenSegs(parts) as s (s.label)}
                {#if s.pct > 0}<div style="width: {s.pct}%; background: {s.color}"></div>{/if}
              {/each}
            </div>
            <div class="bd-list">
              {#each tokenSegs(parts) as s (s.label)}
                <div class="bd-item">
                  <i class="swatch" style="background: {s.color}"></i>
                  <span class="bd-label">{s.label}</span>
                  <span class="bd-val" title={s.v.toLocaleString()}>{fmtNum(s.v)}</span>
                  <span class="bd-pct dim">{s.pct.toFixed(0)}%</span>
                </div>
              {/each}
            </div>
          </section>
        {/if}

        {#if usage.summary}
          <div class="grid">
            <!-- Provider breakdown -->
            <section class="panel card" aria-labelledby="usage-prov-title">
              <div class="panel-head">
                <h3 id="usage-prov-title">By provider</h3>
                {#if usage.summary.providers.length > 0}
                  <button class="btn small ghost" onclick={exportProviders} title="Download providers as CSV" aria-label="Download providers as CSV">
                    <Icon name="download" size={12} /> CSV
                  </button>
                {/if}
              </div>
              {#if usage.summary.providers.length > 0}
                {@const pmax = Math.max(1, ...usage.summary.providers.map((p) => p.total_tokens))}
                <div class="bars">
                  {#each usage.summary.providers as p (p.provider)}
                    <div class="bar-row">
                      <span class="bar-name" title={p.provider}>{p.provider}</span>
                      <div class="bar-track">
                        <div class="bar-fill stacked" style="width: {(p.total_tokens / pmax) * 100}%" title={breakdownTitle(p)}>
                          {#each tokenSegs(p) as s (s.label)}
                            {#if s.pct > 0}<div style="width: {s.pct}%; background: {s.color}"></div>{/if}
                          {/each}
                        </div>
                      </div>
                      <span class="bar-val" title="{p.total_tokens.toLocaleString()} tokens">{fmtNum(p.total_tokens)}</span>
                      <span class="bar-cost dim">{fmtCost(p.cost_usd)}</span>
                    </div>
                  {/each}
                </div>
              {:else}
                <p class="dim small">No usage in this window. Providers appear here as agents run.</p>
              {/if}
            </section>

            <!-- Daily cost (SVG chart with y-axis labels, gridlines, x-axis ticks,
                 and per-point hover tooltip via <title>).
                 Uses the same stacked-token colour scheme as the bar chart. -->
            <section class="panel card" aria-labelledby="usage-daily-title">
              <div class="panel-head">
                <h3 id="usage-daily-title">Daily cost</h3>
                {#if usage.summary.daily.length > 0}
                  <button class="btn small ghost" onclick={exportDaily} title="Download daily cost as CSV" aria-label="Download daily cost as CSV">
                    <Icon name="download" size={12} /> CSV
                  </button>
                {/if}
              </div>
              {#if usage.summary.daily.length > 0 && dailyMaxCost === 0}
                <p class="dim small" data-testid="daily-cost-empty">No spend recorded in this window.</p>
              {:else if usage.summary.daily.length > 0}
                {@const days = dailyDays}
                {@const n = days.length}
                <svg
                  class="daily-svg"
                  viewBox="0 0 {SVG_W} {SVG_H}"
                  aria-label="Daily cost over the last {usage.summary.days} days, peak {fmtCost(dailyMaxCost)}"
                  role="img"
                >
                  <!-- Gridlines + y-axis labels -->
                  {#each yTicks as tick (tick)}
                    {@const y = svgY(tick)}
                    <line class="grid-line" x1={AXIS_L} y1={y} x2={SVG_W} y2={y} />
                    <text class="axis-label y-label" x={AXIS_L - 4} y={y + 4} text-anchor="end">
                      {fmtCost(tick)}
                    </text>
                  {/each}

                  <!-- Stacked bars (one per day): a thin rect per token category -->
                  {#each days as d, i (d.day)}
                    {@const x = svgX(i, n)}
                    {@const barW = Math.max(2, Math.min(40, ((SVG_W - AXIS_L) / Math.max(1, n)) * 0.7))}
                    {@const barH = (d.cost_usd / (yTicks[yTicks.length - 1] || 1)) * (SVG_H - AXIS_B)}
                    {@const barY = SVG_H - AXIS_B - barH}
                    {@const segs = tokenSegs(d)}
                    <!-- stacked colour segments (bottom = input, then cache-write, cache-read, output) -->
                    {#each segs as s, si (s.label)}
                      {#if s.pct > 0}
                        {@const segH = (s.v / Math.max(1, d.total_tokens)) * barH}
                        {@const segOffset = segs.slice(0, si).reduce((acc, prev) => acc + (prev.v / Math.max(1, d.total_tokens)) * barH, 0)}
                        <rect
                          x={x - barW / 2}
                          y={barY + segOffset}
                          width={barW}
                          height={segH}
                          fill={s.color}
                          rx="1"
                        />
                      {/if}
                    {/each}
                    <!-- Invisible hit target for the tooltip, drawn OVER the
                         segments so hovering the bar itself shows this day. (A
                         bare <title> used to sit directly in the <svg>, which made
                         day 1's text the tooltip for every bar.) -->
                    <rect
                      class="bar-hit"
                      x={x - barW / 2}
                      y={0}
                      width={barW}
                      height={SVG_H - AXIS_B}
                    >
                      <title>{shortDay(d.day)} · {fmtCost(d.cost_usd)} · {breakdownTitle(d)}</title>
                    </rect>

                    <!-- x-axis label (thinned) -->
                    {#if showXLabel(i, n)}
                      <text
                        class="axis-label x-label"
                        x={x}
                        y={SVG_H - 4}
                        text-anchor="middle"
                      >{shortDay(d.day)}</text>
                    {/if}
                  {/each}

                  <!-- x-axis baseline -->
                  <line class="axis-line" x1={AXIS_L} y1={SVG_H - AXIS_B} x2={SVG_W} y2={SVG_H - AXIS_B} />
                </svg>
              {:else}
                <p class="dim small">No daily data in this window.</p>
              {/if}
            </section>
          </div>

          <!-- By feature (by-kind): review / product / channel / agent / … -->
          <section class="panel card" aria-labelledby="usage-feat-title">
            <div class="panel-head">
              <h3 id="usage-feat-title">By feature</h3>
              {#if usage.summary.by_kind.length > 0}
                <span class="dim small">Tokens and cost by kind of work</span>
              {/if}
            </div>
            {#if usage.summary.by_kind.length > 0}
              {@const fmax = Math.max(1, ...usage.summary.by_kind.map((f) => f.total_tokens))}
              <div class="bars">
                {#each usage.summary.by_kind as f (f.feature)}
                  <div class="bar-row feat-row">
                    <span class="bar-name" title={featureLabel(f.feature)}>
                      <span class="kind-badge">{featureLabel(f.feature)}</span>
                    </span>
                    <div class="bar-track">
                      <div class="bar-fill stacked" style="width: {(f.total_tokens / fmax) * 100}%" title={breakdownTitle(f)}>
                        {#each tokenSegs(f) as s (s.label)}
                          {#if s.pct > 0}<div style="width: {s.pct}%; background: {s.color}"></div>{/if}
                        {/each}
                      </div>
                    </div>
                    <span class="bar-val" title="{f.total_tokens.toLocaleString()} tokens">{fmtNum(f.total_tokens)}</span>
                    <span class="bar-cost dim">{fmtCost(f.cost_usd)}</span>
                  </div>
                {/each}
              </div>
            {:else}
              <p class="dim small">No usage in this window. Cost splits by feature (reviews, product, channels, agents…) appear here as work runs.</p>
            {/if}
          </section>

          <!-- Work-graph attribution drilldown (B1): "why did this cost so much?"
               Loads on its own, with its own loading/empty/error states. -->
          <AttributionDrilldown days={usage.days} />
        {/if}

        <!-- Budgets (opt-in spend caps). Config, not engine data: renders even
             when the summary failed so caps stay visible and editable. -->
        <section class="panel card" aria-labelledby="usage-budget-title">
          <div class="panel-head">
            <h3 id="usage-budget-title">Budgets</h3>
            <button
              class="btn small"
              aria-expanded={budgetsOpen}
              aria-controls="usage-budget-editor"
              onclick={() => (budgetsOpen ? discardBudgets() : (budgetsOpen = true))}
              title={budgetsOpen && budgetsDirty ? 'Close the editor and discard unsaved changes' : undefined}
            >
              {#if budgetsOpen}{budgetsDirty ? 'Discard changes' : 'Close editor'}{:else}<Icon name="edit" size={12} /> Edit budgets{/if}
            </button>
          </div>
          <p class="dim small intro">
            Spend caps per workspace or provider. They only warn until you turn on enforcement.
          </p>

          <!-- Status: budget vs spend -->
          {#if usage.budgets && usage.budgets.rows.length > 0}
            <div class="budget-rows">
              {#each usage.budgets.rows as r (r.scope + ':' + r.key)}
                {@const name = r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key)}
                <div class="budget-row" class:warn={r.warning && !r.exceeded} class:over={r.exceeded}>
                  <span class="budget-name" title="{r.scope === 'workspace' ? 'Workspace' : 'Provider'}: {name}">
                    <span class="kind-badge">{r.scope === 'workspace' ? 'Workspace' : 'Provider'}</span>
                    <span class="ellip-any">{name}</span>
                  </span>
                  <div
                    class="bar-track"
                    role="meter"
                    aria-label="{name} spend"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(Math.min(100, r.used_fraction * 100))}
                  >
                    <div
                      class="bar-fill"
                      style="width: {Math.min(100, r.used_fraction * 100)}%"
                      title="{fmtCost(r.spent_usd)} of {fmtCost(r.limit_usd)}"
                    ></div>
                  </div>
                  <span class="budget-val">
                    {fmtCost(r.spent_usd)} / {fmtCost(r.limit_usd)}
                    {#if r.exceeded}<span class="over-tag">Over</span>
                    {:else if r.warning}<span class="warn-tag">{(r.used_fraction * 100).toFixed(0)}%</span>{/if}
                  </span>
                </div>
              {/each}
            </div>
            {#if usage.budgets.config.enforce && usage.budgets.rows.some((r) => r.exceeded)}
              <div class="budget-alert" role="status">
                <Icon name="warning" size={14} />
                {usage.budgets.config.block_on_exceed
                  ? 'Enforcement is on and blocking: new work in an over-budget scope can be refused.'
                  : 'Enforcement is on (warn only): over-budget scopes are flagged, not blocked.'}
              </div>
            {/if}
            <!-- Heads-up for caps close to their limit (the exceeded case is the
                 alert above). -->
            {#if !usage.budgets.config.enforce || !usage.budgets.rows.some((r) => r.exceeded)}
              {#each usage.budgets.rows.filter((r) => r.warning && !r.exceeded) as r (r.scope + ':' + r.key)}
                <p class="dim small heads-up">
                  {r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key)}
                  is at {(r.used_fraction * 100).toFixed(0)}% of its cap
                  ({fmtCost(r.spent_usd)} of {fmtCost(r.limit_usd)}).
                </p>
              {/each}
            {/if}
          {:else if usage.budgetsError}
            <LoadState
              what="budgets"
              variant="compact"
              error={usage.budgetsError}
              empty
              onretry={() => void usage.loadBudgets()}
            />
          {:else if !usage.budgets}
            <LoadState what="budgets" variant="compact" loading empty />
          {:else if !budgetsOpen}
            <p class="dim small">No budgets yet. Use Edit budgets to set a cap and track spend against it.</p>
          {/if}

          <!-- Editor -->
          {#if budgetsOpen}
            <div class="budget-editor" id="usage-budget-editor">
              <label class="checkbox-row">
                <input type="checkbox" bind:checked={budgetCfg.enforce} onchange={() => (budgetsDirty = true)} />
                <span>Enforce budgets: warn prominently when a cap is exceeded</span>
              </label>
              <label class="checkbox-row" class:disabled={!budgetCfg.enforce} title={budgetCfg.enforce ? undefined : 'Turn on enforcement first'}>
                <input
                  type="checkbox"
                  bind:checked={budgetCfg.block_on_exceed}
                  disabled={!budgetCfg.enforce}
                  onchange={() => (budgetsDirty = true)}
                />
                <span>Block new work in a scope that is over its cap (otherwise warn only)</span>
              </label>
              <div class="window-row">
                <label for="budget-window">Compare spend over the last</label>
                <input
                  id="budget-window"
                  class="input num-in"
                  type="number"
                  min="1"
                  bind:value={budgetCfg.window_days}
                  oninput={() => (budgetsDirty = true)}
                />
                <span class="dim">days</span>
              </div>

              <div class="editor-section">
                <div class="editor-head">
                  <span id="budget-ws-head">Per workspace</span>
                  <button class="btn small ghost" onclick={addWsBudget}><Icon name="plus" size={12} /> Add workspace cap</button>
                </div>
                {#each budgetCfg.workspaces as b, i (i)}
                  <div class="editor-line">
                    <select class="input" aria-label="Workspace" bind:value={b.workspace_id} onchange={() => (budgetsDirty = true)}>
                      <option value="">Choose a workspace…</option>
                      {#each ws.workspaces as w (w.id)}
                        <option value={w.id}>{w.name}</option>
                      {/each}
                    </select>
                    <span class="usd">$</span>
                    <input
                      class="input num-in"
                      type="number"
                      min="0"
                      step="1"
                      placeholder="0"
                      aria-label="Cap in US dollars"
                      bind:value={b.monthly_usd}
                      oninput={() => (budgetsDirty = true)}
                    />
                    <button class="icon-btn rm-btn" onclick={() => removeWsBudget(i)} title="Remove this cap" aria-label="Remove this workspace cap">
                      <Icon name="trash" size={14} />
                    </button>
                  </div>
                {:else}
                  <p class="dim small">No workspace caps.</p>
                {/each}
              </div>

              <div class="editor-section">
                <div class="editor-head">
                  <span>Per provider</span>
                  <button class="btn small ghost" onclick={addProviderBudget}><Icon name="plus" size={12} /> Add provider cap</button>
                </div>
                {#each budgetCfg.providers as b, i (i)}
                  <div class="editor-line">
                    <select class="input" aria-label="Provider" bind:value={b.provider} onchange={() => (budgetsDirty = true)}>
                      <option value="">Choose a provider…</option>
                      {#each providerChoices as p (p)}
                        <option value={p}>{p}</option>
                      {/each}
                    </select>
                    <span class="usd">$</span>
                    <input
                      class="input num-in"
                      type="number"
                      min="0"
                      step="1"
                      placeholder="0"
                      aria-label="Cap in US dollars"
                      bind:value={b.monthly_usd}
                      oninput={() => (budgetsDirty = true)}
                    />
                    <button class="icon-btn rm-btn" onclick={() => removeProviderBudget(i)} title="Remove this cap" aria-label="Remove this provider cap">
                      <Icon name="trash" size={14} />
                    </button>
                  </div>
                {:else}
                  <p class="dim small">No provider caps.</p>
                {/each}
              </div>

              <div class="editor-actions">
                <span class="dim small">Rows without a scope or a cap above $0 are dropped on save.</span>
                <button
                  class="btn primary"
                  disabled={usage.savingBudgets || !budgetsDirty}
                  title={budgetsDirty ? undefined : 'No changes to save'}
                  onclick={saveBudgets}
                >
                  {usage.savingBudgets ? 'Saving…' : 'Save budgets'}
                </button>
              </div>
            </div>
          {/if}
        </section>

        {#if usage.summary}
          <!-- System metrics -->
          <section class="panel card" aria-labelledby="usage-sys-title">
            <div class="panel-head">
              <h3 id="usage-sys-title">System load</h3>
              {#if latest}
                <span class="dim small now">
                  CPU {latest.cpu_pct.toFixed(0)}% · memory {latest.mem_pct.toFixed(0)}%
                  ({fmtNum(Math.round(latest.mem_used_mb))} of {fmtNum(Math.round(latest.mem_total_mb))} MB) ·
                  Otto daemon {latest.process_rss_mb.toFixed(0)} MB · {plural(latest.active_sessions, 'active session')}
                </span>
              {/if}
            </div>
            {#if usage.metrics.length > 1}
              <div class="metrics">
                <div class="metric">
                  <span class="metric-label">CPU %</span>
                  <svg viewBox="0 0 300 48" preserveAspectRatio="none" class="spark" role="img" aria-label="CPU over the last {usage.metrics.length} samples">
                    <path d={sparkPath(cpuSeries, 100, 300, 48)} />
                  </svg>
                </div>
                <div class="metric">
                  <span class="metric-label">Memory %</span>
                  <svg viewBox="0 0 300 48" preserveAspectRatio="none" class="spark" role="img" aria-label="Memory over the last {usage.metrics.length} samples">
                    <path d={sparkPath(memSeries, 100, 300, 48)} />
                  </svg>
                </div>
              </div>
              <span class="dim small">Last {usage.metrics.length} samples, one every {usage.status.metrics_interval_secs} s</span>
            {:else}
              <p class="dim small">Collecting samples… one every {usage.status.metrics_interval_secs} s.</p>
            {/if}
          </section>

          <!-- Sessions leaderboard — rows are virtualized so raising SESSION_LIMIT
               (currently 50) stays DOM-bounded. The header stays fixed above the
               virtual list; rows use a CSS-grid div layout. -->
          <section class="panel card" aria-labelledby="usage-sess-title">
            <div class="panel-head">
              <h3 id="usage-sess-title">Top sessions</h3>
              {#if usage.summary.sessions.length > 0}
                <button class="btn small ghost" onclick={exportSessions} title="Download sessions as CSV" aria-label="Download sessions as CSV">
                  <Icon name="download" size={12} /> CSV
                </button>
              {/if}
            </div>
            {#if usage.summary.sessions.length > 0}
              <div class="sess-scroll">
                <!-- Column header row (fixed, not virtualized) -->
                <div class="sess-head">
                  <span>Session</span>
                  <span>Workspace</span>
                  <span>Provider / model</span>
                  <span class="num">Events</span>
                  <span class="num">Tokens</span>
                  <span class="num">Cost</span>
                  <span>Last active</span>
                </div>
                <!-- Virtualized body: each row is ~46px. -->
                <VirtualList items={usage.summary.sessions} estimateHeight={46} class="sess-vlist">
                  {#snippet row(s)}
                    {@const isOttoSession = s.kind != null || s.title != null}
                    <div class="sess-row">
                      <!-- Title first; the raw id is secondary (mono, dim, full id on hover).
                           An Otto session's title is a real button that opens it; a session
                           run outside Otto has nothing to open, so it stays plain text. -->
                      <div class="sess-cell" title={s.title ? `${s.title}\n${s.session_id}` : s.session_id}>
                        <div class="sess-top">
                          {#if isOttoSession}
                            <button class="sess-name sess-open ellip-any" onclick={() => openSession(s.session_id)}
                              aria-label="Open session {s.title ?? s.session_id}">{s.title ?? s.session_id.slice(0, 12)}</button>
                          {:else}
                            <span class="sess-name ellip-any">{s.title ?? 'Session outside Otto'}</span>
                          {/if}
                          {#if s.kind}<span class="kind-badge">{s.kind}</span>{/if}
                        </div>
                        <div class="sess-id mono">{s.session_id.slice(0, 12)}</div>
                      </div>
                      <div class="dim ellip-any" title={s.workspace_name ?? undefined}>{s.workspace_name ?? '—'}</div>
                      <div class="model-cell">
                        <span class="ellip-any" title={s.provider}>{s.provider}</span>
                        {#if s.model}
                          <span class="model-name dim ellip-any" title={s.model}>{s.model}</span>
                        {/if}
                      </div>
                      <div class="num">{fmtNum(s.events)}</div>
                      <div class="num">
                        <div class="sess-tok">
                          <span title="{s.total_tokens.toLocaleString()} tokens">{fmtNum(s.total_tokens)}</span>
                          <div class="seg-bar mini" title={breakdownTitle(s)}>
                            {#each tokenSegs(s) as seg (seg.label)}
                              {#if seg.pct > 0}<div style="width: {seg.pct}%; background: {seg.color}"></div>{/if}
                            {/each}
                          </div>
                        </div>
                      </div>
                      <div class="num" title={s.fallback_priced ? 'Estimated: this model isn’t in the rate table, so it is priced at the Opus tier' : undefined}>
                        {fmtCost(s.cost_usd)}
                        {#if s.fallback_priced}<span class="est-tag">est.</span>{/if}
                      </div>
                      <div class="dim">{fmtLastActive(s.last_active)}</div>
                    </div>
                  {/snippet}
                </VirtualList>
              </div>
            {:else}
              <p class="dim small">No sessions in this window.</p>
            {/if}
          </section>
        {/if}
      </div>
    {/if}
  </PageBody>
</div>

<style>
  .usage {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  /* Toggle state for header buttons (auto-refresh, settings). */
  .on {
    border-color: var(--accent);
    color: var(--accent-text);
    background: var(--accent-soft);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }
  @media (max-width: 1024px) {
    .cards {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  @media (max-width: 640px) {
    .cards {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .stat {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .stat-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  /* One sans family for every KPI figure, tabular so digits line up. */
  .stat-value {
    font-size: var(--fs-2xl);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }
  .stat-sub {
    font-size: var(--fs-s);
    color: var(--text-dim);
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 6px;
  }

  /* Stacked token-composition bar (input · cache-write · cache-read · output) */
  .seg-bar {
    display: flex;
    height: 6px;
    margin-top: 8px;
    border-radius: 3px;
    overflow: hidden;
    background: var(--surface-2);
  }
  .seg-bar > div {
    height: 100%;
    flex-shrink: 0;
  }
  .seg-bar.big {
    height: 12px;
    margin: 0 0 14px;
    border-radius: 6px;
  }
  .seg-bar.mini {
    height: 4px;
    width: 72px;
    margin-top: 4px;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
  }
  .lg {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .lg i,
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex-shrink: 0;
  }

  .bd-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 8px 20px;
  }
  .bd-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .bd-label {
    color: var(--text-dim);
  }
  .bd-val {
    margin-inline-start: auto;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    color: var(--text);
  }
  .bd-pct {
    min-width: 32px;
    text-align: end;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }

  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 14px;
  }
  .panel {
    padding: 14px 16px;
    min-width: 0;
  }
  /* Card titles: sentence case, one level (dashboard archetype). */
  .panel h3 {
    font-size: var(--fs-m);
    font-weight: 600;
    margin: 0;
    color: var(--text);
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    min-height: 24px;
    margin-bottom: 12px;
    gap: 6px 12px;
  }
  .intro {
    margin: -6px 0 10px;
  }

  .bars {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .bar-row {
    display: grid;
    grid-template-columns: 88px minmax(0, 1fr) 56px 56px;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
  }
  .bar-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    min-width: 0;
  }
  .bar-track {
    height: 8px;
    background: var(--surface-2);
    border-radius: 4px;
    overflow: hidden;
  }
  .bar-fill {
    height: 100%;
    border-radius: 4px;
    background: var(--accent-solid);
    transition: width 200ms ease-out;
  }
  /* Stacked variant: width = provider share of max; segments = composition. */
  .bar-fill.stacked {
    display: flex;
    overflow: hidden;
    min-width: 2px;
    background: none;
  }
  .bar-fill.stacked > div {
    height: 100%;
    flex-shrink: 0;
  }
  .bar-val {
    text-align: end;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .bar-cost {
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  @media (prefers-reduced-motion: reduce) {
    .bar-fill {
      transition: none;
    }
  }

  /* Daily cost SVG chart --------------------------------------------------- */
  .daily-svg {
    width: 100%;
    height: 120px;
    display: block;
    overflow: visible;
  }
  .grid-line {
    stroke: var(--border);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  .axis-line {
    stroke: var(--border);
    stroke-width: 1;
  }
  .axis-label {
    fill: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .y-label {
    dominant-baseline: middle;
  }
  .x-label {
    dominant-baseline: auto;
  }
  .bar-hit {
    fill: transparent;
    cursor: crosshair;
  }
  .bar-hit:hover {
    fill: var(--hover);
  }

  .metrics {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 16px;
    margin-bottom: 6px;
  }
  .metric {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .metric-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .spark {
    width: 100%;
    height: 48px;
  }
  /* Both series are labelled; colour isn't carrying meaning here. */
  .spark path {
    fill: none;
    stroke: var(--info);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  .now {
    font-variant-numeric: tabular-nums;
  }

  /* Sessions leaderboard: fixed column header + VirtualList rows ----------- */
  /* Wide table scrolls inside its own container, never the page. */
  .sess-scroll {
    overflow-x: auto;
  }
  /* 7-column CSS grid: session · workspace · provider/model · events · tokens · cost · last active */
  .sess-head,
  .sess-row {
    display: grid;
    grid-template-columns: minmax(160px, 2fr) minmax(90px, 1fr) minmax(110px, 1.4fr) 60px 90px 72px minmax(100px, 1fr);
    align-items: center;
    font-size: var(--fs-s);
    min-width: 680px;
  }
  .sess-head {
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
    font-weight: 600;
  }
  .sess-head > span,
  .sess-row > div {
    padding: 6px 8px;
    min-width: 0;
  }
  .sess-row {
    border-bottom: 1px solid var(--separator);
    color: var(--text);
  }
  .sess-row:last-child {
    border-bottom: none;
  }
  .sess-head .num {
    text-align: end;
  }
  .sess-row .num {
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  /* VirtualList container: max-height so it stays bounded. */
  :global(.sess-vlist) {
    max-height: 460px;
    /* As wide as its rows, so the header and rows scroll sideways together
       inside .sess-scroll (not the rows alone inside the list). */
    min-width: 680px;
    overflow-x: hidden;
  }
  .sess-tok {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
  }
  .sess-cell {
    min-width: 0;
  }
  .sess-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .sess-name {
    color: var(--text);
  }
  .sess-id {
    margin-top: 2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ellip-any {
    display: block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind-badge {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 7px;
    border-radius: 999px;
    /* Kinds are categories, not statuses: one neutral chip, told apart by the
       word (foundations.md — no categorical rainbows). */
    background: var(--surface-2);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .sess-row:hover {
    background: var(--hover);
  }
  .sess-open {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .sess-open:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .sess-open:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .model-cell {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .model-name {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
  }
  /* "Estimated" cost tag — the model is not in the rate table. */
  .est-tag {
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 0 5px;
    border-radius: 999px;
    margin-inline-start: 4px;
    background: var(--warning-soft);
    color: var(--warning);
  }

  /* By-feature rows: widen the label column so the feature badge fits. */
  .feat-row {
    grid-template-columns: 128px minmax(0, 1fr) 56px 56px;
  }

  /* Storage & retention ----------------------------------------------------- */
  .cfg-grid {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    gap: 10px 12px;
    align-items: center;
    max-width: 640px;
  }
  .cfg-grid label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .cfg-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    max-width: 640px;
    margin-top: 14px;
  }
  .engine-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    margin-top: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--separator);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .engine-meta > span {
    max-width: 100%;
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* Not-installed page: the secondary "use an existing binary" path under the
     EmptyState's one CTA. */
  .install-alt {
    width: min(460px, 100%);
    display: flex;
    flex-direction: column;
    gap: 6px;
    text-align: start;
    margin-top: 12px;
    padding-top: 14px;
    border-top: 1px solid var(--separator);
  }
  .install-alt label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .path-input {
    display: flex;
    gap: 8px;
  }
  .path-input > :global(:first-child) {
    flex: 1;
    min-width: 0;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  p.small {
    margin: 0;
    line-height: 1.5;
  }

  /* --- Budgets --------------------------------------------------------- */
  .budget-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .budget-row {
    display: grid;
    grid-template-columns: minmax(140px, 1.4fr) minmax(0, 2fr) minmax(120px, auto);
    align-items: center;
    gap: 10px;
  }
  .budget-name {
    font-size: var(--fs-s);
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .budget-val {
    font-size: var(--fs-s);
    color: var(--text-dim);
    text-align: end;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .budget-row.warn .bar-fill {
    background: var(--warning);
  }
  .budget-row.over .bar-fill {
    background: var(--danger);
  }
  .warn-tag {
    color: var(--warning);
    font-weight: 600;
    margin-inline-start: 4px;
  }
  .over-tag {
    color: var(--danger);
    font-weight: 600;
    margin-inline-start: 4px;
  }
  .heads-up {
    margin-top: 6px;
  }
  /* Live WS budget banner — top of the page body. */
  .budget-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 12px;
    margin: 0 0 14px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .budget-banner > :global(svg:first-child) {
    color: var(--danger);
    flex-shrink: 0;
  }
  .budget-banner.recovered {
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 40%, transparent);
  }
  .budget-banner.recovered > :global(svg:first-child) {
    color: var(--success);
  }
  .budget-banner span {
    flex: 1;
    min-width: 0;
  }
  .budget-alert {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .budget-alert > :global(svg) {
    color: var(--danger);
    flex-shrink: 0;
  }
  .budget-editor {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 640px;
  }
  .checkbox-row.disabled {
    opacity: 0.55;
  }
  .window-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
  }
  .editor-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .editor-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .editor-line {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .editor-line select {
    flex: 1;
    min-width: 0;
  }
  .usd {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .num-in {
    width: 96px;
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  .rm-btn:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .editor-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    flex-wrap: wrap;
  }

  @media (max-width: 1024px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (max-width: 640px) {
    .metrics {
      grid-template-columns: minmax(0, 1fr);
    }
    /* Bar rows: name + value on one line, the bar underneath. */
    .bar-row,
    .feat-row {
      grid-template-columns: minmax(0, 1fr) 56px;
      grid-template-rows: auto auto;
      gap: 4px 8px;
    }
    .bar-row .bar-name {
      grid-column: 1;
      grid-row: 1;
    }
    .bar-row .bar-val {
      grid-column: 2;
      grid-row: 1;
    }
    .bar-row .bar-track {
      grid-column: 1 / 3;
      grid-row: 2;
    }
    .bar-row .bar-cost {
      display: none; /* cost stays in the bar's hover title */
    }
    .budget-row {
      grid-template-columns: minmax(0, 1fr) auto;
      grid-template-rows: auto auto;
      gap: 4px 8px;
    }
    .budget-row .budget-name {
      grid-column: 1;
      grid-row: 1;
    }
    .budget-row .budget-val {
      grid-column: 2;
      grid-row: 1;
    }
    .budget-row .bar-track {
      grid-column: 1 / 3;
      grid-row: 2;
    }
    .cfg-grid {
      grid-template-columns: minmax(0, 1fr);
    }
    /* Sessions: drop workspace + last-active; the rest fits a phone. */
    .sess-head,
    .sess-row {
      grid-template-columns: minmax(0, 2fr) minmax(0, 1.5fr) 48px 64px 56px;
      min-width: 0;
    }
    :global(.sess-vlist) {
      min-width: 0;
    }
    .sess-head > span:nth-child(2),
    .sess-head > span:nth-child(7),
    .sess-row > div:nth-child(2),
    .sess-row > div:nth-child(7) {
      display: none;
    }
  }
</style>
