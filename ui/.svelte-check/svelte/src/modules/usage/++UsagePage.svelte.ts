///<reference types="svelte" />
;
import { scrollBehavior } from '../../lib/motion';
import { loadErrorText } from '../../lib/loadError';
import { announce, announceShown } from '../../lib/announce.svelte';
import PathField from '../../lib/components/PathField.svelte';
// Usage dashboard: tokens first (provider/day/model/session rollups; cost is
// the secondary figure), the ccusage-style Report tab, and — for root only —
// system CPU/RAM metrics, the ccusage cross-check and the embedded-ClickHouse
// install/retention/budget controls. A non-root user holding Usage:View sees
// their own sessions (`summary.scope === 'own'`). All data comes from the
// daemon's /usage/* endpoints (otto-usage engine).
import { onMount } from 'svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import { onTabKey } from '../../lib/tabKeys';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { kindLabel } from '../../lib/labels';
import { pluralNoun } from '../../lib/plural';
import { registry } from '../../lib/commands.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import { toasts } from '../../lib/toast.svelte';
import { formatBytes, formatCount, formatUsd } from '../../lib/metric-format';
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
import ModelBreakdown from './ModelBreakdown.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  let reportLoad = $state(0);

  /** Root sees everything and owns the engine/budget controls; anyone else
   *  with Usage:View gets a read-only view of their own sessions. */
  const admin = $derived(auth.isRoot);
  const canView = $derived(auth.isRoot || auth.can('usage', 'view'));
  /** Overview dashboard vs the ccusage-style report tables. */
  let view = $state<'overview' | 'report' | 'otto'>('overview');
  const ottoUsage = () => import('./OttoUsage.svelte');
  /** Bumped by the Otto-usage Retry: re-keys the block for a fresh import(). */
  let ottoLoad = $state(0);
  let telemetryDirty = $state(false);
  async function selectView(next: typeof view): Promise<void> {
    if (next === view) return;
    if (telemetryDirty && !await confirmer.ask('You have unsaved telemetry settings. Leaving now discards them.', { title: 'Discard unsaved changes?', confirmLabel: 'Discard', cancelLabel: 'Keep editing', danger: true })) return;
    telemetryDirty = false;
    view = next;
  }

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
  const storageDirty = $derived(
    configOpen &&
      !!usage.status &&
      (retention !== usage.status.retention_days ||
        interval !== usage.status.metrics_interval_secs ||
        chPath.trim() !== (usage.status.binary ?? '')),
  );
  const storageError = $derived(
    !Number.isInteger(retention) || retention < 1 || retention > 3650
      ? 'Keep history must be a whole number of days from 1 to 3650.'
      : !Number.isInteger(interval) || interval < 5 || interval > 3600
        ? 'Sample interval must be a whole number of seconds from 5 to 3600.'
        : '',
  );

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
  let budgetValidated = $state(false);
  $effect(() => guardUnsaved(() => budgetsDirty || telemetryDirty || storageDirty, { what: 'your usage settings' }));
  const budgetWindowInvalid = $derived(!Number.isInteger(budgetCfg.window_days) || budgetCfg.window_days < 1 || budgetCfg.window_days > 3650);
  const budgetValidation = $derived.by(() => {
    if (budgetWindowInvalid)
      return 'Choose a whole number of days from 1 to 3650.';
    if ([...budgetCfg.workspaces, ...budgetCfg.providers].some((b) => b.monthly_usd != null && (!Number.isFinite(b.monthly_usd) || b.monthly_usd < 0)))
      return 'Caps must be zero or a positive amount in US dollars.';
    const providers = budgetCfg.providers.filter((b) => b.provider && b.monthly_usd > 0).map((b) => b.provider);
    if (new Set(providers).size !== providers.length) return 'Use one cap per provider. Remove the duplicate row or choose another provider.';
    const workspaces = budgetCfg.workspaces.filter((b) => b.workspace_id && b.monthly_usd > 0).map((b) => b.workspace_id);
    if (new Set(workspaces).size !== workspaces.length) return 'Use one cap per workspace. Remove the duplicate row or choose another workspace.';
    return '';
  });

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
    if (configOpen) queueMicrotask(() => settingsEl?.scrollIntoView({ block: 'nearest', behavior: scrollBehavior() }));
  }

  // ⌘K verbs while Usage is open.
  $effect(() =>
    registry.register('usage', [
      { id: 'usage.refresh', title: 'Refresh usage', group: 'Usage', keywords: 'reload tokens cost', run: () => void usage.loadAll() },
      { id: 'usage.view-overview', title: 'Show usage overview', group: 'Usage', keywords: 'dashboard charts', run: () => void selectView('overview') },
      { id: 'usage.view-report', title: 'Show usage report tables', group: 'Usage', keywords: 'ccusage daily monthly table', run: () => void selectView('report') },
      ...WINDOWS.map((w) => ({ id: `usage.window-${w.days}`, title: `Show the last ${w.days} days`, group: 'Usage', keywords: `usage window range ${w.label}`, run: () => usage.setDays(w.days) })),
      { id: 'usage.export-summary', title: 'Export usage summary (JSON)', group: 'Usage', keywords: 'download save json report', run: exportSummary },
      ...(admin ? [{ id: 'usage.view-otto', title: 'Show Otto performance and profiling', group: 'Usage', keywords: 'telemetry cpu ram slow traces', run: () => void selectView('otto') }, { id: 'usage.settings', title: 'Open usage storage and retention', group: 'Usage', keywords: 'clickhouse retention interval settings', run: () => { if (!configOpen) toggleSettings(); } }] : []),
    ]),
  );

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
    exported('daily tokens as CSV', `otto-usage-daily-${usage.days}d.csv`);
  }
  function exportSessions(): void {
    usage.exportSessionsCsv();
    exported('sessions as CSV', `otto-usage-sessions-${usage.days}d.csv`);
  }
  function exportModels(): void {
    usage.exportModelsCsv();
    exported('models as CSV', `otto-usage-models-${usage.days}d.csv`);
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
    usage.admin = admin;
    if (canView) void usage.loadAll();
    // Live metric ticks refetch the sparkline only while this page is mounted.
    const unwatch = usage.watchMetrics();
    return () => {
      unwatch();
      // Tear down auto-refresh on unmount so we don't poll in the background.
      usage.setAutoRefresh(false);
    };
  });

  // Keep the store's root flag in step (it decides whether /usage/metrics,
  // a root-only endpoint, is requested at all).
  $effect(() => {
    usage.admin = admin;
  });

  // Mirror server status into the editable fields whenever it refreshes,
  // unless the user is actively editing Storage & retention.
  $effect(() => {
    const s = usage.status;
    if (s && !storageDirty) {
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

  // Spend couldn't be read (error-handling-5-07): rows carry 0, not a reading.
  const spendUnavailable = $derived(usage.budgets?.spend_unavailable === true);
  // a11y-6-01: announced at the edge — the warning line is inserted with its text.
  $effect(() => { if (spendUnavailable) announce('Spend unavailable: the usage engine couldn’t report spend for this window.'); });

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
    if (usage.savingBudgets || budgetValidation) return;
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
  /** "1 session" / "3,412 sessions" — lib/plural with a locale-formatted count. */
  function plural(n: number, one: string, many = one + 's'): string {
    return `${formatCount(n)} ${pluralNoun(n, one, many)}`;
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

  // ── Daily tokens SVG chart ────────────────────────────────────────────────
  // The chart is hand-rolled SVG (no dependencies). It renders TOKENS on the
  // y-axis (labeled ticks; cost is in each bar's tooltip), days on the x-axis
  // (thinned for 30d/90d), gridlines at each y-tick, and a per-point tooltip
  // via <title>. Works at 7d/30d/90d windows.

  // Chart viewport (inner drawing area, inside the axis labels).
  let dailyChartWidth = $state(500);
  const SVG_W = $derived(dailyChartWidth || 500);
  let selectedDailyKey = $state('');
  const SVG_H = 110;
  const AXIS_L = 52;  // left margin for y-axis labels
  const AXIS_B = 22;  // bottom margin for x-axis labels

  const dailyTokens = $derived((usage.summary?.daily ?? []).map((d) => d.total_tokens));
  const dailyMaxTokens = $derived(Math.max(...dailyTokens, 0));
  const dailyDays = $derived(usage.summary?.daily ?? []);
  const selectedDaily = $derived(dailyDays.find(d => d.day === selectedDailyKey) ?? dailyDays.at(-1));
  const hasUsage = $derived(Boolean(usage.summary && (usage.summary.total_tokens > 0 || usage.summary.total_events > 0 || usage.summary.sessions.length > 0)));

  // Y-axis: 4 ticks from 0 to ceiling. Round the top tick to a "nice" value
  // (never below one token, so a near-empty window can't repeat a label).
  const yTicks = $derived.by(() => {
    const top = dailyMaxTokens;
    const raw = Math.max(top / 3, 1);
    // Pick a magnitude step that gives readable labels.
    const mag = Math.pow(10, Math.floor(Math.log10(raw || 1)));
    const nice = Math.ceil(raw / mag) * mag;
    return [0, nice, nice * 2, nice * 3];
  });

  function svgY(tokens: number): number {
    const plotH = SVG_H - AXIS_B;
    const max = yTicks[yTicks.length - 1] || 1;
    return plotH - (tokens / max) * plotH;
  }
  // Bars sit at the CENTRE of equal bands: pinning the first/last bar to the
  // plot edges put half of the first bar over the y-axis labels and half of
  // the last one (and its date) past the right edge.
  function svgX(i: number, total: number): number {
    const plotW = SVG_W - AXIS_L;
    return AXIS_L + ((i + 0.5) / Math.max(1, total)) * plotW;
  }

  // The viewBox uses measured CSS pixels; keep date labels at least 52px apart.
  function showXLabel(i: number, total: number): boolean {
    const step = Math.max(1, Math.ceil(52 * total / Math.max(1, SVG_W - AXIS_L)));
    return i % step === 0;
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
;
async () => {

 { svelteHTML.createElement("div", { "class":`usage`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":`Usage`,"subtitle":admin ? 'Tokens by provider, model and session, with estimated cost and system load' : 'Tokens used by your sessions, with estimated cost',tabs:() => { async ()/*Ωignore_positionΩ*/ => {
      if(usage.status?.available || admin){
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`tablist`,"aria-label":`Usage view`,});
           { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":view === 'overview',"tabindex":view === 'overview' ? 0 : -1,"onclick":() => void selectView('overview'),"onkeydown":onTabKey,});view === 'overview';  }
           { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":view === 'report',"tabindex":view === 'report' ? 0 : -1,"onclick":() => void selectView('report'),"onkeydown":onTabKey,...__sveltets_2_empty({"data-testid":`usage-view-report`}),});view === 'report';  }
          if(admin){ { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":view === 'otto',"tabindex":view === 'otto' ? 0 : -1,"onclick":() => void selectView('otto'),"onkeydown":onTabKey,});view === 'otto';  }}
         }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(usage.status?.available && view !== 'otto'){
         { svelteHTML.createElement("button", {               "class":`icon-btn`,"onclick":() => usage.loadAll(),"disabled":usage.loading,"title":usage.loading ? 'Refreshing…' : 'Refresh',"aria-label":`Refresh`,...__sveltets_2_empty({"data-icon":`refresh`}),...__sveltets_2_empty({"data-label":`Refresh`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
         }
         { svelteHTML.createElement("button", {               "class":`btn small`,"disabled":!usage.summary,"onclick":exportSummary,"title":usage.summary ? 'Download the full summary as JSON' : 'Nothing to export yet',...__sveltets_2_empty({"data-icon":`download`}),...__sveltets_2_empty({"data-label":`Export summary as JSON`}),...__sveltets_2_empty({"data-overflow":`-1`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
        if(admin){
         { svelteHTML.createElement("button", {                "class":`icon-btn`,"aria-pressed":configOpen,"onclick":toggleSettings,"title":`Storage and retention settings`,"aria-label":`Storage and retention settings`,...__sveltets_2_empty({"data-icon":`gear`}),...__sveltets_2_empty({"data-label":`Storage and retention`}),});configOpen;
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":14,}});}
         }
        }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
    if(view === 'otto' && admin){
      
      ottoLoad; {
            {   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {  "what":`Otto usage`,"loading":true,"empty":true,}});}try { const $$_value = await (ottoUsage());{ const module = $$_value;  { const $$_tluafed_eludom2C = __sveltets_2_ensureComponent(module.default); const $$_tluafed_eludom2 = new $$_tluafed_eludom2C({ target: __sveltets_2_any(), props: {   dirty:telemetryDirty,}});/*Ωignore_startΩ*/() => telemetryDirty = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_tluafed_eludom2.$$bindings = 'dirty';}}} catch($$_e) { 
           { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`Otto usage`,"error":`Otto may have been updated in the background. Retry, or reload Otto to pick up the new version.`,"empty":true,"onretry":() => ottoLoad++,errorActions:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => location.reload(),});  } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => void selectView('overview'),});   }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
            
           LoadState}
        }}
      }
    }else{
    
    if(usage.status?.available){
       { svelteHTML.createElement("div", {     "class":`usage-bar`,"role":`toolbar`,"aria-label":`Usage scope`,});
        if(admin){
           { svelteHTML.createElement("div", {         "class":`segmented`,"role":`group`,"aria-label":`Sessions to count`,"title":`Otto: only sessions run inside Otto. All: every Claude/Codex session on this Mac.`,});
             { svelteHTML.createElement("button", {    "aria-pressed":usage.ottoOnly,"onclick":() => usage.setOttoOnly(true),});usage.ottoOnly;  }
             { svelteHTML.createElement("button", {    "aria-pressed":!usage.ottoOnly,"onclick":() => usage.setOttoOnly(false),});!usage.ottoOnly;  }
           }
        }
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Time window`,});
             for(let w of __sveltets_2_ensureArray(WINDOWS)){w.days;
             { svelteHTML.createElement("button", {        "aria-pressed":usage.days === w.days,"title":`Last ${w.days} days`,"onclick":() => usage.setDays(w.days),});usage.days === w.days;
              w.label;
             }
          }
         }
         { svelteHTML.createElement("button", {          "class":`btn small`,"aria-pressed":usage.autoRefresh,"onclick":() => usage.setAutoRefresh(!usage.autoRefresh),"title":usage.autoRefresh ? 'Refreshing every 60 s — click to stop' : 'Refresh this page every 60 s',});usage.autoRefresh;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}
          usage.autoRefresh ? 'Live' : 'Auto-refresh';
         }
       }
    }

    
    if(usage.status?.tailer && usage.status.tailer.state !== 'running'){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Usage isn’t being recorded: ${usage.status.tailer.cause ?? 'the usage journal is unavailable'}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`budget-banner tailer-banner`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
         { svelteHTML.createElement("span", {});
             usage.status.tailer.state === 'retrying' ? ' — Otto is retrying' : ''; usage.status.tailer.cause ?? 'the usage journal is unavailable';usage.status.tailer.state === 'stopped' ? '. Restart Otto after fixing it.' : '.';
         }
       }}
    }

    
    if(budgetAlert){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(budgetAlert.direction === 'recovered' ? `Back under budget: ${budgetAlert.provider || 'workspace'}` : { text: `Over budget: ${budgetAlert.provider || 'workspace'} spent ${formatUsd(budgetAlert.spendUsd)} of its ${formatUsd(budgetAlert.capUsd)} cap.`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {    "class":`budget-banner`,});budgetAlert.direction === 'recovered';
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":budgetAlert.direction === 'recovered' ? 'check' : 'warning',"size":14,}});}
        if(budgetAlert.direction === 'recovered'){
           { svelteHTML.createElement("span", {});
                { svelteHTML.createElement("strong", {});budgetAlert.provider || 'workspace'; }
             formatUsd(budgetAlert.spendUsd);    formatUsd(budgetAlert.capUsd); 
           }
        }else{
           { svelteHTML.createElement("span", {});
               { svelteHTML.createElement("strong", {});budgetAlert.provider || 'workspace'; }
             formatUsd(budgetAlert.spendUsd);   formatUsd(budgetAlert.capUsd); 
           }
        }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":dismissBudgetAlert,"title":`Dismiss`,"aria-label":`Dismiss budget alert`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
       }}
    }

    if(!canView){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`chart`,"title":`You don’t have access to usage`,"body":`Token and cost history is visible to people granted Usage. Ask whoever set up this Otto for access.`,}});}
    } else if (!usage.status){
      
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {              "what":`usage`,"variant":`page`,"rows":6,"loading":usage.loading || !usage.statusError,"error":usage.statusError,"empty":true,"onretry":() => void usage.loadAll(),}});}
    } else if (!usage.status.available && !admin){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`chart`,"title":`Usage tracking isn’t set up`,"body":`Otto records token usage once the root account sets up usage tracking.`,}});}
    } else if (!usage.status.available){
      
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`chart`,"title":`Set up usage tracking`,"body":`Otto keeps token, cost and system-load history in an embedded ClickHouse engine (clickhouse local — no server, no port). Install it once and Otto manages it from then on.`,"actionLabel":usage.installing ? 'Installing…' : 'Install ClickHouse',"actionIcon":`download`,"onaction":() => { if (!usage.installing) void usage.install(); },}});
         { svelteHTML.createElement("div", { "class":`install-alt`,});
           { svelteHTML.createElement("label", { "for":`ch-path`,});        }
           { svelteHTML.createElement("div", { "class":`path-input`,});
             { const $$_dleiFhtaP5C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP5 = new $$_dleiFhtaP5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },value:chPath,"files":true,}});/*Ωignore_startΩ*/() => chPath = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP5.$$bindings = 'value'; { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`ch-path`,"class":`input mono`,"placeholder":`/usr/local/bin/clickhouse`,"bind:value":chPath,"spellcheck":`false`,});/*Ωignore_startΩ*/() => chPath = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
             { svelteHTML.createElement("button", {         "class":`btn`,"disabled":usage.saving || chPath.trim() === '',"title":chPath.trim() === '' ? 'Enter the path to a clickhouse binary first' : 'Use this binary',"onclick":() => usage.saveConfig({ enabled: true, clickhouse_path: chPath.trim() }),});
              usage.saving ? 'Saving…' : 'Use binary';
             }
           }
          if(usage.status?.binary){
             { svelteHTML.createElement("span", { "class":`dim small`,});  { svelteHTML.createElement("span", { "class":`mono`,});usage.status.binary; } }
          }
          if(usage.status?.priced_as_of){
             { svelteHTML.createElement("span", { "class":`dim small`,});
                     usage.status.priced_as_of;    
                    
             }
          }
         }
       EmptyState}
    }else{
       { svelteHTML.createElement("div", { "class":`body`,});
        if(usage.summary?.scope === 'own'){
           { svelteHTML.createElement("p", {   "class":`scope-note`,"role":`note`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}     }
        }
        
        if(configOpen && admin){
           { const $$_section3 = svelteHTML.createElement("section", {    "class":`panel card`,"aria-labelledby":`usage-cfg-title`,});settingsEl = $$_section3;
             { svelteHTML.createElement("div", { "class":`panel-head`,});
               { svelteHTML.createElement("h3", { "id":`usage-cfg-title`,});   }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (configOpen = false),"title":`Close`,"aria-label":`Close storage and retention`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
               }
             }
             { svelteHTML.createElement("div", { "class":`cfg-grid`,});
               { svelteHTML.createElement("label", { "for":`cfg-retention`,});    }
               { svelteHTML.createElement("input", {            "id":`cfg-retention`,"class":`input`,"type":`number`,"min":`1`,"max":`3650`,"bind:value":retention,});/*Ωignore_startΩ*/() => retention = __sveltets_2_any(null);/*Ωignore_endΩ*/}

               { svelteHTML.createElement("label", { "for":`cfg-interval`,});     }
               { svelteHTML.createElement("input", {            "id":`cfg-interval`,"class":`input`,"type":`number`,"min":`5`,"max":`3600`,"bind:value":interval,});/*Ωignore_startΩ*/() => interval = __sveltets_2_any(null);/*Ωignore_endΩ*/}

               { svelteHTML.createElement("label", { "for":`cfg-path`,});  }
               { const $$_dleiFhtaP5C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP5 = new $$_dleiFhtaP5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },value:chPath,"files":true,}});/*Ωignore_startΩ*/() => chPath = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP5.$$bindings = 'value'; { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`cfg-path`,"class":`input mono`,"bind:value":chPath,"spellcheck":`false`,});/*Ωignore_startΩ*/() => chPath = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
             }
            if(storageError){ { svelteHTML.createElement("p", {   "class":`budget-error`,"role":`alert`,});storageError; }}
             { svelteHTML.createElement("div", { "class":`cfg-actions`,});
               { svelteHTML.createElement("button", {     "class":`btn`,"disabled":usage.installing,"onclick":() => usage.install(),});
                usage.installing ? 'Updating…' : 'Update ClickHouse';
               }
               { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":usage.saving || !!storageError,"title":storageError || undefined,"onclick":() =>
                  usage.saveConfig({
                    enabled: true,
                    retention_days: retention,
                    metrics_interval_secs: interval,
                    clickhouse_path: chPath.trim(),
                  }),});
                usage.saving ? 'Saving…' : 'Save settings';
               }
             }
             { svelteHTML.createElement("div", { "class":`engine-meta`,});
               { svelteHTML.createElement("span", {});  { svelteHTML.createElement("span", { "class":`mono`,});usage.status.version ?? '—'; } }
               { svelteHTML.createElement("span", {   "class":`ellip-any`,"title":usage.status.data_dir,});  { svelteHTML.createElement("span", { "class":`mono`,});usage.status.data_dir; } }
               { svelteHTML.createElement("span", { "title":`${usage.status.usage_rows.toLocaleString()} usage rows · ${usage.status.metric_rows.toLocaleString()} metric rows`,});
                  formatBytes(usage.status.disk_bytes);
                plural(usage.status.usage_rows, 'row');
               }
               { svelteHTML.createElement("span", {});  usage.status.retention_days;  }
              if(usage.status.priced_as_of){
                 { svelteHTML.createElement("span", { "title":`Cost estimates use published rates as of this date. Unknown models fall back to the Opus tier.`,});
                     usage.status.priced_as_of;
                 }
              }
             }
           }
        }

        if(view === 'report'){
          reportLoad; {
                { 
                { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {  "what":`Usage report`,"loading":true,"empty":true,}});}
            try { const $$_value = await (import('./UsageReportView.svelte'));{ const module = $$_value; 
               { const $$_tluafed_eludom3C = __sveltets_2_ensureComponent(module.default); new $$_tluafed_eludom3C({ target: __sveltets_2_any(), props: { admin,}});}
            }} catch($$_e) { const error = __sveltets_2_any();
               { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`Usage report`,"error":loadErrorText(error),"empty":true,"onretry":() => reportLoad++,}});}
            }}
          }
        }else{
        if(usage.summaryError){
          
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {           "what":`usage summary`,"loading":usage.loading,"error":usage.summaryError,"empty":!usage.summary,"onretry":() => void usage.loadAll(),}});}
        } else if (!usage.summary && usage.loading){
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {    "what":`usage summary`,"loading":true,"empty":true,"rows":4,}});}
        }

        if(usage.summary && !hasUsage){
           { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`chart`,"title":`No usage in this window`,"body":`Choose a longer window or All sessions to include activity outside Otto. New agent activity will appear here.`,}});}
        }
        
        if(usage.summary && hasUsage){
           { svelteHTML.createElement("div", { "class":`cards`,});
             { svelteHTML.createElement("div", { "class":`stat card`,});
               { svelteHTML.createElement("span", { "class":`stat-label`,});  }
               { svelteHTML.createElement("span", {   "class":`stat-value`,"title":usage.summary.total_tokens.toLocaleString(),});fmtNum(usage.summary.total_tokens); }
               { svelteHTML.createElement("div", {   "class":`seg-bar`,"title":breakdownTitle(summaryParts(usage.summary)),});
                   for(let s of __sveltets_2_ensureArray(tokenSegs(summaryParts(usage.summary)))){s.label;
                  if(s.pct > 0){ { svelteHTML.createElement("div", { "style":`width: ${s.pct}%; background: ${s.color}`,}); }}
                }
               }
               { svelteHTML.createElement("span", {   "class":`stat-sub`,"title":`Estimated at published list prices`,});
                 formatUsd(usage.summary.total_cost_usd);    usage.summary.days; 
                
                if(usage.summary.providers.length > 0){
                   { const $$_pihCtsaceroFtsoC6C = __sveltets_2_ensureComponent(CostForecastChip); new $$_pihCtsaceroFtsoC6C({ target: __sveltets_2_any(), props: {     "feature":`agent`,"provider":usage.summary.providers[0].provider,}});}
                }
               }
             }
             { svelteHTML.createElement("div", { "class":`stat card`,});
               { svelteHTML.createElement("span", { "class":`stat-label`,});  }
               { svelteHTML.createElement("span", {   "class":`stat-value`,"title":usage.summary.total_output_tokens.toLocaleString(),});fmtNum(usage.summary.total_output_tokens); }
               { svelteHTML.createElement("span", {   "class":`stat-sub`,"title":usage.summary.total_input_tokens.toLocaleString(),});fmtNum(usage.summary.total_input_tokens);   }
             }
             { svelteHTML.createElement("div", { "class":`stat card`,});
               { svelteHTML.createElement("span", { "class":`stat-label`,});  }
               { svelteHTML.createElement("span", {   "class":`stat-value`,"title":usage.summary.total_cache_read_tokens.toLocaleString(),});fmtNum(usage.summary.total_cache_read_tokens); }
               { svelteHTML.createElement("span", {   "class":`stat-sub`,"title":usage.summary.total_cache_write_tokens.toLocaleString(),});fmtNum(usage.summary.total_cache_write_tokens);   }
             }
             { svelteHTML.createElement("div", { "class":`stat card`,});
               { svelteHTML.createElement("span", { "class":`stat-label`,});  }
               { svelteHTML.createElement("span", { "class":`stat-value`,});fmtNum(usage.summary.total_events); }
               { svelteHTML.createElement("span", { "class":`stat-sub`,});
                usage.summary.total_events === 1 ? 'event' : 'events';  plural(usage.summary.providers.length, 'provider'); 
                plural(usage.summary.sessions.length, 'session');
               }
             }
           }
        }

        
        if(usage.summary && usage.summary.total_tokens > 0){
          const parts = summaryParts(usage.summary);
           { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-bd-title`,});
             { svelteHTML.createElement("div", { "class":`panel-head`,});
               { svelteHTML.createElement("h3", { "id":`usage-bd-title`,});  }
               { svelteHTML.createElement("div", { "class":`legend`,});
                   for(let s of __sveltets_2_ensureArray(tokenSegs(parts))){s.label;
                   { svelteHTML.createElement("span", { "class":`lg`,}); { svelteHTML.createElement("i", { "style":`background: ${s.color}`,}); }s.label; }
                }
               }
             }
             { svelteHTML.createElement("div", {   "class":`seg-bar big`,"title":breakdownTitle(parts),});
                 for(let s of __sveltets_2_ensureArray(tokenSegs(parts))){s.label;
                if(s.pct > 0){ { svelteHTML.createElement("div", { "style":`width: ${s.pct}%; background: ${s.color}`,}); }}
              }
             }
             { svelteHTML.createElement("div", { "class":`bd-list`,});
                 for(let s of __sveltets_2_ensureArray(tokenSegs(parts))){s.label;
                 { svelteHTML.createElement("div", { "class":`bd-item`,});
                   { svelteHTML.createElement("i", {   "class":`swatch`,"style":`background: ${s.color}`,}); }
                   { svelteHTML.createElement("span", { "class":`bd-label`,});s.label; }
                   { svelteHTML.createElement("span", {   "class":`bd-val`,"title":s.v.toLocaleString(),});fmtNum(s.v); }
                   { svelteHTML.createElement("span", { "class":`bd-pct dim`,});s.pct.toFixed(0);  }
                 }
              }
             }
           }
        }

        if(usage.summary && hasUsage){
           { svelteHTML.createElement("div", { "class":`grid`,});
            
             { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-prov-title`,});
               { svelteHTML.createElement("div", { "class":`panel-head`,});
                 { svelteHTML.createElement("h3", { "id":`usage-prov-title`,});  }
                if(usage.summary.providers.length > 0){
                   { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":exportProviders,"title":`Download providers as CSV`,"aria-label":`Download providers as CSV`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
                   }
                }
               }
              if(usage.summary.providers.length > 0){
                const pmax = Math.max(1, ...usage.summary.providers.map((p) => p.total_tokens));
                 { svelteHTML.createElement("div", { "class":`bars`,});
                     for(let p of __sveltets_2_ensureArray(usage.summary.providers)){p.provider;
                     { svelteHTML.createElement("div", { "class":`bar-row`,});
                       { svelteHTML.createElement("span", {   "class":`bar-name`,"title":p.provider,});p.provider; }
                       { svelteHTML.createElement("div", { "class":`bar-track`,});
                         { svelteHTML.createElement("div", {     "class":`bar-fill stacked`,"style":`width: ${(p.total_tokens / pmax) * 100}%`,"title":breakdownTitle(p),});
                             for(let s of __sveltets_2_ensureArray(tokenSegs(p))){s.label;
                            if(s.pct > 0){ { svelteHTML.createElement("div", { "style":`width: ${s.pct}%; background: ${s.color}`,}); }}
                          }
                         }
                       }
                       { svelteHTML.createElement("span", {   "class":`bar-val`,"title":`${p.total_tokens.toLocaleString()} tokens`,});fmtNum(p.total_tokens); }
                       { svelteHTML.createElement("span", { "class":`bar-cost dim`,});formatUsd(p.cost_usd); }
                     }
                  }
                 }
              }else{
                 { svelteHTML.createElement("p", { "class":`dim small`,});           }
              }
             }

            
             { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-daily-title`,});
               { svelteHTML.createElement("div", { "class":`panel-head`,});
                 { svelteHTML.createElement("h3", { "id":`usage-daily-title`,});  }
                if(usage.summary.daily.length > 0){
                   { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":exportDaily,"title":`Download daily tokens as CSV`,"aria-label":`Download daily tokens as CSV`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
                   }
                }
               }
              if(usage.summary.daily.length > 0 && dailyMaxTokens === 0){
                 { svelteHTML.createElement("p", {   "class":`dim small`,...__sveltets_2_empty({"data-testid":`daily-cost-empty`}),});      }
              } else if (usage.summary.daily.length > 0){
                const days = dailyDays;
                const n = days.length;
                 { const $$_div5 = svelteHTML.createElement("div", {  "class":`usage-chart`,});dailyChartWidth= $$_div5.clientWidth;
                 { svelteHTML.createElement("svg", {         "class":`daily-svg`,"viewBox":`0 0 ${SVG_W} ${SVG_H}`,"aria-label":`Daily tokens over the last ${usage.summary.days} days, peak ${fmtNum(dailyMaxTokens)}`,"role":`group`,});
                  
                     for(let tick of __sveltets_2_ensureArray(yTicks)){tick;
                    const y = svgY(tick);
                     { svelteHTML.createElement("line", {          "class":`grid-line`,"x1":AXIS_L,"y1":y,"x2":SVG_W,"y2":y,});}
                     { svelteHTML.createElement("text", {       "class":`axis-label y-label`,"x":AXIS_L - 4,"y":y + 4,"text-anchor":`end`,});
                      fmtNum(tick);
                     }
                  }

                  
                      for(let d of __sveltets_2_ensureArray(days)){let i = 1;d.day;
                    const x = svgX(i, n);
                    const barW = Math.max(2, Math.min(40, ((SVG_W - AXIS_L) / Math.max(1, n)) * 0.7));
                    const barH = (d.total_tokens / (yTicks[yTicks.length - 1] || 1)) * (SVG_H - AXIS_B);
                    const barY = SVG_H - AXIS_B - barH;
                    const segs = tokenSegs(d);
                    
                        for(let s of __sveltets_2_ensureArray(segs)){let si = 1;s.label;
                      if(s.pct > 0){
                        const segH = (s.v / Math.max(1, d.total_tokens)) * barH;
                        const segOffset = segs.slice(0, si).reduce((acc, prev) => acc + (prev.v / Math.max(1, d.total_tokens)) * barH, 0);
                         { svelteHTML.createElement("rect", {             "x":x - barW / 2,"y":barY + segOffset,"width":barW,"height":segH,"fill":s.color,"rx":`1`,});}
                      }
                    }
                    
                     { svelteHTML.createElement("rect", {                         "class":`bar-hit`,"role":`button`,"tabindex":selectedDaily?.day === d.day ? 0 : -1,"aria-label":`${d.day}: ${d.total_tokens.toLocaleString()} tokens, ${formatUsd(d.cost_usd)}`,"aria-pressed":selectedDaily?.day === d.day,"onclick":() => (selectedDailyKey = d.day),"onfocus":() => (selectedDailyKey = d.day),"onkeydown":(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectedDailyKey = d.day; } },"x":x - barW / 2,"y":0,"width":barW,"height":SVG_H - AXIS_B,});
                       { svelteHTML.createElement("title", {});shortDay(d.day);  d.total_tokens.toLocaleString();   formatUsd(d.cost_usd);  breakdownTitle(d); }
                     }

                    
                    if(showXLabel(i, n)){
                       { svelteHTML.createElement("text", {         "class":`axis-label x-label`,"x":x,"y":SVG_H - 4,"text-anchor":`middle`,});shortDay(d.day); }
                    }
                  }

                  
                   { svelteHTML.createElement("line", {          "class":`axis-line`,"x1":AXIS_L,"y1":SVG_H - AXIS_B,"x2":SVG_W,"y2":SVG_H - AXIS_B,});}
                 }
                 }
                if(selectedDaily){
                   { svelteHTML.createElement("div", {   "class":`usage-chart-details`,...__sveltets_2_empty({"data-testid":`usage-daily-details`}),});
                     { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`Daily tokens day`,"value":selectedDaily.day,"onchange":(event) => (selectedDailyKey = event.currentTarget.value),});
                         for(let d of __sveltets_2_ensureArray(days)){d.day; { svelteHTML.createElement("option", { "value":d.day,});d.day; }}
                     } }
                     { svelteHTML.createElement("p", { "role":`status`,});selectedDaily.day;  selectedDaily.total_tokens.toLocaleString();    formatUsd(selectedDaily.cost_usd);  breakdownTitle(selectedDaily); }
                   }
                }
              }else{
                 { svelteHTML.createElement("p", { "class":`dim small`,});      }
              }
             }
           }

          
           { const $$_nwodkaerBledoM3C = __sveltets_2_ensureComponent(ModelBreakdown); new $$_nwodkaerBledoM3C({ target: __sveltets_2_any(), props: {         "models":usage.summary.models ?? [],"dailyModels":usage.summary.daily_models ?? [],"days":usage.summary.days,"onexport":exportModels,}});}

          
           { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-feat-title`,});
             { svelteHTML.createElement("div", { "class":`panel-head`,});
               { svelteHTML.createElement("h3", { "id":`usage-feat-title`,});  }
              if(usage.summary.by_kind.length > 0){
                 { svelteHTML.createElement("span", { "class":`dim small`,});        }
              }
             }
            if(usage.summary.by_kind.length > 0){
              const fmax = Math.max(1, ...usage.summary.by_kind.map((f) => f.total_tokens));
               { svelteHTML.createElement("div", { "class":`bars`,});
                   for(let f of __sveltets_2_ensureArray(usage.summary.by_kind)){f.feature;
                   { svelteHTML.createElement("div", { "class":`bar-row feat-row`,});
                     { svelteHTML.createElement("span", {   "class":`bar-name`,"title":featureLabel(f.feature),});
                       { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {  "label":featureLabel(f.feature),}});}
                     }
                     { svelteHTML.createElement("div", { "class":`bar-track`,});
                       { svelteHTML.createElement("div", {     "class":`bar-fill stacked`,"style":`width: ${(f.total_tokens / fmax) * 100}%`,"title":breakdownTitle(f),});
                           for(let s of __sveltets_2_ensureArray(tokenSegs(f))){s.label;
                          if(s.pct > 0){ { svelteHTML.createElement("div", { "style":`width: ${s.pct}%; background: ${s.color}`,}); }}
                        }
                       }
                     }
                     { svelteHTML.createElement("span", {   "class":`bar-val`,"title":`${f.total_tokens.toLocaleString()} tokens`,});fmtNum(f.total_tokens); }
                     { svelteHTML.createElement("span", { "class":`bar-cost dim`,});formatUsd(f.cost_usd); }
                   }
                }
               }
            }else{
               { svelteHTML.createElement("p", { "class":`dim small`,});                  }
            }
           }

          
           { const $$_nwodllirDnoitubirttA3C = __sveltets_2_ensureComponent(AttributionDrilldown); new $$_nwodllirDnoitubirttA3C({ target: __sveltets_2_any(), props: {  "days":usage.days,}});}
        }

        
         { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-budget-title`,});
           { svelteHTML.createElement("div", { "class":`panel-head`,});
             { svelteHTML.createElement("h3", { "id":`usage-budget-title`,});  }
            if(admin && usage.budgets){
             { svelteHTML.createElement("button", {           "class":`btn small`,"aria-expanded":budgetsOpen,"aria-controls":`usage-budget-editor`,"onclick":() => (budgetsOpen ? discardBudgets() : (budgetsOpen = true)),"title":budgetsOpen && budgetsDirty ? 'Close the editor and discard unsaved changes' : undefined,});
              if(budgetsOpen){budgetsDirty ? 'Discard changes' : 'Close editor';}else{ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}  }
             }
            }
           }
           { svelteHTML.createElement("p", { "class":`dim small intro`,});
                         
           }

          
          if(usage.budgets && usage.budgets.rows.length > 0){
            if(spendUnavailable){ { svelteHTML.createElement("div", { "class":`budget-alert`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}           }}
             { svelteHTML.createElement("div", { "class":`budget-rows`,});
                 for(let r of __sveltets_2_ensureArray(usage.budgets.rows)){r.scope + ':' + r.key;
                const name = r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key);
                 { svelteHTML.createElement("div", {   "class":`budget-row`,});r.warning && !r.exceeded;r.exceeded;
                   { svelteHTML.createElement("span", {   "class":`budget-name`,"title":`${r.scope === 'workspace' ? 'Workspace' : 'Provider'}: ${name}`,});
                     { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {  "label":r.scope === 'workspace' ? 'Workspace' : 'Provider',}});}
                     { svelteHTML.createElement("span", { "class":`ellip-any`,});name; }
                   }
                   { svelteHTML.createElement("div", {             "class":`bar-track`,"role":`meter`,"aria-label":`${name} spend`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":Math.round(Math.min(100, r.used_fraction * 100)),});
                     { svelteHTML.createElement("div", {       "class":`bar-fill`,"style":`width: ${Math.min(100, r.used_fraction * 100)}%`,"title":`${formatUsd(r.spent_usd)} of ${formatUsd(r.limit_usd)}`,}); }
                   }
                   { svelteHTML.createElement("span", { "class":`budget-val`,});
                    spendUnavailable ? 'Spend unavailable' : formatUsd(r.spent_usd);  formatUsd(r.limit_usd);
                    if(spendUnavailable){} else if (r.exceeded){ { svelteHTML.createElement("span", { "class":`over-tag`,});  }
                    } else if (r.warning){ { svelteHTML.createElement("span", { "class":`warn-tag`,});(r.used_fraction * 100).toFixed(0);  }}
                   }
                 }
              }
             }
            if(usage.budgets.config.enforce && usage.budgets.rows.some((r) => r.exceeded)){
               {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: usage.budgets.config.block_on_exceed ? 'A budget is exceeded and enforcement is blocking new work in that scope.' : 'A budget is exceeded; enforcement is warn-only.', assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`budget-alert`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
                usage.budgets.config.block_on_exceed
                  ? 'Enforcement is on and blocking: new work in an over-budget scope can be refused.'
                  : 'Enforcement is on (warn only): over-budget scopes are flagged, not blocked.';
               }}
            }
            
            if(!usage.budgets.config.enforce || !usage.budgets.rows.some((r) => r.exceeded)){
                 for(let r of __sveltets_2_ensureArray(usage.budgets.rows.filter((r) => r.warning && !r.exceeded))){r.scope + ':' + r.key;
                 { svelteHTML.createElement("p", { "class":`dim small heads-up`,});
                  r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key);
                    (r.used_fraction * 100).toFixed(0);   
                  formatUsd(r.spent_usd);  formatUsd(r.limit_usd);
                 }
              }
            }
          } else if (usage.budgetsError){
             { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":`budgets`,"variant":`compact`,"error":usage.budgetsError,"empty":true,"onretry":() => void usage.loadBudgets(),}});}
          } else if (!usage.budgets){
              { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {    "what":`budgets`,"variant":`compact`,"loading":true,"empty":true,}});}
          } else if (!budgetsOpen){
             { svelteHTML.createElement("p", { "class":`dim small`,});admin ? 'No budgets yet. Use Edit budgets to set a cap and track spend against it.' : 'No budgets set.'; }
          }

          
          if(budgetsOpen && admin && usage.budgets){
             { svelteHTML.createElement("div", {   "class":`budget-editor`,"id":`usage-budget-editor`,});
               { svelteHTML.createElement("label", { "class":`checkbox-row`,});
                 { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":budgetCfg.enforce,"onchange":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => budgetCfg.enforce = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 { svelteHTML.createElement("span", {});         }
               }
               { svelteHTML.createElement("label", {    "class":`checkbox-row`,"title":budgetCfg.enforce ? undefined : 'Turn on enforcement first',});!budgetCfg.enforce;
                 { svelteHTML.createElement("input", {         "type":`checkbox`,"bind:checked":budgetCfg.block_on_exceed,"disabled":!budgetCfg.enforce,"onchange":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => budgetCfg.block_on_exceed = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 { svelteHTML.createElement("span", {});              }
               }
               { svelteHTML.createElement("div", { "class":`window-row`,});
                 { svelteHTML.createElement("label", { "for":`budget-window`,});     }
                 { svelteHTML.createElement("input", {                       "id":`budget-window`,"class":`input num-in`,"type":`number`,"min":`1`,"max":`3650`,"step":`1`,"aria-invalid":budgetValidated && budgetWindowInvalid,"aria-describedby":budgetValidated && budgetValidation ? 'budget-validation' : undefined,"onblur":() => (budgetValidated = true),"bind:value":budgetCfg.window_days,"oninput":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => budgetCfg.window_days = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 { svelteHTML.createElement("span", { "class":`dim`,});  }
               }

               { svelteHTML.createElement("div", { "class":`editor-section`,});
                 { svelteHTML.createElement("div", { "class":`editor-head`,});
                   { svelteHTML.createElement("span", { "id":`budget-ws-head`,});  }
                   { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":addWsBudget,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    }
                 }
                    for(let b of __sveltets_2_ensureArray(budgetCfg.workspaces)){let i = 1;i;
                   { svelteHTML.createElement("div", { "class":`editor-line`,});
                     { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`Workspace`,"bind:value":b.workspace_id,"onchange":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => b.workspace_id = __sveltets_2_any(null);/*Ωignore_endΩ*/
                       { svelteHTML.createElement("option", {"value":"",});   }
                         for(let w of __sveltets_2_ensureArray(ws.workspaces)){w.id;
                         { svelteHTML.createElement("option", { "value":w.id,});w.name; }
                      }
                     }
                     { svelteHTML.createElement("span", { "class":`usd`,});  }
                     { svelteHTML.createElement("input", {                       "class":`input num-in`,"type":`number`,"min":`0`,"step":`1`,"placeholder":`0`,"aria-label":`Cap in US dollars`,"bind:value":b.monthly_usd,"aria-invalid":budgetValidated && b.monthly_usd != null && (!Number.isFinite(b.monthly_usd) || b.monthly_usd < 0),"aria-describedby":budgetValidated && budgetValidation ? 'budget-validation' : undefined,"onblur":() => (budgetValidated = true),"oninput":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => b.monthly_usd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                     { svelteHTML.createElement("button", {       "class":`icon-btn rm-btn`,"onclick":() => removeWsBudget(i),"title":`Remove this cap`,"aria-label":`Remove this workspace cap`,});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                     }
                   }
                }
                   { svelteHTML.createElement("p", { "class":`dim small`,});   }
                
               }

               { svelteHTML.createElement("div", { "class":`editor-section`,});
                 { svelteHTML.createElement("div", { "class":`editor-head`,});
                   { svelteHTML.createElement("span", {});  }
                   { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":addProviderBudget,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    }
                 }
                    for(let b of __sveltets_2_ensureArray(budgetCfg.providers)){let i = 1;i;
                   { svelteHTML.createElement("div", { "class":`editor-line`,});
                     { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`Provider`,"bind:value":b.provider,"onchange":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => b.provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
                       { svelteHTML.createElement("option", {"value":"",});   }
                         for(let p of __sveltets_2_ensureArray(providerChoices)){p;
                         { svelteHTML.createElement("option", { "value":p,});p; }
                      }
                     }
                     { svelteHTML.createElement("span", { "class":`usd`,});  }
                     { svelteHTML.createElement("input", {                       "class":`input num-in`,"type":`number`,"min":`0`,"step":`1`,"placeholder":`0`,"aria-label":`Cap in US dollars`,"bind:value":b.monthly_usd,"aria-invalid":budgetValidated && b.monthly_usd != null && (!Number.isFinite(b.monthly_usd) || b.monthly_usd < 0),"aria-describedby":budgetValidated && budgetValidation ? 'budget-validation' : undefined,"onblur":() => (budgetValidated = true),"oninput":() => (budgetsDirty = true),});/*Ωignore_startΩ*/() => b.monthly_usd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                     { svelteHTML.createElement("button", {       "class":`icon-btn rm-btn`,"onclick":() => removeProviderBudget(i),"title":`Remove this cap`,"aria-label":`Remove this provider cap`,});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                     }
                   }
                }
                   { svelteHTML.createElement("p", { "class":`dim small`,});   }
                
               }

              if(budgetValidated && budgetValidation){ { svelteHTML.createElement("p", {   "id":`budget-validation`,"role":`alert`,});budgetValidation; }}
               { svelteHTML.createElement("div", { "class":`editor-actions`,});
                 { svelteHTML.createElement("span", { "class":`dim small`,});             }
                 { svelteHTML.createElement("button", {          "class":`btn`,"disabled":usage.savingBudgets || !budgetsDirty || !!budgetValidation,"title":budgetValidation || (budgetsDirty ? undefined : 'No changes to save'),"onclick":saveBudgets,});!configOpen;
                  usage.savingBudgets ? 'Saving…' : 'Save budgets';
                 }
               }
             }
          }
         }

        if(usage.summary && admin){
          
           { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-sys-title`,});
             { svelteHTML.createElement("div", { "class":`panel-head`,});
               { svelteHTML.createElement("h3", { "id":`usage-sys-title`,});  }
              if(latest){
                 { svelteHTML.createElement("span", { "class":`dim small now`,});
                   latest.cpu_pct.toFixed(0);   latest.mem_pct.toFixed(0);
                  fmtNum(Math.round(latest.mem_used_mb));  fmtNum(Math.round(latest.mem_total_mb));  
                    latest.process_rss_mb.toFixed(0);   plural(latest.active_sessions, 'active session');
                 }
              }
             }
            if(usage.metrics.length > 1){
               { svelteHTML.createElement("div", { "class":`metrics`,});
                 { svelteHTML.createElement("div", { "class":`metric`,});
                   { svelteHTML.createElement("span", { "class":`metric-label`,});  }
                   { svelteHTML.createElement("svg", {         "viewBox":`0 0 300 48`,"preserveAspectRatio":`xMidYMid meet`,"class":`spark`,"role":`img`,"aria-label":`CPU over the last ${usage.metrics.length} samples`,});
                     { svelteHTML.createElement("path", {  "d":sparkPath(cpuSeries, 100, 300, 48),});}
                   }
                 }
                 { svelteHTML.createElement("div", { "class":`metric`,});
                   { svelteHTML.createElement("span", { "class":`metric-label`,});  }
                   { svelteHTML.createElement("svg", {         "viewBox":`0 0 300 48`,"preserveAspectRatio":`xMidYMid meet`,"class":`spark`,"role":`img`,"aria-label":`Memory over the last ${usage.metrics.length} samples`,});
                     { svelteHTML.createElement("path", {  "d":sparkPath(memSeries, 100, 300, 48),});}
                   }
                 }
               }
               { svelteHTML.createElement("span", { "class":`dim small`,}); usage.metrics.length;    usage.status.metrics_interval_secs;  }
            }else{
               { svelteHTML.createElement("p", { "class":`dim small`,});    usage.status.metrics_interval_secs;  }
            }
           }
        }

        if(usage.summary){
          
           { svelteHTML.createElement("section", {   "class":`panel card`,"aria-labelledby":`usage-sess-title`,});
             { svelteHTML.createElement("div", { "class":`panel-head`,});
               { svelteHTML.createElement("h3", { "id":`usage-sess-title`,});  }
              if(usage.summary.sessions.length > 0){
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":exportSessions,"title":`Download sessions as CSV`,"aria-label":`Download sessions as CSV`,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
                 }
              }
             }
            if(usage.summary.sessions.length > 0){
               { svelteHTML.createElement("div", { "class":`sess-scroll`,});
                
                 { svelteHTML.createElement("div", { "class":`sess-head`,});
                   { svelteHTML.createElement("span", {});  }
                   { svelteHTML.createElement("span", {});  }
                   { svelteHTML.createElement("span", {});   }
                   { svelteHTML.createElement("span", { "class":`num`,});  }
                   { svelteHTML.createElement("span", { "class":`num`,});  }
                   { svelteHTML.createElement("span", { "class":`num`,});  }
                   { svelteHTML.createElement("span", {});  }
                 }
                
                 { const $$_tsiLlautriV5C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV5 = new $$_tsiLlautriV5C({ target: __sveltets_2_any(), props: {         "items":usage.summary.sessions,"estimateHeight":46,"class":`sess-vlist`,"findText":(s) =>
                    [
                      s.title ?? (s.kind != null ? s.session_id.slice(0, 12) : 'Session outside Otto'),
                      s.kind ?? '',
                      s.session_id.slice(0, 12),
                      s.workspace_name ?? '—',
                      s.provider,
                      s.model ?? '',
                    ].join('\n'),row:(s) => { async ()/*Ωignore_positionΩ*/ => {
                    const isOttoSession = s.kind != null || s.title != null;
                     { svelteHTML.createElement("div", { "class":`sess-row`,});
                      
                       { svelteHTML.createElement("div", {   "class":`sess-cell`,"title":s.title ? `${s.title}\n${s.session_id}` : s.session_id,});
                         { svelteHTML.createElement("div", { "class":`sess-top`,});
                          if(isOttoSession){
                             { svelteHTML.createElement("button", {     "class":`sess-name sess-open ellip-any`,"onclick":() => openSession(s.session_id),"aria-label":`Open session ${s.title ?? s.session_id}`,});s.title ?? s.session_id.slice(0, 12); }
                          }else{
                             { svelteHTML.createElement("span", { "class":`sess-name ellip-any`,});s.title ?? 'Session outside Otto'; }
                          }
                          if(s.kind){ { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":kindLabel(s.kind),}});}}
                         }
                         { svelteHTML.createElement("div", { "class":`sess-id mono`,});s.session_id.slice(0, 12); }
                       }
                       { svelteHTML.createElement("div", {   "class":`dim ellip-any`,"title":s.workspace_name ?? undefined,});s.workspace_name ?? '—'; }
                       { svelteHTML.createElement("div", { "class":`model-cell`,});
                         { svelteHTML.createElement("span", {   "class":`ellip-any`,"title":s.provider,});s.provider; }
                        if(s.model){
                           { svelteHTML.createElement("span", {   "class":`model-name dim ellip-any`,"title":s.model,});s.model; }
                        }
                       }
                       { svelteHTML.createElement("div", { "class":`num`,});fmtNum(s.events); }
                       { svelteHTML.createElement("div", { "class":`num`,});
                         { svelteHTML.createElement("div", { "class":`sess-tok`,});
                           { svelteHTML.createElement("span", { "title":`${s.total_tokens.toLocaleString()} tokens`,});fmtNum(s.total_tokens); }
                           { svelteHTML.createElement("div", {   "class":`seg-bar mini`,"title":breakdownTitle(s),});
                               for(let seg of __sveltets_2_ensureArray(tokenSegs(s))){seg.label;
                              if(seg.pct > 0){ { svelteHTML.createElement("div", { "style":`width: ${seg.pct}%; background: ${seg.color}`,}); }}
                            }
                           }
                         }
                       }
                       { svelteHTML.createElement("div", {   "class":`num`,"title":s.fallback_priced ? 'Estimated: this model isn’t in the rate table, so it is priced at the Opus tier' : undefined,});
                        formatUsd(s.cost_usd);
                        if(s.fallback_priced){ { svelteHTML.createElement("span", { "class":`est-tag`,}); { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Est.`,}});} }}
                       }
                       { svelteHTML.createElement("div", { "class":`dim`,});fmtLastActive(s.last_active); }
                     }
                  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV5.$$prop_def;/*Ωignore_endΩ*/
                  
                 VirtualList}
               }
            }else{
               { svelteHTML.createElement("p", { "class":`dim small`,});     }
            }
           }
        }
        }
       }
    }
    }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const UsagePage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type UsagePage__SvelteComponent_ = ReturnType<typeof UsagePage__SvelteComponent_>;
/*Ωignore_endΩ*/export default UsagePage__SvelteComponent_;