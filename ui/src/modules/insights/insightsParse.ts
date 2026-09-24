// Pure parsing for Insights reports — no Svelte, no imports, unit-tested in
// ui/unit/insightsParse.test.ts.
//
// A report is a triple on disk (docs/features/insights.md §5): report HTML, a
// ≤10-sentence `summary-*.md` written by the agent, and a metrics JSON written
// by the collector, plus a rolling `index.json` (headline `series` + the
// `action_ledger`). The summary markdown is free-form agent prose, so every
// parser here is BEST-EFFORT: anything it can't recognise comes back empty /
// null and the UI falls back to rendering the markdown as-is. The shape it
// expects (from the bundled `insights` skill):
//
//   # Insights summary — Wed 23 Sep 2026 (daily)
//
//   <headline paragraph: "… 173 Claude sessions …, 268 user turns …, 56 tool errors …">
//   <more short paragraphs>
//
//   ## Action Plan (carried into the ledger)
//
//   1. **Title** — metric: 36, 92% → ≤5, <20% — effort S — act-20260906-01 (regressed)
//   2. **Title** — targets metric: ~5.7 → ≤3 — effort M — new act-20260923-01
//   3. **Title** — metric: 0 → 1 — effort S — carried, improved 9.0pp

export type MetricKey = 'sessions' | 'turns' | 'toolErrors' | 'spend' | 'achievement';

export const METRIC_KEYS: readonly MetricKey[] = ['sessions', 'turns', 'toolErrors', 'spend', 'achievement'];

/** Direction that counts as better for a metric: `up`, `down`, or `neutral`
 *  (more sessions is neither good nor bad by itself). */
export const METRIC_GOOD: Record<MetricKey, 'up' | 'down' | 'neutral'> = {
  sessions: 'neutral',
  turns: 'neutral',
  toolErrors: 'down',
  spend: 'down',
  achievement: 'up',
};

export type Metrics = Partial<Record<MetricKey, number>>;

/** Status vocabulary an action item can carry (ledger + summary tags). */
export type ActionStatus = 'regressed' | 'improved' | 'new' | 'carried' | 'closed' | 'open';

export interface ActionItem {
  /** 1-based position in the plan. */
  index: number;
  title: string;
  /** The targeted metric's name ("lens sessions per small-PR run"), when found. */
  metric: string | null;
  /** Current value text ("~5.7"), when a `current → target` pair was found. */
  current: string | null;
  /** Target value text ("≤3"). */
  target: string | null;
  effort: 'S' | 'M' | 'L' | null;
  /** Ledger ids mentioned (`act-YYYYMMDD-NN`), in order, de-duplicated. */
  ids: string[];
  /** Status words found in the item's tail, in order, de-duplicated. */
  statuses: ActionStatus[];
  /** The whole item as written (markdown), for the fallback view. */
  raw: string;
}

export interface ParsedSummary {
  /** The H1 text ("Insights summary — Wed 23 Sep 2026 (daily)"). */
  title: string | null;
  /** First sentence of the first paragraph — the report's one-line headline. */
  headline: string | null;
  metrics: Metrics;
  actions: ActionItem[];
  /** The markdown minus the H1 and (when actions parsed) the Action Plan
   *  section — what the Preview renders under the key findings. */
  body: string;
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

/** "2,197" → 2197; "12.5" → 12.5; anything else → null. */
export function toNumber(s: string | undefined | null): number | null {
  if (s == null) return null;
  const n = Number(s.replace(/,/g, ''));
  return Number.isFinite(n) ? n : null;
}

function firstMatch(text: string, patterns: RegExp[]): number | null {
  for (const re of patterns) {
    const m = re.exec(text);
    if (m) {
      const n = toNumber(m[1]);
      if (n != null) return n;
    }
  }
  return null;
}

/** Pull the headline metrics out of free-form summary prose. Only the text
 *  BEFORE the Action Plan is read — action items are full of target numbers
 *  ("36 runs → ≤5") that would poison the headline. */
export function extractMetrics(prose: string): Metrics {
  const text = prose.replace(/\*\*|__|`/g, '');
  const out: Metrics = {};
  const sessions = firstMatch(text, [
    /\b(\d[\d,]*)\s+(?:[A-Za-z][\w-]*\s+)?sessions?\b/i,
    /\bsessions?\s*[:=]\s*(\d[\d,]*)/i,
  ]);
  if (sessions != null) out.sessions = sessions;
  const turns = firstMatch(text, [
    /\b(\d[\d,]*)\s+(?:user\s+|human\s+)?(?:turns|messages|prompts)\b/i,
    /\b(?:turns|messages)\s*[:=]\s*(\d[\d,]*)/i,
  ]);
  if (turns != null) out.turns = turns;
  const errors = firstMatch(text, [
    /\b(\d[\d,]*)\s+tool[- ]errors?\b/i,
    /\btool[- ]errors?\s*(?:[:=]|total(?:led)?|of)?\s*(\d[\d,]*)/i,
    /\b(\d[\d,]*)\s+(?:tool\s+)?(?:call\s+)?failures?\b/i,
  ]);
  if (errors != null) out.toolErrors = errors;
  const spend = extractSpend(text);
  if (spend != null) out.spend = spend;
  const ach = firstMatch(text, [
    /achievement(?:\s+rate)?[^0-9%\n]{0,24}?(\d{1,3}(?:\.\d+)?)\s*%/i,
    /(\d{1,3}(?:\.\d+)?)\s*%\s+(?:achievement|achieved|fully(?: or mostly)? achieved)/i,
  ]);
  if (ach != null && ach <= 100) out.achievement = ach;
  return out;
}

/** A dollar amount only counts as spend when a spend word sits next to it —
 *  "$4.32 spend", "cost $12", "spent $3.10". A bare "$2/day" target doesn't. */
function extractSpend(text: string): number | null {
  const re = /\$\s?(\d[\d,]*(?:\.\d+)?)/g;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    const before = text.slice(Math.max(0, m.index - 28), m.index).toLowerCase();
    const after = text.slice(m.index + m[0].length, m.index + m[0].length + 28).toLowerCase();
    if (/^\s*\//.test(after)) continue; // "$2/day" is a rate, not a total
    if (/(spend|spent|cost|costing|bill)\b[^.$]*$/.test(before) || /^[^.$]*?\b(spend|spent|in cost|of cost|total cost|cost)\b/.test(after)) {
      const n = toNumber(m[1]);
      if (n != null) return n;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Markdown structure
// ---------------------------------------------------------------------------

const ACTION_HEADING = /^#{1,4}\s+.*\baction\s*plan\b/i;
const HEADING = /^#{1,6}\s+/;

/** Split the markdown into (before Action Plan, the Action Plan lines, after). */
function splitActionPlan(md: string): { before: string[]; plan: string[] | null; after: string[] } {
  const lines = md.replace(/\r\n?/g, '\n').split('\n');
  const start = lines.findIndex((l) => ACTION_HEADING.test(l.trim()));
  if (start < 0) return { before: lines, plan: null, after: [] };
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (HEADING.test(lines[i].trim())) {
      end = i;
      break;
    }
  }
  return { before: lines.slice(0, start), plan: lines.slice(start + 1, end), after: lines.slice(end) };
}

/** First sentence of a paragraph, tolerant of decimals ("9.0pp"), "e.g.",
 *  and "vs." — splits only on `. ` / `! ` / `? ` followed by a capital/digit. */
export function firstSentence(p: string): string {
  const s = p.trim();
  const re = /([.!?])\s+(?=[A-Z0-9"“(])/g;
  for (let m = re.exec(s); m; m = re.exec(s)) {
    const head = s.slice(0, m.index);
    if (/\b(?:e\.g|i\.e|vs|approx|incl|etc|No|St|Mr|Dr)$/i.test(head)) continue;
    return s.slice(0, m.index + 1);
  }
  return s;
}

/** Plain text of an inline-markdown string (for headlines and titles). */
export function plainInline(s: string): string {
  return s
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/\*\*([^*]+)\*\*/g, '$1')
    .replace(/__([^_]+)__/g, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, '$1$2')
    .replace(/\s+/g, ' ')
    .trim();
}

const STATUS_WORDS: Record<string, ActionStatus> = {
  regressed: 'regressed',
  regression: 'regressed',
  worse: 'regressed',
  improved: 'improved',
  improving: 'improved',
  new: 'new',
  carried: 'carried',
  'carried over': 'carried',
  closed: 'closed',
  done: 'closed',
  open: 'open',
  'still open': 'open',
};

/** Parse one Action Plan item's text (after the `1. ` marker). */
export function parseActionItem(text: string, index: number): ActionItem {
  const raw = text.trim();
  let title = '';
  let rest = raw;
  const bold = /^\*\*(.+?)\*\*\s*[:.]?/.exec(raw);
  if (bold) {
    title = plainInline(bold[1]);
    rest = raw.slice(bold[0].length);
  } else {
    const cut = raw.search(/\s[—–]\s|\s--\s/);
    title = plainInline(cut > 0 ? raw.slice(0, cut) : raw);
    rest = cut > 0 ? raw.slice(cut) : '';
  }
  const segments = rest
    .split(/\s+[—–]\s+|\s+--\s+|^\s*[—–]\s*/)
    .map((s) => s.trim())
    .filter(Boolean);

  let effort: ActionItem['effort'] = null;
  const effortMatch = /\beffort\s*[:=]?\s*\(?\s*(S|M|L)\b/i.exec(rest);
  if (effortMatch) effort = effortMatch[1].toUpperCase() as 'S' | 'M' | 'L';

  let metric: string | null = null;
  let current: string | null = null;
  let target: string | null = null;
  const arrowSeg = segments.find((s) => /→|->/.test(s) && !/^effort\b/i.test(s));
  if (arrowSeg) {
    const seg = plainInline(arrowSeg).replace(/^targets?\s+/i, '');
    const colon = seg.search(/:\s/);
    const values = colon > 0 ? seg.slice(colon + 1).trim() : seg;
    if (colon > 0) metric = seg.slice(0, colon).trim();
    const [cur, ...tgt] = values.split(/\s*(?:→|->)\s*/);
    current = cur?.trim() || null;
    target = tgt.join(' → ').trim() || null;
  }

  const ids = [...new Set(raw.match(/\bact-\d{8}-\d{1,3}\b/g) ?? [])];

  // Status words live in the tail — after the effort segment when present,
  // else in whatever segment follows the metric.
  const effortIdx = segments.findIndex((s) => /^effort\b/i.test(s));
  const tailSegs = effortIdx >= 0 ? segments.slice(effortIdx + 1) : segments.filter((s) => s !== arrowSeg);
  const tail = tailSegs.join(' ').toLowerCase();
  const statuses: ActionStatus[] = [];
  const statusRe = /\b(still open|carried over|regressed|regression|improved|improving|carried|closed|new|open|done)\b/g;
  for (let m = statusRe.exec(tail); m; m = statusRe.exec(tail)) {
    const s = STATUS_WORDS[m[1]];
    if (s && !statuses.includes(s)) statuses.push(s);
  }

  return { index, title, metric, current, target, effort, ids, statuses, raw };
}

/** Parse the Action Plan list lines into items. Continuation lines (indented
 *  or wrapped) join the item above them. */
export function parseActionPlan(planLines: string[]): ActionItem[] {
  const items: string[] = [];
  for (const line of planLines) {
    const m = /^\s*(?:\d+[.)]|[-*])\s+(.*)$/.exec(line);
    if (m && !/^\s{4,}/.test(line)) items.push(m[1]);
    else if (line.trim() && items.length > 0) items[items.length - 1] += ' ' + line.trim();
  }
  return items.map((t, i) => parseActionItem(t, i + 1)).filter((a) => a.title.length > 0);
}

/** Remove `lead` (the first sentence) from the first prose line that starts
 *  with it; the rest of that line stays. Untouched when it isn't found. */
function dropLead(lines: string[], lead: string): string[] {
  const i = lines.findIndex((l) => l.trim() !== '' && !HEADING.test(l.trim()));
  if (i < 0) return lines;
  const t = lines[i].trim();
  if (!t.startsWith(lead)) return lines;
  const rest = t.slice(lead.length).trim();
  const out = [...lines];
  if (rest) out[i] = rest;
  else out.splice(i, 1);
  return out;
}

/** Parse one report's summary markdown. Never throws. */
export function parseSummary(md: string | null | undefined): ParsedSummary {
  const src = (md ?? '').replace(/\r\n?/g, '\n');
  const empty: ParsedSummary = { title: null, headline: null, metrics: {}, actions: [], body: src };
  if (!src.trim()) return empty;
  try {
    const { before, plan, after } = splitActionPlan(src);
    let title: string | null = null;
    const beforeBody: string[] = [];
    for (const l of before) {
      const h1 = /^#\s+(.*)$/.exec(l.trim());
      if (h1 && title == null) {
        title = plainInline(h1[1]);
        continue;
      }
      beforeBody.push(l);
    }
    // First paragraph = first run of non-blank, non-heading, non-list lines.
    const paras: string[] = [];
    let cur: string[] = [];
    for (const l of beforeBody) {
      const t = l.trim();
      if (!t || HEADING.test(t) || /^[-*]\s|^\d+[.)]\s|^>/.test(t)) {
        if (cur.length) paras.push(cur.join(' '));
        cur = [];
        continue;
      }
      cur.push(t);
    }
    if (cur.length) paras.push(cur.join(' '));
    const lead = paras.length ? firstSentence(paras[0]) : null;
    const headline = lead ? plainInline(lead) : null;
    const metrics = extractMetrics(beforeBody.join('\n'));
    const actions = plan ? parseActionPlan(plan) : [];
    // The headline is shown above the report; drop it from the body so the
    // Summary doesn't open by repeating it. Only when the first paragraph
    // really starts with it (it's a one-line paragraph in the skill's shape).
    const prose = lead ? dropLead(beforeBody, lead) : beforeBody;
    const bodyLines = actions.length > 0 ? [...prose, ...after] : [...prose, ...(plan ? src.split('\n').slice(before.length, before.length + 1 + plan.length) : []), ...after];
    return { title, headline, metrics, actions, body: bodyLines.join('\n').trim() };
  } catch {
    return empty;
  }
}

// ---------------------------------------------------------------------------
// index.json: series + ledger
// ---------------------------------------------------------------------------

export interface SeriesRow {
  periodKey: string;
  kind: string;
  start: string;
  end: string;
  metrics: Metrics;
}

export interface LedgerEntry {
  id: string;
  action: string;
  targetMetric: string;
  targetValue: string;
  openedValue: string;
  latestValue: string;
  status: string;
  effort: string;
  openedPeriod: string;
  lastCheckedPeriod: string;
}

export interface InsightsIndex {
  series: SeriesRow[];
  ledger: Map<string, LedgerEntry>;
}

function str(v: unknown): string {
  return v == null ? '' : typeof v === 'string' ? v : String(v);
}
function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

/** Parse `index.json` (any subset of it). Unknown shapes → empty. */
export function parseIndex(raw: unknown): InsightsIndex {
  const out: InsightsIndex = { series: [], ledger: new Map() };
  if (!raw || typeof raw !== 'object') return out;
  const o = raw as Record<string, unknown>;
  if (Array.isArray(o.series)) {
    for (const r of o.series) {
      if (!r || typeof r !== 'object') continue;
      const row = r as Record<string, unknown>;
      const h = (row.headline && typeof row.headline === 'object' ? row.headline : {}) as Record<string, unknown>;
      const metrics: Metrics = {};
      const s = num(h.total_sessions);
      const t = num(h.total_messages);
      const e = num(h.tool_error_total);
      const a = num(h.achievement_rate);
      const c = num(h.total_cost_usd) ?? num(h.cost_usd) ?? num(h.spend_usd);
      if (s != null) metrics.sessions = s;
      if (t != null) metrics.turns = t;
      if (e != null) metrics.toolErrors = e;
      if (a != null) metrics.achievement = a;
      if (c != null) metrics.spend = c;
      out.series.push({ periodKey: str(row.period_key), kind: str(row.kind), start: str(row.start), end: str(row.end), metrics });
    }
  }
  if (Array.isArray(o.action_ledger)) {
    for (const r of o.action_ledger) {
      if (!r || typeof r !== 'object') continue;
      const e = r as Record<string, unknown>;
      const id = str(e.id);
      if (!id) continue;
      out.ledger.set(id, {
        id,
        action: str(e.action),
        targetMetric: str(e.target_metric),
        targetValue: str(e.target_value),
        openedValue: str(e.opened_value),
        latestValue: str(e.latest_value),
        status: str(e.status).toLowerCase(),
        effort: str(e.effort),
        openedPeriod: str(e.opened_period),
        lastCheckedPeriod: str(e.last_checked_period),
      });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Reports: keys, paths, KPIs, deltas
// ---------------------------------------------------------------------------

export interface ReportLike {
  kind: string;
  period_start: string;
  period_end: string;
  html_path: string | null;
  summary: string;
  created_at: string;
}

/** `daily:20260923_20260923` — the same key the index `series` uses. */
export function periodKey(r: Pick<ReportLike, 'kind' | 'period_start' | 'period_end'>): string {
  return `${r.kind}:${r.period_start.replace(/-/g, '')}_${r.period_end.replace(/-/g, '')}`;
}

/** Sibling artifact path of a report HTML (`report-*.html` → `summary-*.md`). */
export function siblingPath(htmlPath: string, which: 'summary' | 'metrics'): string | null {
  const m = /^(.*[\\/])report-([^\\/]+)\.html$/.exec(htmlPath);
  if (!m) return null;
  return which === 'summary' ? `${m[1]}summary-${m[2]}.md` : `${m[1]}metrics-${m[2]}.json`;
}

/** `<insights>/index.json` from any report HTML path (`<insights>/<kind>/report-….html`). */
export function indexPathFrom(htmlPath: string): string | null {
  const m = /^(.*)[\\/][^\\/]+[\\/]report-[^\\/]+\.html$/.exec(htmlPath);
  return m ? `${m[1]}/index.json` : null;
}

/** Headline KPIs for a report: the summary prose first (it's what the agent
 *  actually reported — the collector is known to under-count errors), the
 *  index `series` row for anything the prose didn't state. */
export function reportMetrics(parsed: Metrics, row: SeriesRow | undefined): { values: Metrics; source: Partial<Record<MetricKey, 'summary' | 'index'>> } {
  const values: Metrics = {};
  const source: Partial<Record<MetricKey, 'summary' | 'index'>> = {};
  for (const k of METRIC_KEYS) {
    if (parsed[k] != null) {
      values[k] = parsed[k];
      source[k] = 'summary';
    } else if (row?.metrics[k] != null) {
      values[k] = row.metrics[k];
      source[k] = 'index';
    }
  }
  return { values, source };
}

export type DeltaTone = 'good' | 'bad' | 'neutral';

export interface Delta {
  diff: number;
  /** Relative change vs the previous value (null when previous is 0). */
  pct: number | null;
  direction: 'up' | 'down' | 'flat';
  tone: DeltaTone;
}

/** Change from `prev` to `cur` for a metric, toned by whether up is good. */
export function delta(key: MetricKey, cur: number | undefined, prev: number | undefined): Delta | null {
  if (cur == null || prev == null) return null;
  const diff = cur - prev;
  const direction = diff > 0 ? 'up' : diff < 0 ? 'down' : 'flat';
  const good = METRIC_GOOD[key];
  const tone: DeltaTone =
    direction === 'flat' || good === 'neutral' ? 'neutral' : direction === good ? 'good' : 'bad';
  return { diff, pct: prev !== 0 ? diff / Math.abs(prev) : null, direction, tone };
}

/** Tone for an action status chip. */
export function statusTone(s: ActionStatus | string): 'warning' | 'success' | 'info' | 'neutral' {
  switch (s) {
    case 'regressed':
      return 'warning';
    case 'improved':
    case 'closed':
      return 'success';
    case 'new':
      return 'info';
    default:
      return 'neutral';
  }
}

/** Human label for a status word. */
export function statusLabel(s: ActionStatus | string): string {
  switch (s) {
    case 'regressed':
      return 'Regressed';
    case 'improved':
      return 'Improved';
    case 'new':
      return 'New';
    case 'carried':
      return 'Carried over';
    case 'closed':
      return 'Closed';
    case 'open':
      return 'Open';
    default:
      return s ? s[0].toUpperCase() + s.slice(1) : '';
  }
}

/** One status for an item: the ledger wins (it's the carried state), then the
 *  summary's own words — regressed beats improved beats new. */
export function itemStatus(item: ActionItem, ledger?: Map<string, LedgerEntry>): ActionStatus | null {
  const ledgerStatuses = item.ids.map((id) => ledger?.get(id)?.status).filter(Boolean) as string[];
  const all = [...item.statuses, ...ledgerStatuses];
  for (const s of ['regressed', 'improved', 'closed', 'new', 'carried', 'open'] as const) {
    if (all.includes(s)) return s;
  }
  return null;
}
