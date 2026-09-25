<script lang="ts" module>
  import type { MetricKey, Delta } from './insightsParse';

  export type ViewMode = 'preview' | 'markdown' | 'html';

  /** One KPI tile, pre-computed by the page (value + delta + trend). */
  export interface Kpi {
    key: MetricKey;
    label: string;
    value: number;
    /** Where the value came from, for the tooltip. */
    source: 'summary' | 'index';
    delta: Delta | null;
    /** Oldest → newest, ending at this report; null = no reading that period. */
    trend: (number | null)[];
    trendLabels: string[];
  }
</script>

<script lang="ts">
  // The right-hand pane of Insights → Reports: one report, rendered.
  //
  //   header   — period chip · date · generated-at, the one-line headline, and
  //              the Preview / Markdown / HTML switch + window/export tools
  //   Preview  — key findings parsed out of the summary markdown (KPI tiles
  //              with deltas + sparklines, the Action Plan as a checklist with
  //              effort + status chips and ledger detail), then the rest of
  //              the summary rendered through the sanitized GFM renderer
  //   Markdown — the summary source, verbatim
  //   HTML     — the agent-generated report in a SANDBOXED iframe: srcdoc with
  //              `allow-scripts` but never `allow-same-origin`, so its charts
  //              run while the app's storage (bearer token) stays out of reach
  //
  // Parsing is best-effort (insightsParse.ts): when the summary doesn't have
  // the expected shape, the key-findings block simply doesn't render and the
  // markdown shows as-is.
  import type { InsightReport } from '../../lib/api/types';
  import { insightsApi } from '../../lib/api/insights';
  import { renderMarkdownGfm } from '../../lib/md';
  import { rel } from '../../lib/stores/now.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Sparkline from '../../lib/components/Sparkline.svelte';
  import {
    itemStatus,
    statusLabel,
    statusTone,
    type ActionItem,
    type LedgerEntry,
    type ParsedSummary,
  } from './insightsParse';

  interface Props {
    report: InsightReport;
    parsed: ParsedSummary;
    /** The summary markdown (full file when it could be read, else the listed excerpt). */
    markdown: string;
    kpis: Kpi[];
    /** Label of the report the deltas compare against ("Tue 22 Sep"), if any. */
    compareLabel: string | null;
    ledger: Map<string, LedgerEntry>;
    mode: ViewMode;
    onmode: (m: ViewMode) => void;
    onopenwindow: () => void;
    onexportmd: () => void;
    ondownloadhtml: () => void;
  }
  let { report, parsed, markdown, kpis, compareLabel, ledger, mode, onmode, onopenwindow, onexportmd, ondownloadhtml }: Props =
    $props();

  const hasHtml = $derived(!!report.html_path);
  const effectiveMode = $derived<ViewMode>(mode === 'html' && !hasHtml ? 'preview' : mode);

  // ---- HTML (lazy, per report) -------------------------------------------
  let html = $state<string | null>(null);
  let htmlFor = $state<string | null>(null);
  let htmlError = $state<string | null>(null);
  let htmlLoading = $state(false);

  async function loadHtml(path: string): Promise<void> {
    htmlLoading = true;
    htmlError = null;
    try {
      const text = await insightsApi.readText(path);
      if (report.html_path === path) {
        html = text;
        htmlFor = path;
      }
    } catch (e) {
      htmlError = e instanceof Error ? e.message : String(e);
    } finally {
      htmlLoading = false;
    }
  }
  $effect(() => {
    const p = report.html_path;
    if (effectiveMode === 'html' && p && htmlFor !== p && !htmlLoading) void loadHtml(p);
  });

  // ---- Display helpers ------------------------------------------------------
  function kindLabel(k: string): string {
    return k === 'adhoc' ? 'Ad-hoc' : k ? k[0].toUpperCase() + k.slice(1) : '';
  }
  function dateLabel(iso: string): string {
    const d = new Date(`${iso}T00:00:00`);
    return Number.isNaN(d.getTime())
      ? iso
      : d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
  }
  const periodText = $derived(
    report.period_start === report.period_end
      ? dateLabel(report.period_start)
      : `${dateLabel(report.period_start)} – ${dateLabel(report.period_end)}`,
  );

  function fmt(key: Kpi['key'], v: number): string {
    if (key === 'spend') return v >= 1000 ? `$${Math.round(v).toLocaleString()}` : `$${v.toFixed(2)}`;
    if (key === 'achievement') return `${Math.round(v)}%`;
    return Math.round(v).toLocaleString();
  }
  function deltaText(k: Kpi): string {
    const d = k.delta;
    if (!d) return '';
    if (d.direction === 'flat') return 'No change';
    if (k.key === 'achievement') return `${Math.abs(Math.round(d.diff))} pts`;
    if (d.pct != null) return `${Math.abs(Math.round(d.pct * 100))}%`;
    return fmt(k.key, Math.abs(d.diff));
  }
  function deltaSr(k: Kpi): string {
    const d = k.delta;
    if (!d) return '';
    const vs = compareLabel ? ` vs ${compareLabel}` : ' vs the previous report';
    if (d.direction === 'flat') return `No change${vs}`;
    const verdict = d.tone === 'good' ? ', better' : d.tone === 'bad' ? ', worse' : '';
    return `${d.direction === 'up' ? 'Up' : 'Down'} ${deltaText(k)}${vs}${verdict}`;
  }

  const bodyHtml = $derived(renderMarkdownGfm(parsed.body));
  const regressed = $derived(parsed.actions.filter((a) => itemStatus(a, ledger) === 'regressed').length);

  // Action rows expand to show their ledger entries.
  let expanded = $state<Set<number>>(new Set());
  function toggle(i: number): void {
    const next = new Set(expanded);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    expanded = next;
  }
  $effect(() => {
    void report.html_path;
    void report.period_end;
    expanded = new Set();
  });

  function ledgerFor(a: ActionItem): LedgerEntry[] {
    return a.ids.map((id) => ledger.get(id)).filter((x): x is LedgerEntry => !!x);
  }
  function markerIcon(s: string | null): 'check' | 'warning' | 'arrowUp' | 'dot' {
    if (s === 'closed') return 'check';
    if (s === 'regressed') return 'warning';
    if (s === 'improved') return 'arrowUp';
    return 'dot';
  }
  function periodKeyLabel(k: string): string {
    const m = /^(\w+):(\d{4})(\d{2})(\d{2})_/.exec(k);
    if (!m) return k;
    return `${kindLabel(m[1])} ${dateLabel(`${m[2]}-${m[3]}-${m[4]}`)}`;
  }
</script>

<article class="report" data-testid="insight-report">
  <header class="r-head">
    <div class="r-meta">
      <span class="chip">{kindLabel(report.kind)}</span>
      <span class="r-period">{periodText}</span>
      {#if report.created_at}
        <span class="dim r-created" title={new Date(report.created_at).toLocaleString()}>· generated {rel(report.created_at)}</span>
      {/if}
    </div>
    {#if parsed.headline}
      <h2 class="r-headline">{parsed.headline}</h2>
    {:else if parsed.title}
      <h2 class="r-headline">{parsed.title}</h2>
    {/if}
    <div class="r-tools">
      <div class="segmented" role="group" aria-label="Report view">
        <button aria-pressed={effectiveMode === 'preview'} class:active={effectiveMode === 'preview'} onclick={() => onmode('preview')}>Preview</button>
        <button aria-pressed={effectiveMode === 'markdown'} class:active={effectiveMode === 'markdown'} onclick={() => onmode('markdown')}>Markdown</button>
        <button
          aria-pressed={effectiveMode === 'html'}
          class:active={effectiveMode === 'html'}
          disabled={!hasHtml}
          title={hasHtml ? 'The full HTML report' : 'This period has no HTML report yet'}
          onclick={() => onmode('html')}>HTML</button
        >
      </div>
      <span class="grow"></span>
      <button class="btn small ghost" onclick={onexportmd} title="Download the summary as a .md file"><Icon name="download" size={12} /> Export .md</button>
      {#if hasHtml}
        <button class="icon-btn" onclick={ondownloadhtml} aria-label="Download HTML report" title="Download HTML report"><Icon name="file" size={14} /></button>
        <button class="icon-btn" onclick={onopenwindow} aria-label="Open in new window" title="Open in new window"><Icon name="external" size={14} /></button>
      {/if}
    </div>
  </header>

  {#if effectiveMode === 'html'}
    <div class="r-html">
      {#if htmlError}
        <div class="r-error" role="alert">
          <Icon name="warning" size={14} />
          <div>
            <strong>Couldn't load the HTML report.</strong>
            <span class="dim">The file may still be being written. Retry, or use Preview.</span>
            <span class="dim mono err-detail">{htmlError}</span>
          </div>
          <button class="btn small" onclick={() => report.html_path && loadHtml(report.html_path)}>Retry</button>
        </div>
      {:else if html == null || htmlLoading}
        <p class="dim r-loading" role="status">Loading the HTML report…</p>
      {:else}
        <!-- Sandboxed: scripts run (charts), but never same-origin. -->
        <iframe
          class="r-frame"
          sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox"
          srcdoc={html}
          title="Insight report {periodText}"
        ></iframe>
      {/if}
    </div>
  {:else if effectiveMode === 'markdown'}
    <pre class="r-source" dir="ltr" data-testid="report-markdown">{markdown}</pre>
  {:else}
    <div class="r-scroll">
      {#if kpis.length > 0}
        <section class="r-section" aria-labelledby="kf-title">
          <div class="r-section-head">
            <h3 id="kf-title" class="section-title">Key findings</h3>
            {#if compareLabel}<span class="dim r-vs">Compared with {compareLabel}</span>{/if}
          </div>
          <div class="kpis">
            {#each kpis as k (k.key)}
              <div class="kpi card" data-testid="kpi-{k.key}">
                <div class="kpi-label" title={k.source === 'summary' ? 'From the report summary' : 'From the metrics index'}>{k.label}</div>
                <div class="kpi-row">
                  <span class="kpi-value">{fmt(k.key, k.value)}</span>
                  {#if k.delta}
                    <span class="kpi-delta {k.delta.tone}" title={deltaSr(k)}>
                      {#if k.delta.direction === 'up'}<Icon name="arrowUp" size={12} />{:else if k.delta.direction === 'down'}<Icon name="arrowDown" size={12} />{/if}
                      <span aria-hidden="true">{deltaText(k)}</span>
                      <span class="sr-only">{deltaSr(k)}</span>
                    </span>
                  {/if}
                </div>
                {#if k.trend.filter((v) => v != null).length >= 2}
                  <Sparkline values={k.trend} labels={k.trendLabels} name={k.label} format={(v) => fmt(k.key, v)} width={120} height={26} />
                {/if}
              </div>
            {/each}
          </div>
        </section>
      {/if}

      {#if parsed.actions.length > 0}
        <section class="r-section" aria-labelledby="ap-title">
          <div class="r-section-head">
            <h3 id="ap-title" class="section-title">Action plan</h3>
            <span class="dim r-vs">
              {parsed.actions.length} item{parsed.actions.length === 1 ? '' : 's'}{#if regressed > 0}{' · '}<span class="warn-text">{regressed} regressed</span>{/if}
            </span>
          </div>
          <ol class="actions" data-testid="action-plan">
            {#each parsed.actions as a (a.index)}
              {@const st = itemStatus(a, ledger)}
              {@const entries = ledgerFor(a)}
              <li class="action">
                <span class="marker {st ? statusTone(st) : 'neutral'}" title={st ? statusLabel(st) : 'No status'}>
                  <Icon name={markerIcon(st)} size={12} />
                </span>
                <div class="a-main">
                  <div class="a-top">
                    <span class="a-title">{a.title}</span>
                    <span class="a-chips">
                      {#each a.statuses.length ? a.statuses : st ? [st] : [] as s (s)}
                        <span class="chip tone-{statusTone(s)}">{statusLabel(s)}</span>
                      {/each}
                      {#if a.effort}
                        <span class="chip" title="Effort: {a.effort === 'S' ? 'small' : a.effort === 'M' ? 'medium' : 'large'}">Effort {a.effort}</span>
                      {/if}
                    </span>
                  </div>
                  {#if a.metric || a.current}
                    <div class="a-metric">
                      {#if a.metric}<span class="dim">{a.metric}</span>{/if}
                      {#if a.current}
                        <span class="a-values"><span>{a.current}</span>{#if a.target}<Icon name="chevronRight" size={12} /><strong>{a.target}</strong>{/if}</span>
                      {/if}
                    </div>
                  {/if}
                  {#if a.ids.length > 0}
                    <div class="a-ids">
                      {#each a.ids as id (id)}<span class="mono a-id" title={ledger.has(id) ? 'In the action ledger' : 'Not found in the ledger'}>{id}</span>{/each}
                      {#if entries.length > 0}
                        <button class="btn small ghost a-ledger-btn" aria-expanded={expanded.has(a.index)} onclick={() => toggle(a.index)}>
                          <Icon name={expanded.has(a.index) ? 'chevronDown' : 'chevronRight'} size={12} /> Ledger
                        </button>
                      {/if}
                    </div>
                  {/if}
                  {#if expanded.has(a.index) && entries.length > 0}
                    <div class="ledger" role="region" aria-label="Ledger entries for {a.title}">
                      {#each entries as e (e.id)}
                        <dl class="ledger-row">
                          <div><dt>Entry</dt><dd class="mono">{e.id}</dd></div>
                          <div><dt>Status</dt><dd><span class="chip tone-{statusTone(e.status)}">{statusLabel(e.status)}</span></dd></div>
                          {#if e.openedPeriod}<div><dt>Opened</dt><dd>{periodKeyLabel(e.openedPeriod)}{#if e.openedValue} · {e.openedValue}{/if}</dd></div>{/if}
                          {#if e.latestValue}<div><dt>Latest</dt><dd>{e.latestValue}</dd></div>{/if}
                          {#if e.targetValue}<div><dt>Target</dt><dd>{e.targetValue}</dd></div>{/if}
                        </dl>
                      {/each}
                    </div>
                  {/if}
                </div>
              </li>
            {/each}
          </ol>
        </section>
      {/if}

      {#if parsed.body.trim()}
        <section class="r-section" aria-labelledby="sum-title">
          {#if kpis.length > 0 || parsed.actions.length > 0}
            <div class="r-section-head"><h3 id="sum-title" class="section-title">Summary</h3></div>
          {/if}
          <!-- renderMarkdownGfm output goes through the allowlist sanitizer. -->
          <div class="md-body report-md" data-testid="report-rendered">{@html bodyHtml}</div>
        </section>
      {/if}
      <p class="dim r-attrib"><Icon name="sparkle" size={12} /> Written by the <code>insights</code> skill from your agent transcripts. Figures are the agent's reading; open HTML for the charts.</p>
    </div>
  {/if}
</article>

<style>
  .report {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .r-head {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 20px 10px;
    border-bottom: 1px solid var(--border);
  }
  .r-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .r-period {
    font-weight: 600;
    color: var(--text);
  }
  .r-created {
    font-size: var(--fs-s);
  }
  .r-headline {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    line-height: 1.4;
    max-width: 90ch;
    color: var(--text);
  }
  .r-tools {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 4px;
  }
  .segmented > button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .r-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 20px 32px;
  }
  .r-section {
    max-width: 1100px;
    margin-top: 16px;
  }
  .r-section-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 8px;
  }
  .r-section-head .section-title {
    margin: 0;
  }
  .r-vs {
    font-size: var(--fs-s);
  }
  .warn-text {
    color: var(--warning);
  }

  /* KPI tiles (dataviz stat-tile contract: label · value · delta · trend). */
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 8px;
  }
  .kpi {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    min-width: 0;
  }
  .kpi-label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .kpi-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }
  .kpi-value {
    font-size: var(--fs-xl);
    font-weight: 600;
    color: var(--text);
  }
  .kpi-delta {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .kpi-delta.good {
    color: var(--success);
  }
  .kpi-delta.bad {
    color: var(--warning);
  }

  /* Action plan checklist */
  .actions {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .action {
    display: flex;
    gap: 10px;
    padding: 10px 12px;
  }
  .action + .action {
    border-top: 1px solid var(--border);
  }
  .marker {
    flex: none;
    width: 20px;
    height: 20px;
    margin-top: 1px;
    border-radius: 999px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    color: var(--text-dim);
    background: var(--surface-2);
  }
  .marker.warning {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
  }
  .marker.success {
    color: var(--success);
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 35%, transparent);
  }
  .marker.info {
    color: var(--info);
    background: var(--info-soft);
    border-color: color-mix(in srgb, var(--info) 35%, transparent);
  }
  .a-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .a-top {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    flex-wrap: wrap;
  }
  .a-title {
    flex: 1 1 260px;
    min-width: 0;
    font-weight: 600;
    color: var(--text);
    line-height: 1.4;
  }
  .a-chips {
    display: inline-flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .chip.tone-warning {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
  }
  .chip.tone-success {
    color: var(--success);
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 35%, transparent);
  }
  .chip.tone-info {
    color: var(--info);
    background: var(--info-soft);
    border-color: color-mix(in srgb, var(--info) 35%, transparent);
  }
  .a-metric {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .a-values {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .a-values :global(svg) {
    color: var(--text-dim);
  }
  .a-values strong {
    font-weight: 600;
  }
  .a-ids {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .a-id {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .a-ledger-btn {
    height: 20px;
    padding: 0 6px;
  }
  .ledger {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 2px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .ledger-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    margin: 0;
    font-size: var(--fs-s);
  }
  .ledger-row > div {
    display: flex;
    gap: 6px;
    align-items: baseline;
    min-width: 0;
  }
  .ledger-row dt {
    color: var(--text-dim);
  }
  .ledger-row dd {
    margin: 0;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  /* Rendered summary typography — a reading column, not a wall. */
  .report-md {
    max-width: 78ch;
    color: var(--text);
  }
  .report-md :global(h1),
  .report-md :global(h2),
  .report-md :global(h3) {
    font-size: var(--fs-m);
    font-weight: 600;
    margin: 16px 0 6px;
  }
  .report-md :global(p) {
    margin: 0 0 10px;
  }
  .report-md :global(ul),
  .report-md :global(ol) {
    padding-inline-start: 22px;
  }
  .report-md :global(li) {
    margin: 3px 0;
  }
  .report-md :global(table) {
    border-collapse: collapse;
    display: block;
    overflow-x: auto;
    font-size: var(--fs-s);
  }
  .report-md :global(th),
  .report-md :global(td) {
    border: 1px solid var(--border);
    padding: 4px 8px;
    text-align: start;
  }
  .report-md :global(a) {
    color: var(--accent-text);
  }
  .r-attrib {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 20px 0 0;
    font-size: var(--fs-xs);
  }

  .r-source {
    flex: 1;
    min-height: 0;
    margin: 0;
    overflow: auto;
    padding: 16px 20px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text);
    user-select: text;
  }

  .r-html {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .r-frame {
    flex: 1;
    width: 100%;
    border: none;
    /* User content preview: the report paints its own page. */
    background: white;
  }
  .r-loading {
    padding: 20px;
    margin: 0;
  }
  .r-error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin: 16px 20px;
    padding: 12px 14px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .r-error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .r-error > div {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .err-detail {
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  @media (max-width: 640px) {
    .r-head {
      padding: 12px 14px 10px;
    }
    .r-scroll {
      padding: 4px 14px 24px;
    }
    .kpis {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
