// Report feed — the deterministic "extras" a generated report needs on top of
// reportInput's model: KPI trend across recent periods, outliers, per-PR flow
// items, rework pairs with charged days, DORA failures with titles, deploy tags
// (hotfix-flagged), blind spots from guardrails, improved/declined KPIs and a
// short rule-based narrative. Pure: no I/O, no clock, no randomness — the same
// input always yields the same output (reports are diffable/commentable).
//
// Masking: titles are dropped and every person becomes a stable alias
// ("Person A", "Person B", … assigned in sorted-name order) so a masked report
// never carries a name, a summary or a free-text period label.
'use strict';

const crypto = require('node:crypto');

const TREND_MIN = 3;
const TREND_MAX = 5;
const TOP_N = 5;
const DAY_HOURS = 24;

const r2 = (v) => Math.round(v * 100) / 100;
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const DEPLOY_RE = /deployed|hotfix|hf/i;
const HOTFIX_RE = /hotfix|hf/i;

/** Stable, non-reversible file hint for a masked per-person report. */
function assigneeFileHint(assignee) {
  return crypto.createHash('sha256').update(String(assignee || '')).digest('hex').slice(0, 12);
}

/** Days → hours (null-safe, 2dp). PR metrics are stored in days; tables show hours. */
function daysToHours(d) {
  const v = num(d);
  return v == null ? null : r2(v * DAY_HOURS);
}

/** name → alias, stable regardless of encounter order. */
function aliasMap(names) {
  const uniq = [...new Set(names.filter((n) => n))].sort((a, b) => a.localeCompare(b));
  const label = (i) => {
    let s = '';
    let n = i;
    do { s = String.fromCharCode(65 + (n % 26)) + s; n = Math.floor(n / 26) - 1; } while (n >= 0);
    return `Person ${s}`;
  };
  return new Map(uniq.map((n, i) => [n, label(i)]));
}

/** Tickets of the current period, normalised from `current.records` or the phase table. */
function ticketsOf(current) {
  if (Array.isArray(current.records)) {
    return current.records.map((r) => {
      const ph = r.phases || {};
      return {
        key: r.key, title: r.summary || r.title || '', person: r.assignee_name || r.person || '',
        dev_days: num(r.dev_days) ?? num(ph.dev && ph.dev.days),
        rework_days: num(r.rework_days) ?? num(ph.rework && ph.rework.in_days),
        estimate_days: num(r.estimate_days),
        actual_days: num(r.actual_days) ?? num(r.dev_days) ?? num(ph.dev && ph.dev.days),
        jira_rework_of: r.rework_of || null,
      };
    });
  }
  return ((current.phases && current.phases.tickets) || []).map((t) => ({
    key: t.key, title: t.title || '', person: t.person || '',
    dev_days: num(t.dev), rework_days: num(t.rework), estimate_days: num(t.points), actual_days: num(t.dev), jira_rework_of: null,
  }));
}

function topBy(items, score, n = TOP_N) {
  return items.map((x) => ({ x, s: score(x) })).filter((e) => e.s != null && e.s > 0)
    .sort((a, b) => b.s - a.s || String(a.x.key).localeCompare(String(b.x.key))).slice(0, n);
}

/** Trend: last 3–5 prior periods + current, per KPI. */
function buildTrend(metricsByPeriod, current, masked) {
  const prior = (metricsByPeriod || []).slice(-(TREND_MAX - 1));
  const series = prior.concat([{ label: current.label || 'Current', kpis: current.kpis || [] }]);
  if (series.length < TREND_MIN) return [];
  return (current.kpis || []).map((k) => {
    const points = series.map((p) => {
      const hit = (p.kpis || []).find((x) => x.id === k.id);
      const value = hit ? num(hit.value) : null;
      return masked ? value : { label: p.label || p.id || '', value };
    });
    return { id: k.id, label: k.label, unit: k.unit || '', better: k.better || null, kind: k.kind || '', points };
  }).filter((t) => t.points.some((p) => (masked ? p : p.value) != null));
}

/** KPI deltas vs the immediately previous period. */
function buildDeltas(metricsByPeriod, current) {
  const prev = (metricsByPeriod || [])[(metricsByPeriod || []).length - 1];
  const improved = [];
  const declined = [];
  if (!prev) return { improved, declined };
  for (const k of current.kpis || []) {
    if (k.better !== 'up' && k.better !== 'down') continue;
    const p = (prev.kpis || []).find((x) => x.id === k.id);
    const from = p ? num(p.value) : null;
    const to = num(k.value);
    if (from == null || to == null || from === to) continue;
    const delta = r2(to - from);
    const pct = from !== 0 ? r2(((to - from) / Math.abs(from)) * 100) : null;
    const good = k.better === 'up' ? delta > 0 : delta < 0;
    (good ? improved : declined).push({ id: k.id, label: k.label, kind: k.kind || '', unit: k.unit || '', from, to, delta, pct });
  }
  const mag = (x) => (x.pct == null ? Infinity : Math.abs(x.pct));
  const ord = (a, b) => mag(b) - mag(a) || a.id.localeCompare(b.id);
  return { improved: improved.sort(ord), declined: declined.sort(ord) };
}

/** Rework pairs (B reworked A's recent code) with charged days: explicit per-pair days, else A's days apportioned by line share. */
function buildReworkPairs(reworkResult, tickets) {
  const pairs = (reworkResult && reworkResult.pairs) || {};
  const byKey = new Map(tickets.map((t) => [t.key, t]));
  const entries = Object.entries(pairs).map(([id, p]) => {
    const [by, key] = id.split('>');
    return { key, by_key: by, lines: num(p.lines) || 0, days: num(p.days) ?? num(p.charged_days) };
  });
  const linesIn = {};
  for (const e of entries) linesIn[e.key] = (linesIn[e.key] || 0) + e.lines;
  return entries.map((e) => {
    const t = byKey.get(e.key);
    let days = e.days;
    let basis = days != null ? 'charged' : null;
    if (days == null && t && t.rework_days != null && linesIn[e.key] > 0) {
      days = r2(t.rework_days * (e.lines / linesIn[e.key]));
      basis = 'apportioned_by_lines';
    }
    return { key: e.key, by_key: e.by_key, title: t ? t.title : '', person: t ? t.person : '', lines: e.lines, days, days_basis: basis };
  }).sort((a, b) => (b.days ?? -1) - (a.days ?? -1) || b.lines - a.lines || a.key.localeCompare(b.key) || a.by_key.localeCompare(b.by_key));
}

/** Guardrails → blind spots grouped by what the reader cannot trust. */
function buildBlindSpots(guardrails) {
  const out = [];
  const seen = new Set();
  for (const g of guardrails || []) {
    const text = `${g.id || ''} ${g.code || ''} ${g.metric || ''} ${g.message || g.reason || g.msg || ''}`;
    let kind = null;
    if (/approximat/i.test(text)) kind = 'approximated_pr_times';
    else if (/design/i.test(text)) kind = 'no_design_evidence';
    else if (/low_n|coverage|not_available|unlinked|sample|n=/i.test(text)) kind = 'low_coverage';
    if (!kind) continue;
    const dedupe = `${kind}|${g.metric || g.id || ''}`;
    if (seen.has(dedupe)) continue;
    seen.add(dedupe);
    out.push({ kind, metric: g.metric || g.id || '', level: g.level || g.severity || 'warn', message: g.message || g.reason || g.msg || '' });
  }
  const rank = { low_coverage: 0, approximated_pr_times: 1, no_design_evidence: 2 };
  return out.sort((a, b) => rank[a.kind] - rank[b.kind] || a.metric.localeCompare(b.metric));
}

/** Deterministic, rule-based narrative lines (no LLM). Reads only numbers + labels. */
function buildNarrative({ improved, declined, outliers, blind_spots, deploy_tags, dora_failures }) {
  const lines = [];
  const fmt = (x) => `${x.label} ${x.from} → ${x.to}${x.pct != null ? ` (${x.pct > 0 ? '+' : ''}${x.pct}%)` : ''}`;
  if (improved.length) lines.push(`Improved: ${improved.slice(0, 3).map(fmt).join('; ')}.`);
  if (declined.length) lines.push(`Declined: ${declined.slice(0, 3).map(fmt).join('; ')}.`);
  const hf = deploy_tags.filter((t) => t.hotfix).length;
  if (deploy_tags.length) lines.push(`${deploy_tags.length} deployment(s) in the period, ${hf} of them hotfix${hf === 1 ? '' : 'es'}.`);
  if (dora_failures.length) lines.push(`${dora_failures.length} change failure(s) recorded.`);
  if (outliers.rework_days.length) lines.push(`Largest rework charge: ${outliers.rework_days[0].value} day(s) on one ticket.`);
  if (blind_spots.length) lines.push(`${blind_spots.length} blind spot(s): read the affected metrics as indicative only.`);
  return lines;
}

/**
 * Build the report extras.
 * @param {object} a
 * @param {string} [a.scope] report scope label (informational)
 * @param {Array<{id?,label?,kpis}>} a.metricsByPeriod prior periods, oldest first (reportInput outputs or {label,kpis})
 * @param {object} a.current reportInput output, optionally with `records` (scope records delivered in the period,
 *   may carry estimate_days/actual_days/rework_of), `tags` ([{name,ts,kind?}]), `period` ({start,end}) and `label`
 * @param {boolean} [a.masked]
 * @param {object} [a.reworkResult] lib/rework.js result ({pairs:{'B>A':{lines,days?}}})
 * @param {Array} [a.prs] per-PR metrics ({id,key,title,pickup_days,review_days,merge_lag_days,size,comments_count})
 * @param {Array} [a.guardrails] report guardrails ({id,level,metric,message}) or raw ({code,severity,reason})
 */
function buildReportExtras({ scope, metricsByPeriod = [], current = {}, masked = false, reworkResult = null, prs = [], guardrails = [] } = {}) {
  void scope;
  const tickets = ticketsOf(current);
  const titleOf = new Map(tickets.map((t) => [t.key, t.title]));
  const recTitle = new Map((current.records || []).map((r) => [r.key, r.summary || r.title || '']));

  const ticketRef = (t, value, extra = {}) => ({ key: t.key, title: t.title, person: t.person, value: num(value) == null ? value : r2(value), ...extra });
  const outliers = {
    dev_days: topBy(tickets, (t) => t.dev_days).map((e) => ticketRef(e.x, e.s)),
    rework_days: topBy(tickets, (t) => t.rework_days).map((e) => ticketRef(e.x, e.s)),
    rework_lines: [],
    estimate_miss: topBy(tickets, (t) => (t.estimate_days > 0 && t.actual_days > 0 ? Math.abs(Math.log(t.actual_days / t.estimate_days)) : null))
      .map((e) => ticketRef(e.x, e.s, { estimate_days: e.x.estimate_days, actual_days: e.x.actual_days, ratio: r2(e.x.actual_days / e.x.estimate_days) })),
    jira_rework: [],
  };
  const rework_pairs = buildReworkPairs(reworkResult, tickets);
  const linesByKey = {};
  for (const p of rework_pairs) linesByKey[p.key] = (linesByKey[p.key] || 0) + p.lines;
  const tByKey = new Map(tickets.map((t) => [t.key, t]));
  outliers.rework_lines = Object.entries(linesByKey).filter(([, v]) => v > 0)
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, TOP_N)
    .map(([key, v]) => { const t = tByKey.get(key) || { key, title: '', person: '' }; return ticketRef(t, v); });
  const jr = (current.rework && current.rework.jira) || [];
  outliers.jira_rework = jr.length
    ? jr.slice().sort((a, b) => (num(b.points) ?? -1) - (num(a.points) ?? -1) || a.key.localeCompare(b.key)).slice(0, TOP_N)
      .map((x) => { const t = tByKey.get(x.key) || {}; return { key: x.key, title: x.title || t.title || '', person: t.person || '', value: num(x.points), source_key: x.source_key || null, reason: x.reason || '' }; })
    : tickets.filter((t) => t.jira_rework_of).sort((a, b) => a.key.localeCompare(b.key)).slice(0, TOP_N)
      .map((t) => ({ key: t.key, title: t.title, person: t.person, value: null, source_key: t.jira_rework_of, reason: '' }));

  const pr_flow_items = (prs || []).map((p) => ({
    id: p.id ?? null, key: p.key || null, title: p.title || '',
    pickup_hours: daysToHours(p.pickup_days), review_hours: daysToHours(p.review_days), merge_hours: daysToHours(p.merge_lag_days),
    size: num(p.size), comments: num(p.comments_count ?? p.comments),
  })).sort((a, b) => String(a.id).localeCompare(String(b.id), undefined, { numeric: true }));
  const total = (x) => (x.pickup_hours || 0) + (x.review_hours || 0) + (x.merge_hours || 0);
  const slow_prs = pr_flow_items.filter((x) => total(x) > 0)
    .sort((a, b) => total(b) - total(a) || String(a.id).localeCompare(String(b.id))).slice(0, TOP_N)
    .map((x) => ({ ...x, total_hours: r2(total(x)) }));

  const phases_by_period = (metricsByPeriod || []).slice(-(TREND_MAX - 1)).concat([{ label: current.label || 'Current', phases: current.phases }])
    .map((p) => ({
      label: masked ? null : (p.label || p.id || ''),
      rows: ((p.phases && p.phases.rows) || []).map((r) => ({ phase: r.phase, median_days: num(r.median_days), tickets_tracked: r.tickets_tracked ?? null, tickets_total: r.tickets_total ?? null })),
    }));

  const dora_failures = ((current.dora && current.dora.failures) || []).map((f) => ({
    ...f, title: recTitle.get(f.key) || titleOf.get(f.key) || f.title || '',
  }));

  const start = current.period && current.period.start;
  const end = current.period && current.period.end;
  const deploy_tags = (current.tags || [])
    .filter((t) => DEPLOY_RE.test(t.name || '') && (!start || t.ts >= start) && (!end || t.ts < end))
    .map((t) => ({ name: t.name, ts: t.ts, hotfix: t.kind === 'hotfix' || HOTFIX_RE.test(t.name || '') }))
    .sort((a, b) => String(a.ts).localeCompare(String(b.ts)) || a.name.localeCompare(b.name));

  const blind_spots = buildBlindSpots(guardrails);
  const trend = buildTrend(metricsByPeriod, current, masked);
  const { improved, declined } = buildDeltas(metricsByPeriod, current);

  const out = { trend, outliers, pr_flow_items, slow_prs, phases_by_period, rework_pairs, dora_failures, deploy_tags, blind_spots, improved, declined };
  out.narrative = buildNarrative(out);
  if (masked) maskInPlace(out);
  return out;
}

/** Drop titles, alias persons, strip free text that may echo a name or summary. */
function maskInPlace(out) {
  const names = [];
  const collect = (x) => { if (x && x.person) names.push(x.person); };
  Object.values(out.outliers).forEach((l) => l.forEach(collect));
  out.rework_pairs.forEach(collect);
  const alias = aliasMap(names);
  const scrub = (x) => {
    if ('title' in x) x.title = '';
    if (x.person) x.person = alias.get(x.person);
    if ('caused_by' in x && x.caused_by) x.caused_by = '';
    return x;
  };
  Object.values(out.outliers).forEach((l) => l.forEach((x) => { scrub(x); if ('reason' in x) x.reason = ''; }));
  out.rework_pairs.forEach(scrub);
  out.pr_flow_items.forEach(scrub);
  out.slow_prs.forEach(scrub);
  out.dora_failures.forEach(scrub);
}

module.exports = { buildReportExtras, assigneeFileHint, daysToHours, aliasMap, TOP_N };
