<script lang="ts">
  // Usage dashboard (root-only): provider/day/session token rollups, system
  // CPU/RAM metrics, and the embedded-ClickHouse install/retention controls.
  // All data comes from the daemon's /usage/* endpoints (otto-usage engine).
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { usage } from '../../lib/api/usage.svelte';

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

  onMount(() => {
    if (auth.isRoot) void usage.loadAll();
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

  // ── Daily bar chart geometry ──────────────────────────────────────────────
  const dailyMax = $derived(
    Math.max(1, ...(usage.summary?.daily ?? []).map((d) => d.total_tokens)),
  );

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
      <button class="btn" class:active={configOpen} onclick={() => (configOpen = !configOpen)} title="Settings">
        <Icon name="gear" size={13} />
      </button>
    {/if}
  </header>

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
            <span class="stat-sub">over {usage.summary.days}d</span>
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
          <h3>By provider</h3>
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

        <!-- Daily tokens -->
        <div class="panel card">
          <h3>Daily tokens</h3>
          {#if usage.summary && usage.summary.daily.length > 0}
            <div class="daily">
              {#each usage.summary.daily as d (d.day)}
                <div class="daily-col" title="{d.day}: {breakdownTitle(d)} · {fmtCost(d.cost_usd)}">
                  <div class="daily-bar" style="height: {Math.max(2, (d.total_tokens / dailyMax) * 100)}%">
                    {#each tokenSegs(d) as s (s.label)}
                      {#if s.pct > 0}<div class="daily-seg" style="height: {s.pct}%; background: {s.color}"></div>{/if}
                    {/each}
                  </div>
                  <span class="daily-label">{shortDay(d.day)}</span>
                </div>
              {/each}
            </div>
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

      <!-- Sessions table -->
      <div class="panel card">
        <h3>Top sessions</h3>
        {#if usage.summary && usage.summary.sessions.length > 0}
          <table class="tbl">
            <thead>
              <tr>
                <th>Session</th>
                <th>Workspace</th>
                <th>Provider</th>
                <th class="num">Events</th>
                <th class="num">Tokens</th>
                <th class="num">Cost</th>
                <th>Last active</th>
              </tr>
            </thead>
            <tbody>
              {#each usage.summary.sessions as s (s.session_id)}
                <tr>
                  <td title={s.session_id}>
                    <div class="sess-top">
                      <span class="mono">{s.session_id.slice(0, 12)}</span>
                      {#if s.kind}<span class="kind-badge kind-{s.kind}">{s.kind}</span>{/if}
                    </div>
                    {#if s.title}<div class="sess-title ellip" title={s.title}>{s.title}</div>{/if}
                  </td>
                  <td class="dim ellip">{s.workspace_name ?? '—'}</td>
                  <td>{s.provider}</td>
                  <td class="num">{fmtNum(s.events)}</td>
                  <td class="num">
                    <div class="sess-tok">
                      <span>{fmtNum(s.total_tokens)}</span>
                      <div class="seg-bar mini" title={breakdownTitle(s)}>
                        {#each tokenSegs(s) as seg (seg.label)}
                          {#if seg.pct > 0}<div style="width: {seg.pct}%; background: {seg.color}"></div>{/if}
                        {/each}
                      </div>
                    </div>
                  </td>
                  <td class="num">{fmtCost(s.cost_usd)}</td>
                  <td class="dim">{fmtLastActive(s.last_active)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
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
            <span>On disk: {fmtBytes(usage.status.disk_bytes)}</span>
            <span>{usage.status.usage_rows.toLocaleString()} usage rows · {usage.status.metric_rows.toLocaleString()} metric rows</span>
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
    margin-left: auto;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    color: var(--text);
  }
  .bd-pct {
    min-width: 32px;
    text-align: right;
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
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .bar-cost {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .daily {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 130px;
  }
  .daily-col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    height: 100%;
    gap: 4px;
  }
  .daily-bar {
    width: 100%;
    max-width: 22px;
    border-radius: 3px 3px 0 0;
    overflow: hidden;
    display: flex;
    flex-direction: column-reverse;
    transition: height 200ms ease-out;
  }
  .daily-seg {
    width: 100%;
    flex-shrink: 0;
  }
  .daily-label {
    font-size: 9px;
    color: var(--text-dim);
    white-space: nowrap;
    transform: rotate(-45deg);
    transform-origin: center;
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

  .tbl {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .tbl th {
    text-align: left;
    color: var(--text-dim);
    font-weight: 500;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
  }
  .tbl th.num,
  .tbl td.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .tbl td {
    padding: 5px 8px;
    border-bottom: 1px solid var(--surface-2);
    color: var(--text);
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
    max-width: 620px;
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
    text-align: left;
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
</style>
