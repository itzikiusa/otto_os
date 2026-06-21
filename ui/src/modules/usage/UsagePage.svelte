<script lang="ts">
  // Usage dashboard (root-only): provider/day/session token rollups, system
  // CPU/RAM metrics, and the embedded-ClickHouse install/retention controls.
  // All data comes from the daemon's /usage/* endpoints (otto-usage engine).
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { usage } from '../../lib/api/usage.svelte';
  import type { UsageBudgetConfig } from '../../lib/api/usage.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
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
    // Drop blank rows before saving (no key or no cap = nothing to enforce).
    const cfg: UsageBudgetConfig = {
      ...budgetCfg,
      window_days: budgetCfg.window_days || 30,
      workspaces: budgetCfg.workspaces.filter((b) => b.workspace_id && b.monthly_usd > 0),
      providers: budgetCfg.providers.filter((b) => b.provider && b.monthly_usd > 0),
    };
    await usage.saveBudgets(cfg);
    budgetsDirty = false;
  }

  // Workspace name for a budget row's id (falls back to the id).
  function wsName(id: string): string {
    return ws.workspaces.find((w) => w.id === id)?.name ?? id;
  }
  // Provider choices for the budget editor (installed CLIs + any already used).
  const providerChoices = $derived.by(() => {
    const set = new Set<string>((usage.summary?.providers ?? []).map((p) => p.provider));
    for (const t of auth.meta?.providers ?? []) set.add(t);
    return [...set].filter(Boolean);
  });

  function fmtNum(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(n >= 10_000 ? 0 : 1) + 'k';
    return String(n);
  }
  function fmtCost(n: number): string {
    if (n === 0) return '$0';
    if (n < 0.01) return '<$0.01';
    return '$' + n.toFixed(n < 100 ? 2 : 0);
  }
  function fmtBytes(n: number): string {
    if (n >= 1 << 30) return (n / (1 << 30)).toFixed(1) + ' GB';
    if (n >= 1 << 20) return (n / (1 << 20)).toFixed(1) + ' MB';
    if (n >= 1 << 10) return (n / (1 << 10)).toFixed(0) + ' KB';
    return n + ' B';
  }
  function shortDay(iso: string): string {
    // "2026-06-16" → "06-16"
    return iso.length >= 10 ? iso.slice(5) : iso;
  }
  function fmtLastActive(iso: string): string {
    // "2026-06-16 14:32:05.123" → "Jun 16, 14:32" (date matters: window is up to
    // 180d, so time-only is ambiguous). Format the stored value directly to avoid
    // a timezone shift.
    const m = iso.match(/(\d{4})-(\d{2})-(\d{2})[ T](\d{2}:\d{2})/);
    if (!m) return iso;
    const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    const mon = months[parseInt(m[2], 10) - 1] ?? m[2];
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
  const dailyMaxCost = $derived(Math.max(...dailyCosts, 0.001));
  const dailyDays = $derived(usage.summary?.daily ?? []);

  // Y-axis: 4 ticks from 0 to ceiling. Round the top tick to a "nice" value.
  const yTicks = $derived.by(() => {
    const top = dailyMaxCost;
    const raw = top / 3;
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
  function svgX(i: number, total: number): number {
    if (total <= 1) return AXIS_L;
    const plotW = SVG_W - AXIS_L;
    return AXIS_L + (i / (total - 1)) * plotW;
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
    { label: 'Input', color: 'var(--accent)', pick: (o: TokenParts) => o.input_tokens },
    { label: 'Cache write', color: '#f59e0b', pick: (o: TokenParts) => o.cache_write_tokens },
    { label: 'Cache read', color: '#10b981', pick: (o: TokenParts) => o.cache_read_tokens },
    { label: 'Output', color: '#8b5cf6', pick: (o: TokenParts) => o.output_tokens },
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
  <header class="usage-head">
    <div class="title">
      <Icon name="chart" size={16} />
      <h1>Usage &amp; Metrics</h1>
      {#if usage.status?.available}
        <span class="pill ok" title={usage.status.version ?? ''}>ClickHouse</span>
      {/if}
    </div>
    <div class="grow"></div>
    {#if usage.status?.available}
      <div class="seg" title="Scope: only sessions run inside Otto, or all Claude/codex usage on this machine">
        <button class="seg-btn" class:active={usage.ottoOnly} onclick={() => usage.setOttoOnly(true)}>
          Otto
        </button>
        <button class="seg-btn" class:active={!usage.ottoOnly} onclick={() => usage.setOttoOnly(false)}>
          All
        </button>
      </div>
      <div class="seg">
        {#each WINDOWS as w (w.days)}
          <button class="seg-btn" class:active={usage.days === w.days} onclick={() => usage.setDays(w.days)}>
            {w.label}
          </button>
        {/each}
      </div>
      <button class="btn" onclick={() => usage.loadAll()} disabled={usage.loading} title="Refresh">
        <Icon name="refresh" size={13} /> Refresh
      </button>
      <button
        class="btn"
        class:active={usage.autoRefresh}
        onclick={() => usage.setAutoRefresh(!usage.autoRefresh)}
        title={usage.autoRefresh ? 'Auto-refresh ON — click to stop' : 'Auto-refresh OFF — click to enable (refreshes every 60s)'}
      >
        <Icon name="clock" size={13} />
        {usage.autoRefresh ? 'Live' : 'Auto'}
      </button>
      <button
        class="btn"
        disabled={!usage.summary}
        onclick={() => usage.exportSummaryJson()}
        title="Download full summary as JSON"
      >
        <Icon name="download" size={13} /> Export
      </button>
      <button class="btn" class:active={configOpen} onclick={() => (configOpen = !configOpen)} title="Settings">
        <Icon name="gear" size={13} />
      </button>
    {/if}
  </header>

  <!-- Live budget-exceeded banner (driven by BudgetExceeded WS event).
       Dismissible; clears automatically on a "recovered" event. -->
  {#if budgetAlert}
    <div class="budget-banner" class:recovered={budgetAlert.direction === 'recovered'}>
      <Icon name="warning" size={14} />
      {#if budgetAlert.direction === 'recovered'}
        <span>
          Budget recovered — <strong>{budgetAlert.provider || 'workspace'}</strong>
          spend (${budgetAlert.spendUsd.toFixed(2)}) is back below the ${budgetAlert.capUsd.toFixed(2)} cap.
        </span>
      {:else}
        <span>
          Budget exceeded — <strong>{budgetAlert.provider || 'workspace'}</strong>
          spent ${budgetAlert.spendUsd.toFixed(2)} of the ${budgetAlert.capUsd.toFixed(2)} cap.
        </span>
      {/if}
      <button class="close-btn" onclick={dismissBudgetAlert} title="Dismiss">×</button>
    </div>
  {/if}

  {#if !auth.isRoot}
    <div class="empty">
      <Icon name="gauge" size={28} />
      <p>Usage analytics are available to the root account.</p>
    </div>
  {:else if usage.loading && !usage.status}
    <div class="empty"><p>Loading…</p></div>
  {:else if !usage.status?.available}
    <!-- ClickHouse not installed: install / configure prompt -->
    <div class="install card">
      <Icon name="db" size={26} />
      <h2>Set up usage tracking</h2>
      <p>
        Otto stores usage history and system metrics in an embedded
        <strong>ClickHouse</strong> engine (run locally via <code>clickhouse local</code>, no
        server or port). Install it once — Otto manages it from here on.
      </p>
      <div class="install-cmd">
        <code>curl https://clickhouse.com/ | sh</code>
      </div>
      <div class="install-actions">
        <button class="btn primary" onclick={() => usage.install()} disabled={usage.installing}>
          {usage.installing ? 'Installing…' : 'Install ClickHouse'}
        </button>
      </div>
      <div class="path-row">
        <label for="ch-path">…or point at an existing binary</label>
        <div class="path-input">
          <input
            id="ch-path"
            class="input mono"
            placeholder="/usr/local/bin/clickhouse"
            bind:value={chPath}
            spellcheck="false"
          />
          <button
            class="btn"
            disabled={usage.saving || chPath.trim() === ''}
            onclick={() => usage.saveConfig({ enabled: true, clickhouse_path: chPath.trim() })}
          >
            Use
          </button>
        </div>
        {#if usage.status?.binary}
          <span class="dim">Detected: <span class="mono">{usage.status.binary}</span></span>
        {/if}
      </div>
      {#if usage.status?.priced_as_of}
        <p class="install-meta dim">
          Cost estimates use rates priced as of <strong>{usage.status.priced_as_of}</strong>.
          Unknown models fall back to the Opus tier (flagged as "estimated" in session rows).
        </p>
      {/if}
    </div>
  {:else}
    <div class="body">
      <!-- Stat cards -->
      {#if usage.summary}
        <div class="cards">
          <div class="stat card">
            <span class="stat-label">Total tokens</span>
            <span class="stat-value">{fmtNum(usage.summary.total_tokens)}</span>
            <div class="seg-bar" title={breakdownTitle(summaryParts(usage.summary))}>
              {#each tokenSegs(summaryParts(usage.summary)) as s (s.label)}
                {#if s.pct > 0}<div style="width: {s.pct}%; background: {s.color}"></div>{/if}
              {/each}
            </div>
          </div>
          <div class="stat card">
            <span class="stat-label">Est. cost</span>
            <span class="stat-value">{fmtCost(usage.summary.total_cost_usd)}</span>
            <span class="stat-sub">
              over {usage.summary.days}d
              <!-- Pre-launch forecast chip: projects the cost of the next run
                   using the most-used provider over the current window. -->
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
            <span class="stat-sub">events recorded</span>
          </div>
          <div class="stat card">
            <span class="stat-label">Providers</span>
            <span class="stat-value">{usage.summary.providers.length}</span>
            <span class="stat-sub">{usage.summary.sessions.length} sessions</span>
          </div>
        </div>
      {/if}

      <!-- Token breakdown: input · cache-write · cache-read · output -->
      {#if usage.summary && usage.summary.total_tokens > 0}
        {@const parts = summaryParts(usage.summary)}
        <div class="panel card">
          <div class="panel-head">
            <h3>Token breakdown</h3>
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
                <i class="chip" style="background: {s.color}"></i>
                <span class="bd-label">{s.label}</span>
                <span class="bd-val">{fmtNum(s.v)}</span>
                <span class="bd-pct dim">{s.pct.toFixed(0)}%</span>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <div class="grid">
        <!-- Provider breakdown -->
        <div class="panel card">
          <div class="panel-head">
            <h3>By provider</h3>
            {#if usage.summary && usage.summary.providers.length > 0}
              <button class="link-btn" onclick={() => usage.exportProvidersCsv()} title="Download as CSV">
                <Icon name="download" size={11} /> CSV
              </button>
            {/if}
          </div>
          {#if usage.summary && usage.summary.providers.length > 0}
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
                  <span class="bar-val">{fmtNum(p.total_tokens)}</span>
                  <span class="bar-cost dim">{fmtCost(p.cost_usd)}</span>
                </div>
              {/each}
            </div>
          {:else}
            <p class="dim small">No usage recorded yet. Activity appears here as agents run.</p>
          {/if}
        </div>

        <!-- Daily cost (SVG chart with y-axis labels, gridlines, x-axis ticks,
             and per-point hover tooltip via <title>).
             Uses the same stacked-token colour scheme as the bar chart. -->
        <div class="panel card">
          <div class="panel-head">
            <h3>Daily cost</h3>
            {#if usage.summary && usage.summary.daily.length > 0}
              <button class="link-btn" onclick={() => usage.exportDailyCsv()} title="Download as CSV">
                <Icon name="download" size={11} /> CSV
              </button>
            {/if}
          </div>
          {#if usage.summary && usage.summary.daily.length > 0}
            {@const days = dailyDays}
            {@const n = days.length}
            <svg
              class="daily-svg"
              viewBox="0 0 {SVG_W} {SVG_H}"
              aria-label="Daily cost chart"
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

              <!-- Stacked area bars (one per day): a thin rect per token category -->
              {#each days as d, i (d.day)}
                {@const x = svgX(i, n)}
                {@const barW = n > 1 ? Math.max(2, (SVG_W - AXIS_L) / n * 0.7) : 20}
                {@const barH = (d.cost_usd / (yTicks[yTicks.length - 1] || 1)) * (SVG_H - AXIS_B)}
                {@const barY = SVG_H - AXIS_B - barH}
                {@const segs = tokenSegs(d)}
                <title>{d.day}: {breakdownTitle(d)} · {fmtCost(d.cost_usd)}</title>
                <!-- invisible hit target for tooltip -->
                <rect
                  class="bar-hit"
                  x={x - barW / 2}
                  y={0}
                  width={barW}
                  height={SVG_H - AXIS_B}
                >
                  <title>{d.day} · {fmtCost(d.cost_usd)} · {breakdownTitle(d)}</title>
                </rect>
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
        </div>
      </div>

      <!-- By feature (by-kind): review / product / channel / agent / … -->
      <div class="panel card">
        <div class="panel-head">
          <h3>By feature</h3>
          {#if usage.summary && usage.summary.by_kind.length > 0}
            <span class="dim small">cost + tokens by kind of work</span>
          {/if}
        </div>
        {#if usage.summary && usage.summary.by_kind.length > 0}
          {@const fmax = Math.max(1, ...usage.summary.by_kind.map((f) => f.total_tokens))}
          <div class="bars">
            {#each usage.summary.by_kind as f (f.feature)}
              <div class="bar-row feat-row">
                <span class="bar-name" title={f.feature}>
                  <span class="kind-badge kind-{f.feature}">{featureLabel(f.feature)}</span>
                </span>
                <div class="bar-track">
                  <div class="bar-fill stacked" style="width: {(f.total_tokens / fmax) * 100}%" title={breakdownTitle(f)}>
                    {#each tokenSegs(f) as s (s.label)}
                      {#if s.pct > 0}<div style="width: {s.pct}%; background: {s.color}"></div>{/if}
                    {/each}
                  </div>
                </div>
                <span class="bar-val">{fmtNum(f.total_tokens)}</span>
                <span class="bar-cost dim">{fmtCost(f.cost_usd)}</span>
              </div>
            {/each}
          </div>
        {:else}
          <p class="dim small">No usage recorded yet. Cost splits by feature (review, product, channels, agents…) appear here as work runs.</p>
        {/if}
      </div>

      <!-- Work-graph attribution drilldown (B1): "why did this cost so much?" -->
      <!-- Shows only when the engine is available (same guard as other panels). -->
      {#if usage.status?.available}
        <AttributionDrilldown days={usage.days} />
      {/if}

      <!-- Budgets (opt-in spend caps) -->
      <div class="panel card">
        <div class="panel-head">
          <h3>Budgets</h3>
          <button class="link-btn" onclick={() => (budgetsOpen = !budgetsOpen)}>
            {budgetsOpen ? 'Hide' : 'Configure'}
          </button>
        </div>
        <span class="dim small">
          Per-workspace / per-provider spend caps. Enforcement is opt-in — caps stay informational
          (warnings only) until you turn enforcement on.
        </span>

        <!-- Status: budget vs spend -->
        {#if usage.budgets && usage.budgets.rows.length > 0}
          <div class="budget-rows">
            {#each usage.budgets.rows as r (r.scope + ':' + r.key)}
              <div class="budget-row" class:warn={r.warning && !r.exceeded} class:over={r.exceeded}>
                <span class="budget-name" title={r.key}>
                  <span class="kind-badge">{r.scope}</span>
                  {r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key)}
                </span>
                <div class="bar-track">
                  <div
                    class="bar-fill"
                    style="width: {Math.min(100, r.used_fraction * 100)}%"
                    title="{fmtCost(r.spent_usd)} of {fmtCost(r.limit_usd)}"
                  ></div>
                </div>
                <span class="budget-val">
                  {fmtCost(r.spent_usd)} / {fmtCost(r.limit_usd)}
                  {#if r.exceeded}<span class="over-tag">over</span>
                  {:else if r.warning}<span class="warn-tag">{(r.used_fraction * 100).toFixed(0)}%</span>{/if}
                </span>
              </div>
            {/each}
          </div>
          {#if usage.budgets.config.enforce && usage.budgets.rows.some((r) => r.exceeded)}
            <div class="budget-alert">
              {usage.budgets.config.block_on_exceed
                ? 'Enforcement is ON (blocking). Work in an over-budget scope can be blocked by the daemon.'
                : 'Enforcement is ON (warn-only). Over-budget scopes are flagged but not blocked.'}
            </div>
          {/if}
          <!-- Pre-launch hint: surface the most-constrained cap as a lightweight
               "heads up" for users about to start work, without hiding it behind
               the Configure toggle. Only shows when not already in exceeded
               state (that case is covered by the alert above). -->
          {#if !usage.budgets.config.enforce || !usage.budgets.rows.some((r) => r.exceeded)}
            {#each usage.budgets.rows.filter((r) => r.warning && !r.exceeded) as r (r.scope + ':' + r.key)}
              <p class="dim small" style="margin-top: 4px;">
                Pre-launch note: {r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key)}
                is at {(r.used_fraction * 100).toFixed(0)}% of its cap
                ({fmtCost(r.spent_usd)} / {fmtCost(r.limit_usd)}).
              </p>
            {/each}
          {/if}
        {:else}
          <p class="dim small">No budgets set. Configure caps to track spend against a target.</p>
        {/if}

        <!-- Editor -->
        {#if budgetsOpen}
          <div class="budget-editor">
            <label class="cfg-row">
              <input type="checkbox" bind:checked={budgetCfg.enforce} onchange={() => (budgetsDirty = true)} />
              <span>Enforce budgets (opt-in) — warn prominently when a cap is exceeded</span>
            </label>
            <label class="cfg-row" class:disabled={!budgetCfg.enforce}>
              <input
                type="checkbox"
                bind:checked={budgetCfg.block_on_exceed}
                disabled={!budgetCfg.enforce}
                onchange={() => (budgetsDirty = true)}
              />
              <span>Block work when a cap is exceeded (otherwise warn only)</span>
            </label>
            <label class="cfg-row">
              <span>Window (days)</span>
              <input
                class="num"
                type="number"
                min="1"
                bind:value={budgetCfg.window_days}
                onchange={() => (budgetsDirty = true)}
              />
            </label>

            <div class="editor-section">
              <div class="editor-head">
                <span>Per workspace</span>
                <button class="link-btn" onclick={addWsBudget}>+ Add</button>
              </div>
              {#each budgetCfg.workspaces as b, i (i)}
                <div class="editor-line">
                  <select bind:value={b.workspace_id} onchange={() => (budgetsDirty = true)}>
                    <option value="">Select workspace…</option>
                    {#each ws.workspaces as w (w.id)}
                      <option value={w.id}>{w.name}</option>
                    {/each}
                  </select>
                  <input
                    class="num"
                    type="number"
                    min="0"
                    step="1"
                    placeholder="USD"
                    bind:value={b.monthly_usd}
                    onchange={() => (budgetsDirty = true)}
                  />
                  <button class="link-btn danger" onclick={() => removeWsBudget(i)}>Remove</button>
                </div>
              {/each}
            </div>

            <div class="editor-section">
              <div class="editor-head">
                <span>Per provider</span>
                <button class="link-btn" onclick={addProviderBudget}>+ Add</button>
              </div>
              {#each budgetCfg.providers as b, i (i)}
                <div class="editor-line">
                  <select bind:value={b.provider} onchange={() => (budgetsDirty = true)}>
                    <option value="">Select provider…</option>
                    {#each providerChoices as p (p)}
                      <option value={p}>{p}</option>
                    {/each}
                  </select>
                  <input
                    class="num"
                    type="number"
                    min="0"
                    step="1"
                    placeholder="USD"
                    bind:value={b.monthly_usd}
                    onchange={() => (budgetsDirty = true)}
                  />
                  <button class="link-btn danger" onclick={() => removeProviderBudget(i)}>Remove</button>
                </div>
              {/each}
            </div>

            <div class="editor-actions">
              <button class="btn primary" disabled={usage.savingBudgets} onclick={saveBudgets}>
                {usage.savingBudgets ? 'Saving…' : 'Save budgets'}
              </button>
            </div>
          </div>
        {/if}
      </div>

      <!-- System metrics -->
      <div class="panel card">
        <div class="panel-head">
          <h3>System metrics</h3>
          {#if latest}
            <span class="dim small">
              CPU {latest.cpu_pct.toFixed(0)}% · Mem {latest.mem_pct.toFixed(0)}%
              ({fmtNum(Math.round(latest.mem_used_mb))}/{fmtNum(Math.round(latest.mem_total_mb))} MB) ·
              ottod {latest.process_rss_mb.toFixed(0)} MB · {latest.active_sessions} active
            </span>
          {/if}
        </div>
        {#if usage.metrics.length > 1}
          <div class="metrics">
            <div class="metric">
              <span class="metric-label">CPU %</span>
              <svg viewBox="0 0 300 48" preserveAspectRatio="none" class="spark">
                <path d={sparkPath(cpuSeries, 100, 300, 48)} class="spark-cpu" />
              </svg>
            </div>
            <div class="metric">
              <span class="metric-label">Memory %</span>
              <svg viewBox="0 0 300 48" preserveAspectRatio="none" class="spark">
                <path d={sparkPath(memSeries, 100, 300, 48)} class="spark-mem" />
              </svg>
            </div>
          </div>
          <span class="dim small">Last {usage.metrics.length} samples</span>
        {:else}
          <p class="dim small">Collecting metrics… (sampled every {usage.status.metrics_interval_secs}s)</p>
        {/if}
      </div>

      <!-- Sessions leaderboard — rows are virtualized so raising SESSION_LIMIT
           (currently 50) stays DOM-bounded. The header stays fixed above the
           virtual list; rows use a CSS-grid div layout that mirrors the old
           table columns (same visual output, same column widths). -->
      <div class="panel card">
        <div class="panel-head">
          <h3>Top sessions</h3>
          {#if usage.summary && usage.summary.sessions.length > 0}
            <button class="link-btn" onclick={() => usage.exportSessionsCsv()} title="Download sessions as CSV">
              <Icon name="download" size={11} /> CSV
            </button>
          {/if}
        </div>
        {#if usage.summary && usage.summary.sessions.length > 0}
          <!-- Column header row (fixed, not virtualized) -->
          <div class="sess-head">
            <span>Session</span>
            <span>Workspace</span>
            <span>Provider / Model</span>
            <span class="num">Events</span>
            <span class="num">Tokens</span>
            <span class="num">Cost</span>
            <span>Last active</span>
          </div>
          <!-- Virtualized body: each row is ~46px (id+title or id-only ~38px,
               plus 8px border; use 46 for a safe estimate that covers titled rows). -->
          <VirtualList items={usage.summary.sessions} estimateHeight={46} class="sess-vlist">
            {#snippet row(s)}
              {@const isOttoSession = s.kind != null || s.title != null}
              <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
              <div
                class="sess-row"
                class:sess-clickable={isOttoSession}
                role={isOttoSession ? 'button' : undefined}
                tabindex={isOttoSession ? 0 : undefined}
                title={isOttoSession ? 'Open this session' : undefined}
                onclick={isOttoSession ? () => openSession(s.session_id) : undefined}
                onkeydown={isOttoSession
                  ? (e) => (e.key === 'Enter' || e.key === ' ') && openSession(s.session_id)
                  : undefined}
              >
                <div title={s.session_id}>
                  <div class="sess-top">
                    <span class="mono">{s.session_id.slice(0, 12)}</span>
                    {#if s.kind}<span class="kind-badge kind-{s.kind}">{s.kind}</span>{/if}
                  </div>
                  {#if s.title}<div class="sess-title ellip" title={s.title}>{s.title}</div>{/if}
                </div>
                <div class="dim ellip">{s.workspace_name ?? '—'}</div>
                <div>
                  <div class="model-cell">
                    <span>{s.provider}</span>
                    {#if s.model}
                      <span class="model-name dim" title={s.model}>{s.model.slice(0, 22)}</span>
                    {/if}
                  </div>
                </div>
                <div class="num">{fmtNum(s.events)}</div>
                <div class="num">
                  <div class="sess-tok">
                    <span>{fmtNum(s.total_tokens)}</span>
                    <div class="seg-bar mini" title={breakdownTitle(s)}>
                      {#each tokenSegs(s) as seg (seg.label)}
                        {#if seg.pct > 0}<div style="width: {seg.pct}%; background: {seg.color}"></div>{/if}
                      {/each}
                    </div>
                  </div>
                </div>
                <div class="num" title={s.fallback_priced ? 'Estimated — model not in the rate table; priced at the Opus tier' : undefined}>
                  {fmtCost(s.cost_usd)}
                  {#if s.fallback_priced}<span class="est-tag">est.</span>{/if}
                </div>
                <div class="dim">{fmtLastActive(s.last_active)}</div>
              </div>
            {/snippet}
          </VirtualList>
        {:else}
          <p class="dim small">No sessions recorded yet.</p>
        {/if}
      </div>

      <!-- Config / engine status -->
      {#if configOpen}
        <div class="panel card">
          <h3>Storage &amp; retention</h3>
          <div class="cfg-grid">
            <label for="cfg-retention">Retention (days)</label>
            <input id="cfg-retention" class="input" type="number" min="1" max="3650" bind:value={retention} />

            <label for="cfg-interval">Metrics sample interval (s)</label>
            <input id="cfg-interval" class="input" type="number" min="5" max="3600" bind:value={interval} />

            <label for="cfg-path">ClickHouse binary</label>
            <input id="cfg-path" class="input mono" bind:value={chPath} spellcheck="false" />
          </div>
          <div class="cfg-actions">
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
              {usage.saving ? 'Saving…' : 'Save'}
            </button>
            <button class="btn" disabled={usage.installing} onclick={() => usage.install()}>
              {usage.installing ? 'Updating…' : 'Update ClickHouse'}
            </button>
          </div>
          <div class="engine-meta">
            <span>Version: <span class="mono">{usage.status.version ?? '—'}</span></span>
            <span>Data: <span class="mono">{usage.status.data_dir}</span></span>
            <span title="{usage.status.usage_rows.toLocaleString()} usage rows · {usage.status.metric_rows.toLocaleString()} metric rows">
              On disk: {fmtBytes(usage.status.disk_bytes)}
              <span class="dim">({usage.status.usage_rows.toLocaleString()} rows)</span>
            </span>
            <span>Retention: {usage.status.retention_days}d</span>
            {#if usage.status.priced_as_of}
              <span title="Cost estimates use published rates as of this date. Unknown models fall back to the Opus tier.">
                Priced as of: <strong>{usage.status.priced_as_of}</strong>
              </span>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .usage {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .usage-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    flex-wrap: wrap;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
  }
  .title h1 {
    font-size: 15px;
    margin: 0;
  }
  .grow {
    flex: 1;
  }
  .pill {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 999px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .pill.ok {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .seg {
    display: flex;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 2px;
  }
  .seg-btn {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    padding: 3px 9px;
    border-radius: calc(var(--radius-s) - 1px);
    cursor: pointer;
  }
  .seg-btn.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: 12px;
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .btn.active {
    border-color: var(--accent);
    color: var(--accent);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .body {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
  }
  @media (min-width: 641px) and (max-width: 1024px) {
    .cards {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  @media (max-width: 640px) {
    .cards {
      grid-template-columns: 1fr;
    }
  }
  .stat {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .stat-label {
    font-size: 11px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .stat-value {
    font-size: 24px;
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .stat-sub {
    font-size: 11px;
    color: var(--text-dim);
  }

  /* Stacked token-composition bar (input · cache-write · cache-read · output) */
  .seg-bar {
    display: flex;
    height: 6px;
    margin-top: 7px;
    border-radius: 3px;
    overflow: hidden;
    background: var(--surface-2);
  }
  .seg-bar > div {
    height: 100%;
    flex-shrink: 0;
  }
  .seg-bar.big {
    height: 13px;
    margin: 2px 0 14px;
    border-radius: 6px;
  }
  .seg-bar.mini {
    height: 5px;
    width: 70px;
    margin-top: 3px;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
  }
  .lg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .lg i {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex-shrink: 0;
  }

  .bd-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 8px 18px;
  }
  .bd-item {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
  }
  .bd-item .chip {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex-shrink: 0;
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
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
  }
  .panel {
    padding: 14px;
  }
  .panel h3 {
    font-size: 12px;
    margin: 0 0 12px;
    color: var(--text);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .panel-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 12px;
    gap: 12px;
  }
  .panel-head h3 {
    margin: 0;
  }

  .bars {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .bar-row {
    display: grid;
    grid-template-columns: 80px 1fr 56px 50px;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .bar-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
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
    transition: width 200ms ease-out;
  }
  /* Stacked variant: width = provider share of max; segments = composition. */
  .bar-fill.stacked {
    display: flex;
    overflow: hidden;
    min-width: 2px;
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
    font-size: 10px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
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
    fill: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  .metrics {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  .metric {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .metric-label {
    font-size: 11px;
    color: var(--text-dim);
  }
  .spark {
    width: 100%;
    height: 48px;
  }
  .spark path {
    fill: none;
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  .spark-cpu {
    stroke: var(--accent);
  }
  .spark-mem {
    stroke: #10b981;
  }

  /* Sessions leaderboard: fixed column header + VirtualList rows ----------- */
  /* 7-column CSS grid: session · workspace · provider/model · events · tokens · cost · last active */
  .sess-head,
  .sess-row {
    display: grid;
    grid-template-columns: minmax(130px, 2fr) minmax(80px, 1fr) minmax(100px, 1.5fr) 56px 90px 64px minmax(90px, 1fr);
    align-items: center;
    gap: 0;
    font-size: 12px;
  }
  .sess-head {
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
    font-weight: 500;
    padding: 5px 0;
  }
  .sess-head > span,
  .sess-row > div {
    padding: 5px 8px;
  }
  .sess-row {
    border-bottom: 1px solid var(--surface-2);
    color: var(--text);
    transition: background 120ms ease-out;
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
  }
  /* Session Tokens cell: total over a compact composition mini-bar. */
  .sess-tok {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 1px;
  }
  .sess-tok .seg-bar.mini {
    margin-top: 2px;
  }
  .ellip {
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Session cell: id + kind badge on top, title (pane name) below */
  .sess-top {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .sess-title {
    margin-top: 2px;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--text-dim);
  }
  .kind-badge {
    flex-shrink: 0;
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .kind-review {
    background: color-mix(in srgb, #f59e0b 20%, transparent);
    color: #b45309;
  }
  .kind-product {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .kind-channel {
    background: color-mix(in srgb, var(--status-working, #4a9eff) 20%, transparent);
    color: var(--status-working, #4a9eff);
  }
  .kind-agent {
    background: color-mix(in srgb, #8b5cf6 20%, transparent);
    color: #8b5cf6;
  }
  .kind-swarm {
    background: color-mix(in srgb, #10b981 22%, transparent);
    color: #0f9d6e;
  }
  .kind-connection {
    background: color-mix(in srgb, #64748b 22%, transparent);
    color: #64748b;
  }

  /* By-feature rows: widen the label column so the feature badge fits. */
  .feat-row {
    grid-template-columns: 120px 1fr 56px 50px;
  }
  .feat-row .bar-name {
    overflow: visible;
  }

  .cfg-grid {
    display: grid;
    grid-template-columns: 200px 1fr;
    gap: 10px 12px;
    align-items: center;
    max-width: min(620px, 92vw);
  }
  .cfg-grid label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .cfg-actions {
    display: flex;
    gap: 8px;
    margin-top: 14px;
  }
  .engine-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    margin-top: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--surface-2);
    font-size: 11px;
    color: var(--text-dim);
  }

  .input {
    height: 28px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
    width: 100%;
  }
  .input.mono,
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
  }

  .install {
    margin: 40px auto;
    max-width: 520px;
    padding: 28px 30px;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 10px;
    color: var(--text-dim);
  }
  .install h2 {
    font-size: 16px;
    margin: 4px 0 0;
    color: var(--text);
  }
  .install p {
    font-size: 12.5px;
    line-height: 1.55;
    margin: 0;
  }
  .install-cmd {
    width: 100%;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 9px 12px;
    margin: 4px 0;
  }
  .install-cmd code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
    color: var(--text);
  }
  .install-actions {
    margin: 4px 0 8px;
  }
  .path-row {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    text-align: start;
    border-top: 1px solid var(--surface-2);
    padding-top: 14px;
  }
  .path-row label {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .path-input {
    display: flex;
    gap: 8px;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: 11.5px;
  }
  code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  /* --- Budgets --------------------------------------------------------- */
  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
    padding: 0;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
  .link-btn.danger {
    color: var(--danger, #c0392b);
  }
  .budget-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
  }
  .budget-row {
    display: grid;
    grid-template-columns: minmax(120px, 1.4fr) 2fr minmax(120px, auto);
    align-items: center;
    gap: 10px;
  }
  .budget-name {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .budget-val {
    font-size: 11.5px;
    color: var(--text-dim);
    text-align: end;
    white-space: nowrap;
  }
  .budget-row.warn .bar-fill {
    background: var(--warn, #d08a18);
  }
  .budget-row.over .bar-fill {
    background: var(--danger, #c0392b);
  }
  .warn-tag {
    color: var(--warn, #d08a18);
    font-weight: 600;
    margin-inline-start: 4px;
  }
  .over-tag {
    color: var(--danger, #c0392b);
    font-weight: 600;
    margin-inline-start: 4px;
    text-transform: uppercase;
  }
  /* Live WS budget-exceeded banner — shown at the top of the Usage page. */
  .budget-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    margin: 0 0 8px;
    border-radius: var(--radius-s, 6px);
    background: color-mix(in srgb, var(--danger, #c0392b) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger, #c0392b) 40%, transparent);
    color: var(--text);
    font-size: 12.5px;
  }
  .budget-banner.recovered {
    background: color-mix(in srgb, var(--success, #27ae60) 12%, transparent);
    border-color: color-mix(in srgb, var(--success, #27ae60) 40%, transparent);
  }
  .budget-banner span {
    flex: 1;
  }
  .budget-banner .close-btn {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    color: var(--text-dim);
    padding: 0 2px;
  }
  .budget-banner .close-btn:hover {
    color: var(--text);
  }
  .budget-alert {
    margin-top: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-s, 6px);
    background: color-mix(in srgb, var(--danger, #c0392b) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger, #c0392b) 40%, transparent);
    color: var(--text);
    font-size: 12px;
  }
  .budget-editor {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cfg-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .cfg-row.disabled {
    opacity: 0.55;
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
    font-size: 12px;
    font-weight: 600;
    color: var(--text-dim);
  }
  .editor-line {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .editor-line select {
    flex: 1;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    font-size: 12px;
    padding: 4px 6px;
  }
  input.num {
    width: 90px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    font-size: 12px;
    padding: 4px 6px;
    text-align: end;
  }
  .editor-actions {
    display: flex;
    justify-content: flex-end;
  }

  @media (max-width: 640px) {
    .usage-head {
      padding: 8px 12px;
      gap: 6px;
    }
    /* The grow spacer collapses so the seg groups wrap to their own rows */
    .grow {
      flex-basis: 100%;
    }
    .body {
      padding: 10px;
    }
    /* Two-column card grid → single column */
    .grid {
      grid-template-columns: 1fr;
    }
    /* Bar rows: replace fixed-width name + cost columns with a wrapping layout */
    .bar-row {
      grid-template-columns: 1fr 56px;
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
      text-align: end;
    }
    .bar-row .bar-track {
      grid-column: 1 / 3;
      grid-row: 2;
    }
    .bar-row .bar-cost {
      display: none; /* keep layout tight; cost shown on bar hover title */
    }
    .feat-row {
      grid-template-columns: 1fr 56px;
      grid-template-rows: auto auto;
      gap: 4px 8px;
    }
    .feat-row .bar-name {
      grid-column: 1;
      grid-row: 1;
      overflow: visible;
    }
    .feat-row .bar-val {
      grid-column: 2;
      grid-row: 1;
      text-align: end;
    }
    .feat-row .bar-track {
      grid-column: 1 / 3;
      grid-row: 2;
    }
    .feat-row .bar-cost {
      display: none;
    }
    /* Budget rows: narrower label column */
    .budget-row {
      grid-template-columns: minmax(80px, 1fr) 1.5fr;
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
      text-align: end;
      white-space: normal;
      font-size: 11px;
    }
    .budget-row .bar-track {
      grid-column: 1 / 3;
      grid-row: 2;
    }
    /* Config grid: one column */
    .cfg-grid {
      grid-template-columns: 1fr;
    }
    /* Sessions: collapse workspace + last-active columns; keep session/tokens/cost readable. */
    .sess-head,
    .sess-row {
      grid-template-columns: minmax(100px, 2fr) minmax(80px, 1.5fr) 56px 72px 56px;
    }
    /* Hide workspace + last-active columns on narrow screens. */
    .sess-head > span:nth-child(2),
    .sess-head > span:nth-child(7),
    .sess-row > div:nth-child(2),
    .sess-row > div:nth-child(7) {
      display: none;
    }
    /* Metrics: stack vertically */
    .metrics {
      grid-template-columns: 1fr;
    }
  }

  /* "Estimated" cost tag — shown when the model is not in the rate table. */
  .est-tag {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 4px;
    border-radius: 3px;
    margin-inline-start: 4px;
    background: color-mix(in srgb, #f59e0b 22%, transparent);
    color: #b45309;
    vertical-align: middle;
  }

  /* Clickable session row: hover highlight + pointer cursor. The hover applies
     to the whole div row (not individual <td> cells, since we use divs now). */
  .sess-clickable {
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .sess-clickable:hover {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }

  /* Provider / model two-line cell in the sessions table. */
  .model-cell {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .model-name {
    font-size: 10.5px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 140px;
  }

  /* Pricing note shown at the bottom of the install card. */
  .install-meta {
    font-size: 11px;
    text-align: center;
    max-width: 420px;
    line-height: 1.5;
  }
</style>
