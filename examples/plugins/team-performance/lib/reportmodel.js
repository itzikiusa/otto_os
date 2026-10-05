// Report model + deterministic HTML renderer for the team-performance reports.
//
// buildReportModel(input) is PURE: it takes the already-computed outputs of the
// analytics passes (phases, DORA, PR flow, rework, flow, capacity, sub-tasks,
// guardrails, people, trends) as plain objects and normalises them into one
// fixed-shape model with every section key present (SECTION_KEYS). Missing
// measurements stay null — they render as "not tracked", never as a fake 0.
//
// Masking (input.mask = true) happens HERE, before anything is rendered or
// embedded: person names become "Person A…", ticket keys "Ticket 1…", free-text
// ticket titles are dropped, and every remaining string is scrubbed of known
// names/keys. The alias dictionary is never put on the model (it holds the real
// names) — it lives in a WeakMap so renderReport can scrub the agent narrative
// and the host's comments with it.
//
// renderReport(model, narrative, opts) inlines report/{template.html,report.css,
// report.js} + the palette CSS (generated from PALETTE below — the ONLY place a
// colour literal lives) + the model JSON into one self-contained HTML string
// whose only <script> is its own nonce'd script (CSP pins script-src to that
// nonce). The agent fills only the narrative slots {summary, strengths[],
// goals[]}; it never writes HTML. opts.comments are rendered by the host too —
// report.js is progressive enhancement (theme, scroll-spy, posting comments).
//
// ---------------------------------------------------------------- input shape
// Every field is optional; unknown fields are ignored. Times are business days
// unless the name says hours.
/**
 * @typedef {object} ReportInput
 * @property {boolean} [mask]
 * @property {'team'|'combined'|'dev'} [report_kind]  'dev' = one person's report
 *   (adds the Goals section and marks DORA / PR flow as team context).
 * @property {string} [scope]  @property {{start,end}} [period]
 * @property {{label,start,end,kpis?:Object<string,number>}} [compare]
 * @property {string} [jiraBase]  https only; ignored when masked.
 * @property {{title,data_as_of,scan_time,ruler_version,generated_at,report_id,comments_endpoint}} [meta]
 * @property {Array<{id,label,value,unit,kind,better,capacity_days,prior,note,history?:number[]}>} [kpis]
 * @property {Array<{id,label,unit,better,kind,points:Array<{label,value}>}>} [trend]
 *   Last 4–7 periods per KPI (oldest first). ids matching a kpi id (or
 *   deploy_frequency / lead_time / change_failure_rate / mttr) feed sparklines.
 * @property {{rows:Array<{phase,median_days,p85_days,total_days,tickets_tracked,tickets_total}>,
 *   splits?:Array<{phase,label,median_days,total_days}>,
 *   by_period?:Array<{label,design,dev,review,deployment,rework}>,
 *   tickets?:Array<{key,title,person,type,design,dev,review,deployment,rework,estimate_days}>}} [phases]
 * @property {{deployments,hotfixes,deploy_frequency_per_week,lead_time_days_median,lead_time_days_p85,
 *   change_failure_rate,failed_deployments,incidents,mttr_hours_median,weekly:Array<{week,deployments}>,
 *   failures:Array<{key,title,tag,kind,restore_hours,caused_by}>}} [dora]
 * @property {{prs,merged,pickup_hours_median,review_hours_median,merge_hours_median,size_lines_median,
 *   comments_per_pr,reviews_per_pr,unreviewed_share,coverage_note,
 *   items?:Array<{key,person,pickup_hours,review_hours,size_lines}>,
 *   by_person?:Array<{person,prs,reviews_given,pickup_hours_median,size_lines_median}>}} [pr_flow]
 * @property {{rate,days_charged,in:Array<{key,by_key,person,lines,days}>,out:Array<…>,
 *   jira:Array<{key,source_key,reason,title,points,excluded_from_scope}>}} [rework]
 *   rework.in pairs are origin ← rework (key = the original ticket, by_key = the later one).
 * @property {{wip_avg,wip_per_person,throughput,investment:Array<{label,share,points}>,
 *   investment_by_epic?:Array<{epic_key,epic_title,share,points,tickets}>,
 *   unplanned_share,unplanned?:Array<{key,title,type,person,reason,points}>,
 *   context_switch_avg,estimate_accuracy:{median_ratio,within_band_share,buckets:Array<{label,count}>}}} [flow]
 * @property {{est_inflation,pace_raw,pace_adjusted,pace_ref,comparable_n,ref_label,note}} [estimate_basis]
 * @property {Array<{person,working_days,time_off_days,capacity_days,delivered_points,points_per_capacity_day,dev_days}>} [capacity]
 * @property {Array<{person,key,parent_key,title,parent_owner,dev_days,commits,reason}>} [substantive_subtasks]
 * @property {Array<{level|severity,metric,message}>} [guardrails]  severity danger→error, warning→warn, info→info.
 * @property {Array<{name,role,capacity_days,time_off_days,delivered_points,…,phase_days?:{design,dev,review,deployment,rework},tickets,notes}>} [people]
 * @property {{dev?,rework?,estimate?,jira_linked?:Array<{key,title,person,value,unit,note}>}} [outliers]  derived when absent.
 * @property {Array<{goal,metric,target,actual,met,note}>} [goals]  previous period's goals (dev reports).
 * @property {string[]} [next_steps]  @property {string[]} [blind_spots]
 * @property {Array<{term,definition}>} [glossary]
 */
'use strict';
const fs = require('fs');
const path = require('path');
const C = require('../report/charts.js');
const THEME = require('./theme.js');
const SAN = require('./sanitize.js');

const TEMPLATE_VERSION = '2';
const SECTION_KEYS = [
  'kpis', 'trend', 'phases', 'outliers', 'dora', 'pr_flow', 'rework', 'flow', 'estimate_basis', 'capacity',
  'substantive_subtasks', 'guardrails', 'people', 'goals', 'next_steps', 'blind_spots', 'glossary', 'meta',
];
const PHASES = [
  ['design', 'Design'],
  ['dev', 'Dev'],
  ['review', 'Review'],
  ['deployment', 'Deployment'],
  ['rework', 'Rework'],
];
const PHASE_CLASSES = PHASES.map(([id]) => C.PHASE_CLASS[id]);
const KEY_RE = /\b[A-Z][A-Z0-9]+-\d+\b/g;
const KEY_FULL = /^[A-Z][A-Z0-9]+-\d+$/;
// Fields whose value is a person / a ticket key / free text that may name either.
const NAME_FIELDS = new Set(['name', 'person', 'assignee', 'author', 'reviewer', 'owner', 'parent_owner', 'display_name']);
const KEY_FIELDS = new Set(['key', 'parent_key', 'source_key', 'target_key', 'epic_key', 'reworked_key', 'by_key', 'ticket', 'caused_by']);
const TITLE_FIELDS = new Set(['title', 'summary', 'parent_title', 'epic_name', 'epic_title', 'description']);
const { NOT_TRACKED, esc, isNum, fmt } = C;

// ------------------------------------------------------------ palette
// Every colour in the report comes from lib/theme.js, which scripts/gen-theme.js
// generates from Otto's ui/src/lib/tokens.css (color-mix resolved), so the
// report can never drift from the app. report.css only references var(--…)
// (plus a zero-specificity tp:fallback block). paletteCss() emits the scale,
// light, dark (prefers-color-scheme unless the reader forced light, plus an
// explicit [data-theme='dark']) and print.
const PALETTE = Object.freeze({ light: THEME.light, dark: THEME.dark, print: THEME.print });
function paletteCss(p = PALETTE, scale = THEME.scale) {
  const decl = (o) => Object.entries(o).map(([k, x]) => `--${k}: ${x};`).join(' ');
  return [
    `:root { color-scheme: light; ${decl(scale)} ${decl(p.light)} }`,
    `@media (prefers-color-scheme: dark) { :root:not([data-theme='light']) { color-scheme: dark; ${decl(p.dark)} } }`,
    `:root[data-theme='dark'] { color-scheme: dark; ${decl(p.dark)} }`,
    `@media print { :root, :root[data-theme='dark'] { color-scheme: light; ${decl(p.print)} } }`,
  ].join('\n');
}

// ------------------------------------------------------------ primitives
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : typeof v === 'string' && v.trim() !== '' && Number.isFinite(+v) ? +v : null);
const str = (v) => (v == null ? null : String(v));
const arr = (v) => (Array.isArray(v) ? v : []);
const obj = (v) => (v && typeof v === 'object' && !Array.isArray(v) ? v : {});
const pct = (v) => (isNum(v) ? `${fmt(v * 100, 0)}%` : NOT_TRACKED);
const iso = (v) => {
  if (v == null || v === '') return null;
  const t = typeof v === 'number' ? v : Date.parse(v);
  return Number.isFinite(t) ? new Date(t).toISOString() : null;
};
const day = (v) => (iso(v) || '').slice(0, 10) || null;
// Jira links only over https, to a plain host[/path] — anything else drops links.
const cleanBase = (u) => (typeof u === 'string' && SAN.jiraOrigin(u.trim()) && /^https:\/\/[A-Za-z0-9.-]+(:\d+)?(\/[A-Za-z0-9._~/-]*)?$/.test(u.trim()) ? u.trim().replace(/\/+$/, '') : null);
const median = (xs) => {
  const v = xs.filter(isNum).sort((a, b) => a - b);
  if (!v.length) return null;
  const m = (v.length - 1) / 2;
  return (v[Math.floor(m)] + v[Math.ceil(m)]) / 2;
};
const alpha = (i) => {
  let s = '';
  let n = i;
  do {
    s = String.fromCharCode(65 + (n % 26)) + s;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return s;
};
const reEsc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

// ------------------------------------------------------------ masking
// Walk the raw input once, collecting every person name and ticket key so the
// alias order is deterministic (people[] order first, then sorted leftovers).
function collectIdentities(input) {
  const names = [];
  const seenN = new Set();
  const addName = (n) => {
    const s = typeof n === 'string' ? n.trim() : '';
    if (s.length < 2 || seenN.has(s)) return;
    seenN.add(s);
    names.push(s);
  };
  for (const p of arr(input.people)) addName(obj(p).name);
  const extraNames = [];
  const keys = new Set();
  const walk = (v, field) => {
    if (typeof v === 'string') {
      if (NAME_FIELDS.has(field)) extraNames.push(v);
      for (const m of v.matchAll(KEY_RE)) keys.add(m[0]);
    } else if (Array.isArray(v)) v.forEach((x) => walk(x, field));
    else if (v && typeof v === 'object') for (const [k, x] of Object.entries(v)) walk(x, k);
  };
  walk(input, '');
  [...new Set(extraNames)].sort().forEach(addName);
  return { names, keys: [...keys].sort(keyCmp) };
}
function keyCmp(a, b) {
  const [pa, na] = a.split('-');
  const [pb, nb] = b.split('-');
  return pa < pb ? -1 : pa > pb ? 1 : +na - +nb;
}

function makeMasker(input) {
  const { names, keys } = collectIdentities(input);
  const nameMap = new Map(names.map((n, i) => [n, `Person ${alpha(i)}`]));
  const keyMap = new Map(keys.map((k, i) => [k, `Ticket ${i + 1}`]));
  // Longest first so "Ann Lee" wins over "Ann"; first names are scrubbed too.
  const nameParts = new Map();
  for (const [n, a] of nameMap) {
    nameParts.set(n, a);
    for (const part of n.split(/\s+/)) if (part.length >= 3 && !nameParts.has(part)) nameParts.set(part, a);
  }
  const ordered = [...nameParts.keys()].sort((a, b) => b.length - a.length);
  const nameRe = ordered.length ? new RegExp(`(^|[^\\p{L}\\p{N}])(${ordered.map(reEsc).join('|')})(?![\\p{L}\\p{N}])`, 'giu') : null;
  const lower = new Map([...nameParts].map(([k, v]) => [k.toLowerCase(), v]));
  const scrub = (s) => {
    if (typeof s !== 'string') return s;
    let out = s.replace(KEY_RE, (k) => keyMap.get(k) || 'a ticket');
    if (nameRe) out = out.replace(nameRe, (m, pre, n) => pre + (lower.get(n.toLowerCase()) || 'a person'));
    return out;
  };
  const person = (n) => (n == null ? null : nameMap.get(String(n).trim()) || scrub(String(n)));
  const key = (k) => (k == null ? null : keyMap.get(String(k)) || scrub(String(k)));
  const deep = (v, field) => {
    if (typeof v === 'string') {
      if (NAME_FIELDS.has(field)) return person(v);
      if (KEY_FIELDS.has(field)) return key(v);
      if (TITLE_FIELDS.has(field)) return null;
      return scrub(v);
    }
    if (Array.isArray(v)) return v.map((x) => deep(x, KEY_FIELDS.has(field) ? field : NAME_FIELDS.has(field) ? field : field === 'keys' ? 'key' : field));
    if (v && typeof v === 'object') {
      const o = {};
      for (const [k, x] of Object.entries(v)) o[k] = deep(x, k);
      return o;
    }
    return v;
  };
  return { deep, scrub, person, key, names, keys };
}

// ------------------------------------------------------------ section normalisers
function normTrend(raw) {
  return arr(raw)
    .map((t) => {
      const o = obj(t);
      const points = arr(o.points).map((p) => ({ label: str(obj(p).label) || '', value: num(obj(p).value) })).slice(-7);
      return { id: str(o.id) || '', label: str(o.label) || str(o.id) || '', unit: str(o.unit) || '', kind: str(o.kind) || 'count', better: o.better === 'down' ? 'down' : o.better === 'up' ? 'up' : null, points };
    })
    .filter((t) => t.id && t.points.length);
}

function normKpis(list, compare, trend) {
  const prior = obj(obj(compare).kpis);
  return arr(list).map((k, i) => {
    const o = obj(k);
    const id = str(o.id) || `kpi-${i + 1}`;
    const value = num(o.value);
    const p = num(o.prior ?? prior[id]);
    const kind = ['ratio', 'count', 'days', 'hours', 'percent', 'points'].includes(o.kind) ? o.kind : 'count';
    const t = trend.find((x) => x.id === id);
    const history = t ? t.points.map((x) => x.value) : arr(o.history).map(num).slice(-7);
    return {
      id,
      label: str(o.label) || id,
      value,
      unit: str(o.unit) || '',
      kind,
      better: o.better === 'down' ? 'down' : o.better === 'up' ? 'up' : null,
      capacity_days: num(o.capacity_days),
      prior: p,
      delta: isNum(value) && isNum(p) ? value - p : null,
      note: str(o.note),
      history,
      history_labels: t ? t.points.map((x) => x.label) : [],
    };
  });
}

function normPhases(raw) {
  const r = obj(raw);
  const byPhase = new Map(arr(r.rows).map((x) => [obj(x).phase, obj(x)]));
  const rows = PHASES.map(([id, label]) => {
    const x = byPhase.get(id) || {};
    const tracked = num(x.tickets_tracked);
    const total = num(x.tickets_total);
    const none = tracked === 0;
    return {
      phase: id,
      label: str(x.label) || label,
      median_days: none ? null : num(x.median_days),
      p85_days: none ? null : num(x.p85_days),
      total_days: none ? null : num(x.total_days),
      tickets_tracked: tracked,
      tickets_total: total,
      coverage: isNum(tracked) && isNum(total) && total > 0 ? tracked / total : null,
      not_tracked: none || (num(x.median_days) == null && num(x.total_days) == null),
    };
  });
  const splits = arr(r.splits).map((s) => ({ phase: str(obj(s).phase), label: str(obj(s).label), median_days: num(obj(s).median_days), total_days: num(obj(s).total_days) }));
  const phaseVals = (o) => Object.fromEntries(PHASES.map(([id]) => [id, num(o[id])]));
  const tickets = arr(r.tickets).map((t) => {
    const o = obj(t);
    return { key: str(o.key), title: str(o.title), person: str(o.person), type: str(o.type), estimate_days: num(o.estimate_days), ...phaseVals(o) };
  });
  // by_period rows are flat {label, design, dev, …} or reportfeed's {label, rows:[{phase, median_days}]}.
  const periodVals = (o) => (Array.isArray(o.rows) ? phaseVals(Object.fromEntries(o.rows.map((x) => [obj(x).phase, obj(x).median_days]))) : phaseVals(o));
  const by_period = arr(r.by_period).map((p, i) => ({ label: str(obj(p).label) || `Period ${i + 1}`, ...periodVals(obj(p)) })).slice(-7);
  return { rows, splits, tickets, by_period };
}

function normDora(raw) {
  const d = obj(raw);
  const failures = arr(d.failures).map((f) => ({ key: str(obj(f).key), title: str(obj(f).title), tag: str(obj(f).tag), kind: str(obj(f).kind), restore_hours: num(obj(f).restore_hours), caused_by: str(obj(f).caused_by) }));
  return {
    deployments: num(d.deployments),
    hotfixes: num(d.hotfixes),
    deploy_frequency_per_week: num(d.deploy_frequency_per_week),
    lead_time_days_median: num(d.lead_time_days_median),
    lead_time_days_p85: num(d.lead_time_days_p85),
    change_failure_rate: num(d.change_failure_rate),
    failed_deployments: num(d.failed_deployments) ?? (failures.length ? failures.length : null),
    incidents: num(d.incidents) ?? (failures.some((f) => isNum(f.restore_hours)) ? failures.filter((f) => isNum(f.restore_hours)).length : null),
    mttr_hours_median: num(d.mttr_hours_median),
    weekly: arr(d.weekly).map((w) => ({ week: str(obj(w).week), deployments: num(obj(w).deployments) })),
    failures,
    // Deployment tags in the window (gitscan deploy_tags): newest first.
    deploy_tags: arr(d.deploy_tags ?? d.tags)
      .map((t) => {
        const o = obj(t);
        const name = str(o.name) || '';
        const kind = o.hotfix === true || o.kind === 'hotfix' || /hotfix|(^|[^a-z])hf([^a-z]|$)/i.test(name) ? 'hotfix' : 'deploy';
        return { name, repo: str(o.repo), at: iso(o.ts ?? o.at ?? o.day), kind, keys: arr(o.keys).map(str).filter(Boolean) };
      })
      .filter((t) => t.name)
      .sort((a, b) => String(b.at || '').localeCompare(String(a.at || ''))),
  };
}

function normPrFlow(raw) {
  const p = obj(raw);
  return {
    prs: num(p.prs),
    merged: num(p.merged),
    pickup_hours_median: num(p.pickup_hours_median),
    review_hours_median: num(p.review_hours_median),
    merge_hours_median: num(p.merge_hours_median),
    size_lines_median: num(p.size_lines_median),
    comments_per_pr: num(p.comments_per_pr),
    reviews_per_pr: num(p.reviews_per_pr),
    unreviewed_share: num(p.unreviewed_share),
    items: arr(p.items).map((x) => {
      const o = obj(x);
      return { key: str(o.key), pr: str(o.pr ?? o.id), repo: str(o.repo), title: str(o.title), person: str(o.person), pickup_hours: num(o.pickup_hours), review_hours: num(o.review_hours), merge_hours: num(o.merge_hours), size_lines: num(o.size_lines ?? o.size), comments: num(o.comments) };
    }),
    // Explicit slow-PR list wins; otherwise renderPrFlow derives it from items.
    slow: p.slow || p.slow_prs ? arr(p.slow ?? p.slow_prs).map((x) => {
      const o = obj(x);
      return { key: str(o.key), pr: str(o.pr ?? o.id), repo: str(o.repo), title: str(o.title), person: str(o.person), pickup_hours: num(o.pickup_hours), review_hours: num(o.review_hours), merge_hours: num(o.merge_hours), size_lines: num(o.size_lines ?? o.size), comments: num(o.comments) };
    }) : null,
    by_person: arr(p.by_person).map((x) => ({ person: str(obj(x).person), prs: num(obj(x).prs), reviews_given: num(obj(x).reviews_given), pickup_hours_median: num(obj(x).pickup_hours_median), size_lines_median: num(obj(x).size_lines_median) })),
    coverage_note: str(p.coverage_note),
  };
}

function normRework(raw) {
  const r = obj(raw);
  const pair = (x) => ({ key: str(obj(x).key), by_key: str(obj(x).by_key), person: str(obj(x).person), lines: num(obj(x).lines), days: num(obj(x).days) });
  return {
    rate: num(r.rate),
    days_charged: num(r.days_charged),
    in: arr(r.in).map(pair),
    out: arr(r.out).map(pair),
    jira: arr(r.jira).map((x) => ({ key: str(obj(x).key), source_key: str(obj(x).source_key), reason: str(obj(x).reason), title: str(obj(x).title), points: num(obj(x).points), excluded_from_scope: obj(x).excluded_from_scope !== false })),
  };
}

function normFlow(raw) {
  const f = obj(raw);
  const ea = obj(f.estimate_accuracy);
  return {
    wip_avg: num(f.wip_avg),
    wip_per_person: num(f.wip_per_person),
    throughput: arr(f.throughput).map((w) => ({ week: str(obj(w).week), count: num(obj(w).count) })),
    investment: arr(f.investment).map((x) => ({ label: str(obj(x).label), share: num(obj(x).share), points: num(obj(x).points) })),
    investment_by_epic: arr(f.investment_by_epic).map((x) => ({ epic_key: str(obj(x).epic_key), epic_title: str(obj(x).epic_title), share: num(obj(x).share), points: num(obj(x).points), tickets: num(obj(x).tickets) })),
    unplanned_share: num(f.unplanned_share),
    unplanned: arr(f.unplanned).map((x) => ({ key: str(obj(x).key), title: str(obj(x).title), type: str(obj(x).type), person: str(obj(x).person), reason: str(obj(x).reason), points: num(obj(x).points) })),
    context_switch_avg: num(f.context_switch_avg),
    estimate_accuracy: {
      median_ratio: num(ea.median_ratio),
      within_band_share: num(ea.within_band_share),
      buckets: arr(ea.buckets).map((b) => ({ label: str(obj(b).label), count: num(obj(b).count) })),
    },
  };
}

function normEstimateBasis(raw) {
  const e = obj(raw);
  return { est_inflation: num(e.est_inflation), pace_raw: num(e.pace_raw), pace_adjusted: num(e.pace_adjusted), pace_ref: num(e.pace_ref), comparable_n: num(e.comparable_n), ref_label: str(e.ref_label), note: str(e.note) };
}

function normCapacity(raw) {
  return arr(raw).map((c) => {
    const o = obj(c);
    const cap = num(o.capacity_days);
    const pts = num(o.delivered_points);
    return {
      person: str(o.person),
      working_days: num(o.working_days),
      time_off_days: num(o.time_off_days),
      capacity_days: cap,
      delivered_points: pts,
      points_per_capacity_day: num(o.points_per_capacity_day) ?? (isNum(pts) && isNum(cap) && cap > 0 ? pts / cap : null),
      dev_days: num(o.dev_days),
    };
  });
}

function normSubtasks(raw) {
  return arr(raw).map((s) => {
    const o = obj(s);
    return { person: str(o.person), key: str(o.key), parent_key: str(o.parent_key), title: str(o.title), parent_owner: str(o.parent_owner), dev_days: num(o.dev_days), commits: num(o.commits), reason: str(o.reason) };
  });
}

const LEVELS = { error: 'error', danger: 'error', critical: 'error', warn: 'warn', warning: 'warn', info: 'info', note: 'info' };
function normGuardrails(raw) {
  return arr(raw).map((g) => {
    const o = obj(g);
    const lvl = LEVELS[String(o.level || o.severity || '').toLowerCase()] || 'warn';
    return { level: lvl, metric: str(o.metric), message: str(o.message) || [str(o.reason), str(o.action)].filter(Boolean).join(' ') };
  });
}

function normPeople(raw, phases) {
  return arr(raw).map((p) => {
    const o = obj(p);
    const name = str(o.name);
    const pd = obj(o.phase_days);
    const mine = phases.tickets.filter((t) => t.person && t.person === name);
    const phase_days = Object.fromEntries(PHASES.map(([id]) => [id, num(pd[id]) ?? median(mine.map((t) => t[id]))]));
    return {
      name,
      role: str(o.role),
      capacity_days: num(o.capacity_days),
      time_off_days: num(o.time_off_days),
      delivered_points: num(o.delivered_points),
      points_per_capacity_day: num(o.points_per_capacity_day),
      cycle_time_days_median: num(o.cycle_time_days_median),
      design_days: num(o.design_days),
      dev_days: num(o.dev_days),
      prs: num(o.prs),
      reviews_given: num(o.reviews_given),
      rework_in_lines: num(o.rework_in_lines),
      rework_out_lines: num(o.rework_out_lines),
      estimate_ratio_median: num(o.estimate_ratio_median),
      phase_days,
      tickets: arr(o.tickets).map((t) => ({ key: str(obj(t).key), title: str(obj(t).title), points: num(obj(t).points), dev_days: num(obj(t).dev_days), status: str(obj(t).status) })),
      notes: arr(o.notes).map(str).filter(Boolean),
    };
  });
}

const outlierRow = (x) => ({ key: str(obj(x).key), title: str(obj(x).title), person: str(obj(x).person), value: num(obj(x).value), unit: str(obj(x).unit) || '', note: str(obj(x).note) });
// Explicit lists win; otherwise derive from the ticket-level phase data, so the
// section is never empty when the tickets carry the numbers.
function normOutliers(raw, phases, rework) {
  const o = obj(raw);
  const top = (list, n = 5) => list.slice(0, n);
  const t = phases.tickets;
  const dev = o.dev ? arr(o.dev).map(outlierRow) : top(t.filter((x) => isNum(x.dev) && x.dev > 0).sort((a, b) => b.dev - a.dev)).map((x) => ({ key: x.key, title: x.title, person: x.person, value: x.dev, unit: 'd', note: null }));
  const rw = o.rework
    ? arr(o.rework).map(outlierRow)
    : t.some((x) => isNum(x.rework) && x.rework > 0)
      ? top(t.filter((x) => isNum(x.rework) && x.rework > 0).sort((a, b) => b.rework - a.rework)).map((x) => ({ key: x.key, title: x.title, person: x.person, value: x.rework, unit: 'd', note: null }))
      : top(rework.in.filter((x) => isNum(x.days) || isNum(x.lines)).sort((a, b) => (b.days ?? 0) - (a.days ?? 0) || (b.lines ?? 0) - (a.lines ?? 0))).map((x) => ({ key: x.key, title: null, person: x.person, value: x.days ?? x.lines, unit: isNum(x.days) ? 'd' : 'lines', note: x.by_key ? `rewritten by ${x.by_key}` : null }));
  const est = o.estimate
    ? arr(o.estimate).map(outlierRow)
    : top(
        t
          .filter((x) => isNum(x.dev) && isNum(x.estimate_days) && x.estimate_days > 0 && x.dev > 0)
          .map((x) => ({ ...x, ratio: x.dev / x.estimate_days }))
          .sort((a, b) => Math.abs(Math.log(b.ratio)) - Math.abs(Math.log(a.ratio))),
      ).map((x) => ({ key: x.key, title: x.title, person: x.person, value: x.ratio, unit: '×', note: `${fmt(x.dev)} d actual vs ${fmt(x.estimate_days)} d estimated` }));
  const jira = o.jira_linked ? arr(o.jira_linked).map(outlierRow) : top(rework.jira).map((x) => ({ key: x.key, title: x.title, person: null, value: x.points, unit: 'pts', note: [x.reason, x.source_key ? `reworks ${x.source_key}` : null].filter(Boolean).join(' · ') || null }));
  return { dev, rework: rw, estimate: est, jira_linked: jira };
}

// What improved / declined vs the comparison period. An explicit list wins;
// otherwise every KPI with a prior value and a "better" direction is sorted
// into one of the two lists by its relative change (flat within ±2% is left out).
function normChanges(raw, kpis) {
  const o = obj(raw);
  const row = (x) => {
    const r = obj(x);
    return { label: str(r.label) || '', from: num(r.from ?? r.prior), to: num(r.to ?? r.value), unit: str(r.unit) || '', kind: str(r.kind) || 'count', note: str(r.note), anchor: str(r.id) };
  };
  if (raw && (o.improved || o.declined)) return { improved: arr(o.improved).map(row).filter((x) => x.label), declined: arr(o.declined).map(row).filter((x) => x.label), derived: false };
  const improved = [];
  const declined = [];
  for (const k of kpis) {
    if (!isNum(k.value) || !isNum(k.prior) || !k.better) continue;
    const rel = k.prior === 0 ? (k.value === 0 ? 0 : 1) : (k.value - k.prior) / Math.abs(k.prior);
    if (Math.abs(rel) < 0.02) continue;
    const x = { label: k.label, from: k.prior, to: k.value, unit: k.unit, kind: k.kind, note: k.kind === 'ratio' && isNum(k.capacity_days) ? `over ${fmt(k.capacity_days)} capacity days` : null, anchor: k.id, rel };
    ((rel > 0) === (k.better === 'up') ? improved : declined).push(x);
  }
  const byMag = (a, b) => Math.abs(b.rel) - Math.abs(a.rel);
  return { improved: improved.sort(byMag), declined: declined.sort(byMag), derived: true };
}

function normGoals(raw) {
  return arr(raw)
    .map((g) => {
      const o = obj(g);
      return { goal: str(o.goal) || str(o.label) || '', metric: str(o.metric), target: str(o.target), actual: str(o.actual), met: o.met === true ? true : o.met === false ? false : null, note: str(o.note) };
    })
    .filter((g) => g.goal);
}

const DEFAULT_GLOSSARY = [
  ['Capacity day', 'A working day the person was available: weekdays in the period minus recorded time off. Every per-person ratio is divided by capacity days, never calendar days.'],
  ['Design', 'Time on design / spike / POC / research work or pre-dev activity. Only counted where evidence exists; otherwise it shows "not tracked" (not zero).'],
  ['Dev', 'In Progress → Code Review, plus commit stretches where the status was not moved. QA counts only when commits kept landing during QA. Backlog / To-Do never counts.'],
  ['Review', 'PR opened → merged. Pickup = PR opened → first review or approval.'],
  ['Deployment', 'PR merged → first deployment tag (a tag containing "deployed", "hf" or "hotfix").'],
  ['Rework', 'Time charged back to a ticket when later work rewrote its recent code (git blame) or Jira links it as a fix / follow-up / reopen. A rework ticket\'s estimate is not counted as new scope.'],
  ['Deployment frequency', 'Deployment tags per week in the period (DORA).'],
  ['Lead time for changes', 'First commit → deployment, median (DORA).'],
  ['Change failure rate', 'Share of deployments followed by a hotfix or a bug traced to that change (DORA).'],
  ['Time to restore', 'Failure detected → fix deployed, median hours (DORA).'],
  ['DORA band', 'Elite / high / medium / low per the published DORA thresholds. A band on a small sample (few deployments) is indicative only.'],
  ['WIP', 'Tickets in active development at the same time, averaged over the period.'],
  ['Throughput', 'Tickets delivered per week.'],
  ['Unplanned share', 'Share of delivered work that was bugs / hotfixes / items added mid-sprint.'],
  ['Estimate accuracy', 'Actual dev time ÷ estimated time. 1.0 is on estimate; above 1 took longer.'],
  ['Estimate inflation', 'How much the estimate level for comparable tasks moved between periods. Above 1 means estimates grew, so a raw "faster than estimate" can hide a slowdown — read the adjusted pace.'],
  ['Not tracked', 'The data needed for this number does not exist for the period. It is a gap in evidence, not a zero.'],
];

// ------------------------------------------------------------ model
const maskers = new WeakMap();

function buildReportModel(input = {}) {
  const raw = obj(input);
  const mask = raw.mask === true;
  const m = mask ? makeMasker(raw) : null;
  const src = m ? m.deep(raw, '') : raw;
  const period = obj(raw.period);
  const compare = raw.compare ? obj(src.compare) : null;
  const metaIn = obj(src.meta);
  const glossaryExtra = arr(src.glossary).map((g) => [str(obj(g).term), str(obj(g).definition)]).filter(([t, d]) => t && d);
  const seen = new Set();
  const glossary = [...glossaryExtra, ...DEFAULT_GLOSSARY].filter(([t]) => (seen.has(t.toLowerCase()) ? false : (seen.add(t.toLowerCase()), true))).map(([term, definition]) => ({ term, definition }));
  const trend = normTrend(src.trend);
  const phases = normPhases(src.phases);
  const rework = normRework(src.rework);

  const model = {
    kpis: normKpis(src.kpis, compare, trend),
    trend,
    phases,
    outliers: normOutliers(src.outliers, phases, rework),
    dora: normDora(src.dora),
    pr_flow: normPrFlow(src.pr_flow),
    rework,
    flow: normFlow(src.flow),
    estimate_basis: normEstimateBasis(src.estimate_basis),
    capacity: normCapacity(src.capacity),
    substantive_subtasks: normSubtasks(src.substantive_subtasks),
    guardrails: normGuardrails(src.guardrails),
    people: normPeople(src.people, phases),
    goals: normGoals(src.goals),
    next_steps: arr(src.next_steps).map(str).filter(Boolean),
    blind_spots: arr(src.blind_spots).map(str).filter(Boolean),
    glossary,
    meta: {
      template_version: TEMPLATE_VERSION,
      title: str(metaIn.title) || 'Team performance report',
      report_kind: ['team', 'combined', 'dev'].includes(raw.report_kind) ? raw.report_kind : 'team',
      scope: str(m ? m.scrub(str(raw.scope) || '') : raw.scope) || 'Team',
      period: { start: day(period.start), end: day(period.end) },
      compare: compare ? { label: str(compare.label), start: day(obj(compare).start), end: day(obj(compare).end) } : null,
      masked: mask,
      jira_base: mask ? null : cleanBase(raw.jiraBase),
      data_as_of: iso(metaIn.data_as_of),
      scan_time: iso(metaIn.scan_time),
      ruler_version: str(metaIn.ruler_version),
      generated_at: iso(metaIn.generated_at),
      report_id: str(metaIn.report_id),
      comments_endpoint: typeof metaIn.comments_endpoint === 'string' && /^\/[^\s"'<>]*$/.test(metaIn.comments_endpoint) ? metaIn.comments_endpoint : null,
    },
  };
  model.changes = normChanges(src.changes || (src.improved || src.declined ? { improved: src.improved, declined: src.declined } : null), model.kpis);
  model.guardrails.push(...derivedGuardrails(model));
  model.next_steps.push(...derivedNextSteps(model).filter((s) => !model.next_steps.includes(s)));
  model.blind_spots.push(...derivedBlindSpots(model).filter((s) => !model.blind_spots.includes(s)));
  if (m) maskers.set(model, m);
  return model;
}

// Weak-input warnings the analytics passes may not have raised themselves.
function derivedGuardrails(model) {
  const out = [];
  const have = new Set(model.guardrails.map((g) => g.metric));
  if (model.meta.masked) out.push({ level: 'info', metric: 'mask.free_text', message: MASK_FREE_TEXT });
  const add = (metric, level, message) => {
    if (!have.has(metric)) {
      have.add(metric);
      out.push({ level, metric, message });
    }
  };
  for (const r of model.phases.rows) {
    if (r.phase === 'design' && r.not_tracked) add('phases.design', 'info', 'Design time is not tracked for most tickets in this period — the phase totals exclude it rather than count it as zero.');
    else if (!r.not_tracked && isNum(r.coverage) && r.coverage < 0.5) add(`phases.${r.phase}`, 'warn', `Only ${pct(r.coverage)} of tickets have ${r.label.toLowerCase()} evidence (${r.tickets_tracked} of ${r.tickets_total}); the ${r.label.toLowerCase()} median describes that subset only.`);
  }
  if (model.kpis.some((k) => k.kind === 'ratio' && !isNum(k.capacity_days))) add('kpis.capacity', 'warn', 'At least one ratio has no capacity figure. Read it as a rough signal, not as productivity.');
  if (model.capacity.length && model.capacity.every((c) => c.time_off_days == null)) add('capacity.time_off', 'warn', 'No time off is recorded for anyone, so capacity may be overstated.');
  const prs = model.pr_flow.prs;
  if (prs == null) add('pr_flow', 'warn', 'Pull-request data was not available, so pickup / review / merge times are not tracked.');
  else if (prs > 0 && prs < 5) add('pr_flow.sample', 'warn', `Only ${prs} pull request${prs > 1 ? 's' : ''} in the period — PR medians move a lot with one more PR.`);
  const dep = model.dora.deployments;
  if (dep === 0 || dep == null) add('dora', 'warn', 'No deployment tags were found in the period. Deployment frequency, lead time and change failure rate are not tracked.');
  else if (dep < 5) add('dora.sample', 'warn', `Only ${dep} deployment${dep > 1 ? 's' : ''} in the period — the change failure rate and DORA bands are indicative only.`);
  const ea = model.flow.estimate_accuracy;
  const hist = ea.buckets.reduce((a, b) => a + (isNum(b.count) ? b.count : 0), 0);
  if (ea.buckets.length && hist < 10) add('flow.estimate_accuracy', 'warn', `The estimate-accuracy distribution rests on ${hist} ticket${hist === 1 ? '' : 's'} — too few to read a shape from.`);
  const eb = model.estimate_basis;
  if (isNum(eb.comparable_n) && eb.comparable_n < 5) add('estimate_basis', 'warn', `Estimate inflation compares only ${eb.comparable_n} comparable task${eb.comparable_n === 1 ? '' : 's'} — treat the adjusted pace as a rough indication.`);
  return out;
}

// Rule-based follow-ups from the numbers themselves (thresholds are in the text
// so the reader can disagree with them).
function derivedNextSteps(model) {
  const s = [];
  const p = model.pr_flow;
  if (isNum(p.pickup_hours_median) && p.pickup_hours_median > 24) s.push(`PR pickup median is ${fmt(p.pickup_hours_median)} h (over 24 h): agree a review rota or a daily review slot.`);
  if (isNum(p.size_lines_median) && p.size_lines_median > 400) s.push(`Median PR size is ${fmt(p.size_lines_median, 0)} lines (over 400): split work into smaller PRs to cut review time.`);
  if (isNum(p.unreviewed_share) && p.unreviewed_share > 0.1) s.push(`${pct(p.unreviewed_share)} of PRs merged without review (over 10%): require at least one approval.`);
  const f = model.flow;
  if (isNum(f.wip_per_person) && f.wip_per_person > 2) s.push(`WIP is ${fmt(f.wip_per_person)} tickets per person (over 2): finish before starting.`);
  if (isNum(f.unplanned_share) && f.unplanned_share > 0.3) s.push(`Unplanned work is ${pct(f.unplanned_share)} of delivery (over 30%): reserve capacity for it explicitly in planning.`);
  if (isNum(model.rework.rate) && model.rework.rate > 0.15) s.push(`Rework rate is ${pct(model.rework.rate)} (over 15%): review the top rework pairs below for missing acceptance criteria or tests.`);
  if (isNum(model.dora.change_failure_rate) && model.dora.change_failure_rate > 0.15) s.push(`Change failure rate is ${pct(model.dora.change_failure_rate)} (over 15%): look at what the failures listed under DORA have in common.`);
  if (isNum(model.estimate_basis.est_inflation) && model.estimate_basis.est_inflation > 1.1) s.push(`Estimates inflated ×${fmt(model.estimate_basis.est_inflation, 2)} against the reference period: recalibrate the estimate ruler before comparing pace.`);
  const design = model.phases.rows.find((r) => r.phase === 'design');
  if (design && design.not_tracked) s.push('Design time is not tracked: log spikes / designs as sub-tasks or use a design status so the phase becomes visible.');
  return s;
}

const MASK_FREE_TEXT = 'Masking replaces known names and ticket keys everywhere and drops ticket titles. Free text (notes, reasons, comments, the summary) is scrubbed for those names and keys only — any other identifying detail written in prose is not masked. Read it before sharing.';
const STATIC_BLIND_SPOTS = [
  'Only work that leaves a trace in Jira or git is visible. Meetings, support, mentoring, interviews and on-call are not measured.',
  'Estimates are AI-generated and person-agnostic; they measure scope, not effort, and drift when the estimate ruler changes.',
  'Medians hide the spread — check the p85 and the outliers before drawing a conclusion about a whole period.',
];
function derivedBlindSpots(model) {
  const s = [...STATIC_BLIND_SPOTS];
  const nt = model.phases.rows.filter((r) => r.not_tracked).map((r) => r.label.toLowerCase());
  if (nt.length) s.push(`No evidence for ${nt.join(', ')} time in this period — those phases are left out, not counted as zero.`);
  if (model.pr_flow.prs == null) s.push('Pull-request data is missing, so review speed and depth cannot be judged.');
  if (model.dora.deployments == null || model.dora.deployments === 0) s.push('No deployment tags were found, so delivery-to-production speed and stability cannot be judged.');
  if (!isNum(model.estimate_basis.est_inflation)) s.push('Estimate inflation was not measured, so a change in "vs estimate" may be the estimates moving rather than the work.');
  if (model.meta.masked) s.push('Free text is not masked beyond known names and ticket keys — see the note at the top.');
  if (model.meta.ruler_version) s.push(`Estimates in this report use ruler ${model.meta.ruler_version}; reports on a different ruler are not directly comparable.`);
  return s;
}

// ------------------------------------------------------------ rendering
let assetCache = null;
function assets() {
  if (!assetCache) {
    const dir = path.join(__dirname, '..', 'report');
    assetCache = {
      template: fs.readFileSync(path.join(dir, 'template.html'), 'utf8'),
      css: fs.readFileSync(path.join(dir, 'report.css'), 'utf8'),
      js: fs.readFileSync(path.join(dir, 'report.js'), 'utf8'),
    };
  }
  return assetCache;
}

function makeRender(model, comments) {
  const base = model.meta.masked ? null : model.meta.jira_base;
  const tk = (k) => {
    if (k == null || k === '') return '';
    if (base && KEY_FULL.test(k)) return `<a href="${esc(`${base}/browse/${k}`)}" target="_blank" rel="noopener noreferrer">${esc(k)}</a>`;
    return esc(k);
  };
  // Free text: escape, then linkify keys (unmasked + a Jira base only).
  const text = (s) => {
    const e = esc(s == null ? '' : s);
    return base ? e.replace(KEY_RE, (k) => tk(k)) : e;
  };
  const counts = new Map();
  for (const c of comments) counts.set(c.anchor, (counts.get(c.anchor) || 0) + 1);
  const R = { tk, text, counts, kind: model.meta.report_kind };
  // A ticket row that can be commented on (anchor "t:<key>").
  R.trow = (key, cells) => (key ? { cells, anchor: ticketAnchor(key), label: String(key), btn: commentBtn(R, ticketAnchor(key), String(key), true) } : cells);
  return R;
}

const NTS = `<span class="nt">${NOT_TRACKED}</span>`;
const v = (x, unit = '', digits = 1) => (isNum(x) ? `${esc(fmt(x, digits))}${unit ? ` ${esc(unit)}` : ''}` : NTS);
const vp = (x) => (isNum(x) ? esc(pct(x)) : NTS);
const howto = (html) => `<p class="howto"><strong>How to read it.</strong> ${html}</p>`;
// Comment anchors: "section:metric" for a tile, "t:<key>" for a ticket row.
// Lower-case [a-z0-9_:-] only (sanitize.validAnchor); built from the already-
// masked model, so a masked report anchors on "t:ticket-3", never a real key.
const slug = (s) => String(s == null ? '' : s).toLowerCase().replace(/[^a-z0-9_]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 60) || 'x';
const ticketAnchor = (k) => `t:${slug(k)}`;
// A tile carries its metric slug; section() turns it into "section:metric" and
// adds the tile's comment button (it knows the section id and the counts).
const tile = (label, value, ctx, extra = '') =>
  `<div class="tile" data-tile="${esc(slug(label))}" data-anchor-label="${esc(label)}"><div class="v">${value}</div><div class="l">${esc(label)}</div>${extra}${ctx ? `<div class="c">${ctx}</div>` : ''}<!--tp:cbtn--></div>`;
// rows: arrays of cell HTML, or {cells, anchor, label, btn} from R.trow (a
// commentable ticket row: data-anchor on the <tr>, the button in its first cell).
const tableHtml = (headers, rows, numericFrom = 1) =>
  rows.length
    ? `<div class="tbl-wrap"><table><thead><tr>${headers.map((h, i) => `<th scope="col"${i >= numericFrom ? ' class="n"' : ''}>${esc(h)}</th>`).join('')}</tr></thead><tbody>${rows
        .map((r) => {
          const cells = Array.isArray(r) ? r : r.cells;
          const attrs = Array.isArray(r) || !r.anchor ? '' : ` data-anchor="${esc(r.anchor)}" data-anchor-label="${esc(r.label)}"`;
          return `<tr${attrs}>${cells.map((c, i) => `<td${i >= numericFrom ? ' class="n"' : ''}>${c}${i === 0 && !Array.isArray(r) && r.btn ? r.btn : ''}</td>`).join('')}</tr>`;
        })
        .join('')}</tbody></table></div>`
    : '<p class="muted small">Nothing in this period.</p>';
const keyTitle = (R, k, t) => R.tk(k) + (t ? ` <span class="muted">${esc(t)}</span>` : '');

const SECTION_TITLES = {
  summary: 'Summary',
  kpis: 'Key numbers',
  changes: 'What improved, what declined',
  trend: 'Trend',
  estimate_basis: 'Estimate basis',
  phases: 'Where the time goes',
  outliers: 'Outliers',
  dora: 'Delivery (DORA)',
  pr_flow: 'Pull-request flow',
  rework: 'Rework',
  flow: 'Flow & investment',
  capacity: 'Capacity',
  substantive_subtasks: 'Sub-tasks with real work',
  people: 'People',
  goals: 'Goals',
  next_steps: 'Next steps',
  blind_spots: 'Blind spots & method',
  comments: 'Comments',
  glossary: 'Glossary',
};
// Guardrail metric prefixes shown as pills on each section's header.
const SECTION_GUARDS = {
  kpis: ['kpis'],
  trend: ['trend'],
  changes: ['kpis', 'changes'],
  estimate_basis: ['estimate_basis', 'estimates', 'ruler'],
  phases: ['phases', 'cycle'],
  dora: ['dora'],
  pr_flow: ['pr_flow', 'prs'],
  rework: ['rework'],
  flow: ['flow', 'wip', 'throughput', 'investment', 'unplanned', 'estimate_accuracy'],
  capacity: ['capacity', 'time_off'],
  substantive_subtasks: ['subtasks', 'substantive_subtasks'],
  people: ['people'],
};
const matchesPrefix = (metric, prefixes) => typeof metric === 'string' && prefixes.some((p) => metric === p || metric.startsWith(`${p}.`) || metric.startsWith(`${p}_`));
const LEVEL_TEXT = { error: 'Unreliable inputs', warn: 'Read with care', info: 'Note' };
const LEVEL_ORDER = { error: 0, warn: 1, info: 2 };
const ICONS = {
  warn: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1.5 15 14H1L8 1.5Zm-.75 4.5v4h1.5V6h-1.5Zm0 5v1.5h1.5V11h-1.5Z"/></svg>',
  error: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1a7 7 0 1 1 0 14A7 7 0 0 1 8 1Zm-.75 3.5v5h1.5v-5h-1.5Zm0 6v1.5h1.5V10.5h-1.5Z"/></svg>',
  info: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1a7 7 0 1 1 0 14A7 7 0 0 1 8 1Zm-.75 6v5h1.5V7h-1.5Zm0-2.5V6h1.5V4.5h-1.5Z"/></svg>',
};

function sectionPills(model, id) {
  const prefixes = SECTION_GUARDS[id];
  if (!prefixes) return '';
  const hits = model.guardrails.map((g, i) => ({ g, i })).filter(({ g }) => matchesPrefix(g.metric, prefixes));
  if (!hits.length) return '';
  const worst = hits.map(({ g }) => g.level).sort((a, b) => LEVEL_ORDER[a] - LEVEL_ORDER[b])[0];
  const first = hits.find(({ g }) => g.level === worst);
  const tip = hits.map(({ g }) => g.message).join(' ');
  return `<a class="pill gr ${worst}" href="#gr-${first.i}" title="${esc(tip)}">${ICONS[worst]}${esc(LEVEL_TEXT[worst])}${hits.length > 1 ? ` · ${hits.length}` : ''}</a>`;
}

const teamPill = (R) => (R.kind === 'dev' ? '<span class="pill team" title="Team-level numbers shown for context. They describe the delivery system, not this person.">Team context (not individual)</span>' : '');

function commentBtn(R, id, label, mini = false) {
  const n = R.counts.get(id) || 0;
  const text = mini ? (n ? String(n) : '+') : n ? `Comments (${n})` : 'Comment';
  return `<button type="button" class="c-btn js-only${mini ? ' mini' : ''}${n ? ' has' : ''}" data-for="${esc(id)}" aria-label="Comment on ${esc(label)}${n ? ` (${n})` : ''}" title="Comment on ${esc(label)}">${text}</button>`;
}
const unesc = (s) => String(s).replace(/&#39;/g, "'").replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');

function section(model, R, id, title, body, extra = '') {
  const sid = esc(id);
  // Tiles become "section:metric" comment anchors with their own button.
  const withTiles = body.replace(/<div class="tile" data-tile="([a-z0-9_-]+)" data-anchor-label="([^"]*)">([\s\S]*?)<!--tp:cbtn-->/g, (all, metric, label, inner) => {
    const anchor = `${slug(id)}:${metric}`;
    const lbl = `${title} · ${unesc(label)}`;
    return `<div class="tile" data-anchor="${esc(anchor)}" data-anchor-label="${esc(lbl)}">${inner}${commentBtn(R, anchor, lbl, true)}`;
  });
  return `<section class="card" id="${sid}" data-anchor="${sid}" data-anchor-label="${esc(title)}" aria-labelledby="${sid}-h"><header><h2 id="${sid}-h">${esc(title)}<a class="anchor" href="#${sid}" aria-label="Link to ${esc(title)}" title="Link to this section">#</a></h2>${extra}${sectionPills(model, id)}${commentBtn(R, id, title)}</header>${withTiles}</section>`;
}

function trendFor(model, id) {
  return model.trend.find((t) => t.id === id) || null;
}

function renderKpis(model, R) {
  const cards = model.kpis.map((k) => {
    let ctx = '';
    if (k.kind === 'ratio') ctx += isNum(k.capacity_days) ? `over ${esc(fmt(k.capacity_days))} capacity days` : '<span class="pill warn">capacity unknown — not a productivity figure</span>';
    if (isNum(k.delta)) {
      const dir = k.delta === 0 || !k.better ? 'flat' : (k.delta > 0) === (k.better === 'up') ? 'better' : 'worse';
      ctx += `${ctx ? ' · ' : ''}<span class="delta ${dir}">${k.delta > 0 ? '+' : ''}${esc(fmt(k.kind === 'percent' ? k.delta * 100 : k.delta))}${k.kind === 'percent' ? ' pp' : ''} vs ${esc(model.meta.compare?.label || 'previous period')}</span>`;
    }
    if (k.note) ctx += `${ctx ? ' · ' : ''}${R.text(k.note)}`;
    const val = k.kind === 'percent' ? vp(k.value) : v(k.value, k.unit);
    const spark = C.sparkline({ values: k.kind === 'percent' ? k.history.map((x) => (isNum(x) ? x * 100 : x)) : k.history, labels: k.history_labels, label: `${k.label} over recent periods`, unit: k.kind === 'percent' ? '%' : k.unit });
    return tile(k.label, val, ctx, spark);
  });
  return section(
    model,
    R,
    'kpis',
    SECTION_TITLES.kpis,
    howto('Each tile is one number for the whole scope. A ratio is always shown with the capacity it was divided by — a lower ratio in a period with less capacity is not lower productivity. The change is against the comparison period when one was chosen; the small line shows recent periods.') +
      (cards.length ? `<div class="grid">${cards.join('')}</div>` : '<p class="muted small">No headline numbers were computed for this period.</p>'),
  );
}

function renderChanges(model, R) {
  const ch = model.changes || { improved: [], declined: [] };
  if (!model.meta.compare && !ch.improved.length && !ch.declined.length) return '';
  const fmtV = (x, kind, unit) => (kind === 'percent' ? vp(x) : v(x, unit));
  const deltaTxt = (x) => {
    if (!isNum(x.from) || !isNum(x.to)) return NTS;
    const d = x.to - x.from;
    const abs = x.kind === 'percent' ? `${d > 0 ? '+' : ''}${fmt(d * 100)} pp` : `${d > 0 ? '+' : ''}${fmt(d)}${x.unit ? ` ${x.unit}` : ''}`;
    const rel = x.from !== 0 && x.kind !== 'percent' ? ` (${d > 0 ? '+' : ''}${fmt((d / Math.abs(x.from)) * 100, 0)}%)` : '';
    return esc(abs + rel);
  };
  const list = (title, cls, items) =>
    `<div class="olist"><h3>${esc(title)}</h3>${
      items.length
        ? tableHtml(['Measure', 'Before', 'Now', 'Change'], items.map((x) => [`${esc(x.label)}${x.note ? `<div class="small muted">${R.text(x.note)}</div>` : ''}`, fmtV(x.from, x.kind, x.unit), fmtV(x.to, x.kind, x.unit), `<span class="delta ${cls}">${deltaTxt(x)}</span>`]))
        : '<p class="muted small">Nothing moved by more than 2% in this direction.</p>'
    }</div>`;
  const vs = model.meta.compare ? model.meta.compare.label || `${model.meta.compare.start || '…'} → ${model.meta.compare.end || '…'}` : 'the comparison period';
  return section(
    model,
    R,
    'changes',
    SECTION_TITLES.changes,
    howto(`Each measure against ${esc(vs)}, sorted by how much it moved. "Improved" follows the measure's own direction (shorter lead time is better). A ratio moves with capacity — check the capacity line before reading it as a change in productivity. Measures whose inputs are weak carry a pill on their section.`) +
      `<div class="grid wide">${list('Improved', 'better', ch.improved)}${list('Declined', 'worse', ch.declined)}</div>`,
  );
}

function renderTrend(model, R) {
  if (!model.trend.length) {
    return section(model, R, 'trend', SECTION_TITLES.trend, howto('Recent periods side by side, oldest first.') + '<p class="muted small">No earlier periods were computed for this report, so there is no trend yet.</p>');
  }
  const labels = [];
  for (const t of model.trend) for (const p of t.points) if (!labels.includes(p.label)) labels.push(p.label);
  const rows = model.trend.map((t) => {
    const at = new Map(t.points.map((p) => [p.label, p.value]));
    const vals = labels.map((l) => (at.has(l) ? at.get(l) : null));
    const pctKind = t.kind === 'percent';
    const show = (x) => (pctKind ? vp(x) : v(x, t.unit));
    const nums = t.points.map((p) => p.value).filter(isNum);
    let dir = '';
    if (nums.length >= 2 && t.better) {
      const d = nums[nums.length - 1] - nums[0];
      const cls = d === 0 ? 'flat' : (d > 0) === (t.better === 'up') ? 'better' : 'worse';
      dir = ` <span class="delta ${cls}">${d === 0 ? 'flat' : cls === 'better' ? 'improving' : 'worsening'}</span>`;
    }
    return [esc(t.label) + dir, ...vals.map(show), C.sparkline({ values: pctKind ? vals.map((x) => (isNum(x) ? x * 100 : x)) : vals, labels, label: t.label, unit: pctKind ? '%' : t.unit }) || NTS];
  });
  return section(
    model,
    R,
    'trend',
    SECTION_TITLES.trend,
    howto('The same measure over the last periods, oldest first. One period is noise; look for a direction held over three or more. "Improving" / "worsening" compares the first and the last period only.') + tableHtml(['Measure', ...labels, 'Trend'], rows),
  );
}

function renderEstimateBasis(model, R) {
  const e = model.estimate_basis;
  const inflated = isNum(e.est_inflation) && e.est_inflation > 1.05;
  const tiles = [
    tile('Estimate inflation', isNum(e.est_inflation) ? `×${esc(fmt(e.est_inflation, 2))}` : NTS, `${isNum(e.comparable_n) ? `${esc(fmt(e.comparable_n, 0))} comparable tasks` : 'comparable tasks unknown'}${e.ref_label ? ` · vs ${esc(e.ref_label)}` : ''}`),
    tile('Pace vs estimate (raw)', v(e.pace_raw, '×', 2), 'actual ÷ estimate, this period'),
    tile('Pace on reference basis', v(e.pace_adjusted, '×', 2), 'raw pace × inflation'),
    tile('Reference pace', v(e.pace_ref, '×', 2), e.ref_label ? esc(e.ref_label) : 'comparison period'),
  ];
  const verdict = inflated
    ? `<p class="callout warn">${ICONS.warn}<span>Estimates grew ×${esc(fmt(e.est_inflation, 2))} for comparable work. Compare <strong>pace on reference basis</strong> with the reference pace — a raw improvement with inflated estimates can be a decline.</span></p>`
    : '';
  return section(
    model,
    R,
    'estimate_basis',
    SECTION_TITLES.estimate_basis,
    howto('Estimates are a ruler. If the ruler stretches between periods (the same kind of task is estimated larger), work looks faster against it without being faster. Inflation compares estimates for comparable tasks; the adjusted pace puts this period on the reference period\'s ruler.') +
      verdict +
      `<div class="grid">${tiles.join('')}</div>${e.note ? `<p class="small">${R.text(e.note)}</p>` : ''}`,
  );
}

function renderPhases(model, R) {
  const P = model.phases;
  const series = PHASES.map(([, l]) => l);
  const teamRows = P.by_period.length ? P.by_period.map((p) => ({ label: p.label, values: PHASES.map(([id]) => p[id]) })) : [{ label: 'This period (medians)', values: P.rows.map((r) => r.median_days) }];
  const chart = C.stackedBar({
    id: 'ch-phases',
    title: 'Team cycle time by phase (median business days)',
    desc: `Median business days in each phase, stacked. Not tracked: ${P.rows.filter((r) => r.not_tracked).map((r) => r.label).join(', ') || 'none'}.`,
    series,
    rows: teamRows,
    unit: 'd',
    classes: PHASE_CLASSES,
  });
  const rows = P.rows.map((r) => [esc(r.label), v(r.median_days, 'd'), v(r.p85_days, 'd'), v(r.total_days, 'd'), isNum(r.tickets_tracked) && isNum(r.tickets_total) ? `${r.tickets_tracked} / ${r.tickets_total} (${esc(pct(r.coverage))})` : NTS]);
  const splits = P.splits.length ? `<h3>Phase detail</h3>${tableHtml(['Part', 'Median', 'Total'], P.splits.map((s) => [esc(s.label || s.phase), v(s.median_days, 'd'), v(s.total_days, 'd')]))}` : '';
  const tix = P.tickets.length
    ? `<details class="as-table"><summary>Tickets (${P.tickets.length})</summary>${tableHtml(
        ['Ticket', 'Person', ...PHASES.map(([, l]) => l)],
        P.tickets.map((t) => R.trow(t.key, [keyTitle(R, t.key, t.title), esc(t.person || ''), ...PHASES.map(([id]) => v(t[id], 'd'))])),
        2,
      )}</details>`
    : '';
  return section(
    model,
    R,
    'phases',
    SECTION_TITLES.phases,
    howto('Ticket time is split into design, dev, review, deployment and rework, in business days. Colours are fixed per phase across the report. <em>Coverage</em> is how many tickets had evidence for the phase; "not tracked" means no evidence, not zero time. Medians resist one long ticket; p85 shows the slow tail. Stacked medians are not a ticket\'s total cycle time — each phase\'s median comes from its own tickets.') +
      chart +
      tableHtml(['Phase', 'Median', 'p85', 'Total', 'Coverage'], rows) +
      splits +
      tix,
  );
}

function renderOutliers(model, R) {
  const o = model.outliers;
  const list = (title, items, digits = 1) =>
    `<div class="olist"><h3>${esc(title)}</h3>${
      items.length
        ? `<ol>${items.map((x) => `<li${x.key ? ` data-anchor="${esc(ticketAnchor(x.key))}" data-anchor-label="${esc(x.key)}"` : ''}><span class="ov">${v(x.value, x.unit, digits)}</span> ${keyTitle(R, x.key, x.title)}${x.person ? ` <span class="muted">· ${esc(x.person)}</span>` : ''}${x.key ? commentBtn(R, ticketAnchor(x.key), x.key, true) : ''}${x.note ? `<div class="small muted">${R.text(x.note)}</div>` : ''}</li>`).join('')}</ol>`
        : '<p class="muted small">Nothing stands out — or the data for it is not tracked.</p>'
    }</div>`;
  return section(
    model,
    R,
    'outliers',
    SECTION_TITLES.outliers,
    howto('The few tickets that move the medians most. An outlier is a question to ask, not a verdict: long dev time is often a ticket that should have been split, and a big estimate deviation is often a mis-scoped ticket rather than slow work.') +
      `<div class="grid wide">${list('Longest dev time', o.dev)}${list('Most rework', o.rework)}${list('Largest estimate deviation (actual ÷ estimate)', o.estimate, 2)}${list('Jira-linked rework & follow-ups', o.jira_linked)}</div>`,
  );
}

// Published DORA bands. Lower is better for lead time / CFR / restore.
const DORA_BANDS = {
  deploy: (x) => (x >= 7 ? 'elite' : x >= 1 ? 'high' : x >= 0.25 ? 'medium' : 'low'),
  lead: (x) => (x < 1 ? 'elite' : x <= 7 ? 'high' : x <= 30 ? 'medium' : 'low'),
  cfr: (x) => (x <= 0.05 ? 'elite' : x <= 0.1 ? 'high' : x <= 0.15 ? 'medium' : 'low'),
  mttr: (x) => (x < 1 ? 'elite' : x < 24 ? 'high' : x < 168 ? 'medium' : 'low'),
};
const bandPill = (kind, x) => (isNum(x) ? `<span class="pill band ${DORA_BANDS[kind](x)}">${DORA_BANDS[kind](x)}</span>` : '');

function renderDora(model, R) {
  const d = model.dora;
  const spark = (id, label, unit, fallback) => {
    const t = trendFor(model, id);
    if (t) return C.sparkline({ values: t.points.map((p) => (t.kind === 'percent' ? (isNum(p.value) ? p.value * 100 : null) : p.value)), labels: t.points.map((p) => p.label), label, unit: t.kind === 'percent' ? '%' : unit });
    return fallback || '';
  };
  const weeklySpark = d.weekly.length >= 2 ? C.sparkline({ values: d.weekly.map((w) => w.deployments), labels: d.weekly.map((w) => w.week), label: 'Deployments per week', digits: 0 }) : '';
  const nOfM = isNum(d.failed_deployments) && isNum(d.deployments) ? `${esc(fmt(d.failed_deployments, 0))} of ${esc(fmt(d.deployments, 0))} deployments failed` : 'hotfix or bug after deploy';
  const tiles = [
    tile('Deployment frequency', v(d.deploy_frequency_per_week, '/ week'), `${bandPill('deploy', d.deploy_frequency_per_week)} ${isNum(d.deployments) ? `${esc(fmt(d.deployments, 0))} deployments${isNum(d.hotfixes) ? `, ${esc(fmt(d.hotfixes, 0))} hotfix` : ''}` : ''}`, spark('deploy_frequency', 'Deployment frequency', '/wk', weeklySpark)),
    tile('Lead time for changes', v(d.lead_time_days_median, 'd'), `${bandPill('lead', d.lead_time_days_median)} ${isNum(d.lead_time_days_p85) ? `p85 ${esc(fmt(d.lead_time_days_p85))} d` : 'median'}`, spark('lead_time', 'Lead time', 'd')),
    tile('Change failure rate', vp(d.change_failure_rate), `${bandPill('cfr', d.change_failure_rate)} ${nOfM}`, spark('change_failure_rate', 'Change failure rate', '%')),
    tile('Time to restore', v(d.mttr_hours_median, 'h'), `${bandPill('mttr', d.mttr_hours_median)} ${isNum(d.incidents) ? `median of ${esc(fmt(d.incidents, 0))} incident${d.incidents === 1 ? '' : 's'}` : 'median'}`, spark('mttr', 'Time to restore', 'h')),
  ];
  const chart = d.weekly.length ? C.columnChart({ id: 'ch-deploys', title: 'Deployments per week', desc: `Deployment tags per week, ${d.weekly.length} weeks.`, points: d.weekly.map((w) => ({ label: w.week, value: w.deployments })), digits: 0 }) : '';
  const tags = d.deploy_tags.length
    ? `<details class="as-table"><summary>Deployment tags (${d.deploy_tags.length}${d.deploy_tags.some((t) => t.kind === 'hotfix') ? `, ${d.deploy_tags.filter((t) => t.kind === 'hotfix').length} hotfix` : ''})</summary>${tableHtml(
        ['Tag', 'Repository', 'When', 'Kind', 'Tickets'],
        d.deploy_tags.map((t) => [`<code>${esc(t.name)}</code>`, esc(t.repo || ''), t.at ? `<time datetime="${esc(t.at)}">${esc(t.at.slice(0, 16).replace('T', ' '))}</time>` : NTS, t.kind === 'hotfix' ? '<span class="pill warn">hotfix</span>' : 'deploy', t.keys.map((k) => R.tk(k)).join(', ')]),
        5,
      )}</details>`
    : '';
  const fails = d.failures.length ? `<h3>Failures</h3>${tableHtml(['Ticket', 'Kind', 'Tag', 'Caused by', 'Restore'], d.failures.map((f) => R.trow(f.key, [keyTitle(R, f.key, f.title), esc(f.kind || ''), esc(f.tag || ''), R.tk(f.caused_by), v(f.restore_hours, 'h')])), 4)}` : '';
  return section(
    model,
    R,
    'dora',
    SECTION_TITLES.dora,
    howto('A deployment is a git tag whose name contains "deployed", "hf" or "hotfix" (hotfixes are deployments too). Change failure rate counts deployments followed by a hotfix or a bug traced to the change. Bands follow the published DORA thresholds. These are team numbers — they describe the delivery system, not any one person.') +
      `<div class="grid">${tiles.join('')}</div>${chart}${tags}${fails}`,
    teamPill(R),
  );
}

function renderPrFlow(model, R) {
  const p = model.pr_flow;
  const tiles = [
    tile('Pull requests', v(p.prs, '', 0), isNum(p.merged) ? `${esc(fmt(p.merged, 0))} merged` : ''),
    tile('Pickup time', v(p.pickup_hours_median, 'h'), 'opened → first review'),
    tile('Review time', v(p.review_hours_median, 'h'), 'first review → approved'),
    tile('Merge time', v(p.merge_hours_median, 'h'), 'opened → merged'),
    tile('PR size', v(p.size_lines_median, 'lines', 0), 'median changed lines'),
    tile('Review depth', v(p.comments_per_pr, 'comments'), isNum(p.reviews_per_pr) ? `${esc(fmt(p.reviews_per_pr))} reviews / PR` : ''),
  ];
  const strip = p.items.some((x) => isNum(x.pickup_hours) || isNum(x.review_hours))
    ? C.stripPlot({
        id: 'ch-pr-strip',
        title: 'Pickup and review time, one tick per PR (hours)',
        desc: 'Each tick is one pull request; the taller mark is the median. Ticks at the right edge are beyond the axis.',
        rows: [
          { label: 'Pickup', values: p.items.map((x) => x.pickup_hours) },
          { label: 'Review', values: p.items.map((x) => x.review_hours) },
          ...(p.items.some((x) => isNum(x.merge_hours)) ? [{ label: 'Open → merged', values: p.items.map((x) => x.merge_hours) }] : []),
        ],
        unit: 'h',
      })
    : '';
  const total = (x) => x.merge_hours ?? (isNum(x.pickup_hours) || isNum(x.review_hours) ? (x.pickup_hours || 0) + (x.review_hours || 0) : null);
  const slowList = (p.slow || p.items.filter((x) => isNum(total(x))).sort((a, b) => total(b) - total(a)).slice(0, 8)).slice(0, 15);
  const prName = (x) => `${x.pr ? `#${esc(x.pr)}` : 'PR'}${x.repo ? ` <span class="muted">${esc(x.repo)}</span>` : ''}${x.title ? `<div class="small muted">${esc(x.title)}</div>` : ''}`;
  const slow = slowList.length
    ? `<h3>Slowest pull requests</h3>${tableHtml(
        ['Pull request', 'Ticket', 'Author', 'Pickup', 'Review', 'Open → merged', 'Size', 'Comments'],
        slowList.map((x) => R.trow(x.key, [prName(x), R.tk(x.key), esc(x.person || ''), v(x.pickup_hours, 'h'), v(x.review_hours, 'h'), v(x.merge_hours, 'h'), v(x.size_lines, '', 0), v(x.comments, '', 0)])),
        3,
      )}`
    : '';
  const by = p.by_person.length
    ? `<h3>By person</h3>${tableHtml(['Person', 'PRs', 'Reviews given', 'Pickup (median)', 'Size (median)'], p.by_person.map((x) => [esc(x.person || ''), v(x.prs, '', 0), v(x.reviews_given, '', 0), v(x.pickup_hours_median, 'h'), v(x.size_lines_median, '', 0)]))}`
    : '';
  return section(
    model,
    R,
    'pr_flow',
    SECTION_TITLES.pr_flow,
    howto(`Pickup is how long a PR waits for its first reviewer; long pickup usually means reviewers are overloaded, not that authors are slow. Small PRs review faster. The tick plot shows every PR so a few slow ones are not hidden behind a median. ${isNum(p.unreviewed_share) ? `${esc(pct(p.unreviewed_share))} of PRs merged without a review.` : ''}${p.coverage_note ? ` ${R.text(p.coverage_note)}` : ''}`) +
      `<div class="grid">${tiles.join('')}</div>${strip}${slow}${by}`,
    teamPill(R),
  );
}

function renderRework(model, R) {
  const r = model.rework;
  // Rework flow: group "origin ← later ticket" pairs by origin ticket.
  const byOrigin = new Map();
  for (const x of r.in) {
    const k = x.key || '—';
    if (!byOrigin.has(k)) byOrigin.set(k, []);
    byOrigin.get(k).push(x);
  }
  const flows = [...byOrigin.entries()]
    .map(([origin, list]) => ({ origin, list, days: list.reduce((a, x) => a + (isNum(x.days) ? x.days : 0), 0), lines: list.reduce((a, x) => a + (isNum(x.lines) ? x.lines : 0), 0) }))
    .sort((a, b) => b.days - a.days || b.lines - a.lines)
    .slice(0, 12);
  const flowHtml = flows.length
    ? `<ul class="rflow">${flows
        .map(
          (f) =>
            `<li><div class="rf-origin">${R.tk(f.origin)}${f.list[0].person ? ` <span class="muted">· ${esc(f.list[0].person)}</span>` : ''}</div><div class="rf-arrow" aria-hidden="true">←</div><div class="rf-by"><span class="sr-only">reworked by </span>${f.list
              .map((x) => `<span class="rf-item">${R.tk(x.by_key) || '<span class="muted">unknown ticket</span>'}${isNum(x.days) ? ` <span class="muted">${esc(fmt(x.days))} d</span>` : ''}${isNum(x.lines) ? ` <span class="muted">${esc(fmt(x.lines, 0))} lines</span>` : ''}</span>`)
              .join('')}</div></li>`,
        )
        .join('')}</ul>`
    : '<p class="muted small">No code-level rework pairs in this period.</p>';
  const pairRows = (list) => list.map((x) => R.trow(x.key, [R.tk(x.key), R.tk(x.by_key), esc(x.person || ''), v(x.lines, '', 0), v(x.days, 'd')]));
  return section(
    model,
    R,
    'rework',
    SECTION_TITLES.rework,
    howto('<em>Rework in</em>: recent code of the original ticket (left) was rewritten by a later ticket (right) — that time is charged back to the original. <em>Rework out</em>: a ticket rewrote someone else\'s recent code. Jira rework tickets (fixes, follow-ups, reopens) are listed below; their estimates do not count as new delivered scope.') +
      `<div class="grid">${tile('Rework rate', vp(r.rate), 'share of changed lines that rewrote recent code')}${tile('Days charged back', v(r.days_charged, 'd'), 'to original tickets')}${tile('Jira rework tickets', v(r.jira.length, '', 0), `${r.jira.filter((x) => x.excluded_from_scope).length} excluded from scope`)}</div>` +
      `<h3>Rework flow (original ← later ticket)</h3>${flowHtml}` +
      (r.out.length ? `<h3>Rework out</h3>${tableHtml(['Ticket', 'Rewrote', 'Person', 'Lines', 'Days'], pairRows(r.out), 3)}` : '') +
      `<h3>Jira-detected rework</h3>${tableHtml(
        ['Ticket', 'Reworks', 'Why', 'Points', 'Scope'],
        r.jira.map((x) => R.trow(x.key, [keyTitle(R, x.key, x.title), R.tk(x.source_key), R.text(x.reason || ''), v(x.points, '', 1), x.excluded_from_scope ? 'not new scope' : 'counted'])),
        3,
      )}`,
  );
}

function renderFlow(model, R) {
  const f = model.flow;
  const tiles = [
    tile('WIP', v(f.wip_avg), isNum(f.wip_per_person) ? `${esc(fmt(f.wip_per_person))} per person` : 'average in progress'),
    tile('Unplanned work', vp(f.unplanned_share), isNum(f.unplanned_share) && f.unplanned.length ? `${f.unplanned.length} tickets listed below` : 'bugs, hotfixes, mid-sprint adds'),
    tile('Context switching', v(f.context_switch_avg), 'tickets touched per person-day'),
    tile('Estimate accuracy', v(f.estimate_accuracy.median_ratio, '×', 2), isNum(f.estimate_accuracy.within_band_share) ? `${esc(pct(f.estimate_accuracy.within_band_share))} within band` : 'actual ÷ estimate, median'),
  ];
  const charts = [
    f.throughput.length ? C.columnChart({ id: 'ch-throughput', title: 'Tickets delivered per week', desc: `Throughput over ${f.throughput.length} weeks.`, points: f.throughput.map((w) => ({ label: w.week, value: w.count })), digits: 0 }) : '',
    f.investment.length ? C.barChart({ id: 'ch-invest', title: 'Investment mix by ticket type (share of delivered work)', desc: 'Share of delivered work per ticket type.', rows: f.investment.map((x) => ({ label: x.label, value: isNum(x.share) ? x.share * 100 : null })), unit: '%', digits: 0, series: 2 }) : '',
    f.estimate_accuracy.buckets.length ? C.barChart({ id: 'ch-accuracy', title: 'Estimate accuracy distribution (tickets per band)', desc: 'Number of tickets per actual-to-estimate band.', rows: f.estimate_accuracy.buckets.map((b) => ({ label: b.label, value: b.count })), digits: 0, series: 4 }) : '',
  ].join('');
  const epics = f.investment_by_epic.length
    ? `<h3>Investment by epic</h3>${tableHtml(
        ['Epic', 'Share', 'Points', 'Tickets'],
        f.investment_by_epic.slice().sort((a, b) => (b.share ?? -1) - (a.share ?? -1)).map((x) => [x.epic_key ? keyTitle(R, x.epic_key, x.epic_title) : `<span class="muted">${esc(x.epic_title || 'No epic')}</span>`, vp(x.share), v(x.points), v(x.tickets, '', 0)]),
      )}`
    : '';
  const unplanned = f.unplanned.length
    ? `<h3>Unplanned work</h3>${tableHtml(['Ticket', 'Type', 'Person', 'Why unplanned', 'Points'], f.unplanned.map((x) => R.trow(x.key, [keyTitle(R, x.key, x.title), esc(x.type || ''), esc(x.person || ''), R.text(x.reason || ''), v(x.points)])), 4)}`
    : '';
  return section(
    model,
    R,
    'flow',
    SECTION_TITLES.flow,
    howto('High WIP and frequent context switching slow everything down even when everyone is busy. Investment mix shows where effort went; a large unplanned share means planned work was squeezed. Estimate accuracy above 1× means work took longer than estimated.') +
      `<div class="grid">${tiles.join('')}</div>${charts}${epics}${unplanned}`,
  );
}

function renderCapacity(model, R) {
  const rows = model.capacity.map((c) => [esc(c.person || ''), v(c.working_days, 'd'), v(c.time_off_days, 'd'), v(c.capacity_days, 'd'), v(c.dev_days, 'd'), v(c.delivered_points), isNum(c.points_per_capacity_day) ? `${esc(fmt(c.points_per_capacity_day, 2))} <span class="muted">over ${v(c.capacity_days, 'd')}</span>` : NTS]);
  return section(
    model,
    R,
    'capacity',
    SECTION_TITLES.capacity,
    howto('Capacity days = working days minus recorded time off. Points per capacity day is a pace signal for planning, not a performance score: it ignores reviews, support, mentoring and design work, and swings with how tickets were sized.') +
      tableHtml(['Person', 'Working', 'Time off', 'Capacity', 'Dev time', 'Points', 'Points / capacity day'], rows),
  );
}

function renderSubtasks(model, R) {
  const groups = new Map();
  for (const s of model.substantive_subtasks) {
    const k = s.person || 'Unassigned';
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(s);
  }
  const body = groups.size
    ? [...groups.entries()]
        .map(([person, list]) => {
          const days = list.reduce((a, s) => a + (isNum(s.dev_days) ? s.dev_days : 0), 0);
          const commits = list.reduce((a, s) => a + (isNum(s.commits) ? s.commits : 0), 0);
          return `<h3>${esc(person)} <span class="muted small">· ${list.length} sub-task${list.length > 1 ? 's' : ''}, ${esc(fmt(days))} d dev time, ${esc(fmt(commits, 0))} commits credited</span></h3>${tableHtml(
            ['Sub-task', 'Parent', 'Why it counts', 'Dev time', 'Commits'],
            list.map((s) => R.trow(s.key, [keyTitle(R, s.key, s.title), R.tk(s.parent_key) + (s.parent_owner ? ` <span class="muted">(${esc(s.parent_owner)})</span>` : ''), R.text(s.reason || ''), v(s.dev_days, 'd'), v(s.commits, '', 0)])),
            3,
          )}`;
        })
        .join('')
    : '<p class="muted small">No sub-task carried real work in this period.</p>';
  return section(
    model,
    R,
    'substantive_subtasks',
    SECTION_TITLES.substantive_subtasks,
    howto('Most sub-tasks are checklists and are ignored. These ones carried real work — their own dev time or commits, under someone else\'s story, or under a story with no timing of its own — so the work is credited to the person who did it.') + body,
  );
}

function renderPeople(model, R) {
  if (!model.people.length) return section(model, R, 'people', SECTION_TITLES.people, '<p class="muted small">No per-person breakdown in this report.</p>');
  const teamMed = Object.fromEntries(model.phases.rows.map((r) => [r.phase, r.median_days]));
  const anyPhase = model.people.some((p) => PHASES.some(([id]) => isNum(p.phase_days[id])));
  const phaseChart = anyPhase
    ? C.stackedBar({
        id: 'ch-people-phases',
        title: 'Phase medians per person vs team (business days)',
        desc: 'Median business days per phase for each person, with the team median as the first row.',
        series: PHASES.map(([, l]) => l),
        rows: [{ label: 'Team median', values: PHASES.map(([id]) => teamMed[id]) }, ...model.people.map((p) => ({ label: p.name || '—', values: PHASES.map(([id]) => p.phase_days[id]) }))],
        unit: 'd',
        classes: PHASE_CLASSES,
      })
    : '';
  const vsRows = anyPhase
    ? model.people.map((p) => [
        esc(p.name || '—'),
        ...PHASES.map(([id]) => {
          const a = p.phase_days[id];
          const t = teamMed[id];
          if (!isNum(a)) return NTS;
          if (!isNum(t) || t <= 0) return esc(`${fmt(a)} d`);
          const r = a / t;
          return `${esc(fmt(a))} d <span class="delta ${r > 1.25 ? 'worse' : r < 0.8 ? 'better' : 'flat'}">×${esc(fmt(r, 2))}</span>`;
        }),
      ])
    : [];
  const subsBy = new Map();
  for (const s of model.substantive_subtasks) subsBy.set(s.person, (subsBy.get(s.person) || 0) + 1);
  const cards = model.people.map((p, i) => {
    const id = `person-${i + 1}`;
    const label = p.name || id;
    const kv = [
      ['Capacity', `${v(p.capacity_days, 'd')}${isNum(p.time_off_days) ? ` <span class="muted">(${esc(fmt(p.time_off_days))} d off)</span>` : ''}`],
      ['Delivered', `${v(p.delivered_points, 'pts')}${isNum(p.points_per_capacity_day) ? ` <span class="muted">· ${esc(fmt(p.points_per_capacity_day, 2))} / capacity day</span>` : ''}`],
      ['Cycle time (median)', v(p.cycle_time_days_median, 'd')],
      ['Design time', v(p.design_days, 'd')],
      ['Dev time', v(p.dev_days, 'd')],
      ['PRs / reviews given', `${v(p.prs, '', 0)} / ${v(p.reviews_given, '', 0)}`],
      ['Rework in / out (lines)', `${v(p.rework_in_lines, '', 0)} / ${v(p.rework_out_lines, '', 0)}`],
      ['Estimate accuracy', v(p.estimate_ratio_median, '×', 2)],
      ['Credited sub-tasks', subsBy.get(p.name) ? `<a href="#substantive_subtasks">${subsBy.get(p.name)}</a>` : '0'],
    ];
    const tix = p.tickets.length ? `<details class="as-table"><summary>Tickets (${p.tickets.length})</summary>${tableHtml(['Ticket', 'Status', 'Points', 'Dev time'], p.tickets.map((t) => R.trow(t.key, [keyTitle(R, t.key, t.title), esc(t.status || ''), v(t.points), v(t.dev_days, 'd')])), 2)}</details>` : '';
    const notes = p.notes.length ? `<ul class="small">${p.notes.map((n) => `<li>${R.text(n)}</li>`).join('')}</ul>` : '';
    return `<div class="person" id="${id}" data-anchor="${id}" data-anchor-label="${esc(label)}"><header><h3>${esc(label)}${p.role ? ` <span class="pill">${esc(p.role)}</span>` : ''}</h3>${commentBtn(R, id, label)}</header><dl class="kv">${kv.map(([k, x]) => `<dt>${esc(k)}</dt><dd>${x}</dd>`).join('')}</dl>${notes}${tix}</div>`;
  });
  return section(
    model,
    R,
    'people',
    SECTION_TITLES.people,
    howto('One card per person. Every figure sits next to the capacity it came from; compare people only with that context, and never on points alone. The phase chart compares each person\'s medians with the team\'s — ×1.25 or more is marked slower, ×0.8 or less faster; a handful of tickets can explain either.') +
      phaseChart +
      (vsRows.length ? `<details class="as-table"><summary>Versus team median, per phase</summary>${tableHtml(['Person', ...PHASES.map(([, l]) => l)], vsRows)}</details>` : '') +
      `<div class="grid wide">${cards.join('')}</div>`,
  );
}

function renderGoals(model, R, narrativeGoals) {
  const prev = model.goals.length
    ? tableHtml(
        ['Goal', 'Target', 'Actual', 'Status'],
        model.goals.map((g) => [R.text(g.goal) + (g.note ? `<div class="small muted">${R.text(g.note)}</div>` : ''), esc(g.target || ''), esc(g.actual || ''), g.met === true ? '<span class="pill ok">met</span>' : g.met === false ? '<span class="pill warn">not met</span>' : NTS]),
        1,
      )
    : '<p class="muted small">No goals were recorded for the previous period.</p>';
  const next = narrativeGoals.length ? `<h3>Proposed for next period</h3><ul>${narrativeGoals.map((x) => `<li>${R.text(x)}</li>`).join('')}</ul><p class="muted small">Drafted by an agent from the numbers in this report — agree them with the person before adopting.</p>` : '';
  return section(model, R, 'goals', SECTION_TITLES.goals, howto('Last period\'s goals with what actually happened, then proposed goals for the next period. A missed goal with weak inputs (see the pills) is a measurement question first.') + `<h3>Previous period</h3>${prev}${next}`);
}

function renderList(model, R, id, intro, items, empty) {
  return section(model, R, id, SECTION_TITLES[id], howto(intro) + (items.length ? `<ul class="steps">${items.map((x) => `<li>${R.text(x)}</li>`).join('')}</ul>` : `<p class="muted small">${esc(empty)}</p>`));
}

function renderBlindSpots(model, R) {
  const gr = model.guardrails.filter((g) => g.level !== 'info');
  const extra = gr.length ? `<h3>Metrics with weak inputs</h3><ul class="steps">${gr.map((g) => `<li><a href="#gr-${model.guardrails.indexOf(g)}">${esc(g.metric || 'metric')}</a> — ${R.text(g.message)}</li>`).join('')}</ul>` : '';
  return section(
    model,
    R,
    'blind_spots',
    SECTION_TITLES.blind_spots,
    howto('What this report cannot see, and how the numbers were made. Read it before quoting a number out of context.') +
      `<ul class="steps">${model.blind_spots.map((x) => `<li>${R.text(x)}</li>`).join('')}</ul>${extra}`,
  );
}

// Where a comment's "go to" link points: the section of a tile anchor; a
// ticket-row anchor has no element id, so report.js resolves it by data-anchor.
const anchorHref = (a) => (a.startsWith('t:') ? '#main' : `#${a.split(':')[0]}`);

function renderComments(model, R, comments) {
  if (!comments.length) return '';
  const items = comments
    .slice()
    .sort((a, b) => (b.t || 0) - (a.t || 0))
    .map((c) => `<li class="c-item"><div class="where"><a href="${esc(anchorHref(c.anchor))}" data-goto="${esc(c.anchor)}">${esc(c.label || c.anchor)}</a>${c.author ? ` · ${esc(c.author)}` : ''}${c.when ? ` · <time datetime="${esc(c.when)}">${esc(c.when.slice(0, 16).replace('T', ' '))}</time>` : ''}</div><div class="txt">${esc(c.text)}</div></li>`)
    .join('');
  return `<section class="card print-only" id="comments" aria-labelledby="comments-h"><header><h2 id="comments-h">${SECTION_TITLES.comments}</h2></header><ul class="c-static">${items}</ul></section>`;
}

function renderGlossary(model, R) {
  return section(model, R, 'glossary', SECTION_TITLES.glossary, `<dl class="glossary">${model.glossary.map((g) => `<dt>${esc(g.term)}</dt><dd>${esc(g.definition)}</dd>`).join('')}</dl>`);
}

function renderGuardrails(model, R) {
  return ['error', 'warn', 'info']
    .map((lvl) => {
      const items = model.guardrails.map((g, i) => ({ g, i })).filter(({ g }) => g.level === lvl);
      if (!items.length) return '';
      return `<div class="banner ${lvl}" role="${lvl === 'info' ? 'note' : 'alert'}" id="guardrails-${lvl}">${ICONS[lvl]}<div><strong>${LEVEL_TEXT[lvl]}:</strong> ${items.length} metric${items.length > 1 ? 's have' : ' has'} ${lvl === 'info' ? 'a note' : 'weak inputs'}.<ul>${items
        .map(({ g, i }) => `<li id="gr-${i}">${g.metric ? `<strong>${esc(g.metric)}</strong> — ` : ''}${R.text(g.message)}</li>`)
        .join('')}</ul></div></div>`;
    })
    .join('');
}

function renderNarrative(model, R, n) {
  const para = n.summary ? n.summary.split(/\n{2,}/).map((p) => `<p>${R.text(p)}</p>`).join('') : '';
  const list = (title, items) => (items.length ? `<h3>${title}</h3><ul>${items.map((x) => `<li>${R.text(x)}</li>`).join('')}</ul>` : '');
  const goals = model.meta.report_kind === 'dev' ? [] : n.goals;
  if (!para && !n.strengths.length && !goals.length) return '';
  return section(model, R, 'summary', SECTION_TITLES.summary, `<div class="narrative">${para}${list('Strengths', n.strengths)}${list('Goals', goals)}<p class="muted small">Written by an agent from the numbers below — check it against them.</p></div>`);
}

function normComments(list, scrub) {
  return arr(list)
    .map((c) => {
      const o = obj(c);
      if (typeof o.text !== 'string' || !o.text.trim()) return null;
      const t = o.at == null || o.at === '' ? NaN : new Date(typeof o.at === 'string' && /^\d+$/.test(o.at) ? +o.at : o.at).getTime();
      return {
        anchor: SAN.validAnchor(String(o.anchor || '')) ? String(o.anchor) : 'report',
        label: o.label ? scrub(String(o.label)) : '',
        text: scrub(String(o.text)).slice(0, 4000),
        author: o.author ? scrub(String(o.author)) : '',
        t: Number.isFinite(t) ? t : null,
        when: Number.isFinite(t) ? new Date(t).toISOString() : '',
      };
    })
    .filter(Boolean);
}

/**
 * Render the self-contained HTML report.
 * @param {object} model  from buildReportModel (or its JSON round-trip)
 * @param {{summary?:string, strengths?:string[], goals?:string[]}} [narrative]
 * @param {{comments?:Array<{anchor,label,text,author,at}>}} [opts]  host-loaded comments
 */
function renderReport(model, narrative = {}, opts = {}) {
  if (!model || typeof model !== 'object' || !SECTION_KEYS.every((k) => k in model)) throw new Error('renderReport: not a report model');
  const m = maskers.get(model);
  // Masked: scrub with the real dictionary when we have it; always drop any
  // remaining Jira-shaped key from free text.
  const scrub = model.meta.masked ? (s) => (m ? m.scrub(s) : s).replace(KEY_RE, 'a ticket') : (s) => s;
  const comments = normComments(obj(opts).comments, scrub);
  const R = makeRender(model, comments);
  const nIn = obj(narrative);
  const clean = (s) => (typeof s === 'string' ? scrub(s.trim()) : '');
  const n = { summary: clean(nIn.summary), strengths: arr(nIn.strengths).map(clean).filter(Boolean), goals: arr(nIn.goals).map(clean).filter(Boolean) };
  const meta = model.meta;
  const body = [
    renderNarrative(model, R, n),
    renderKpis(model, R),
    renderChanges(model, R),
    renderTrend(model, R),
    renderEstimateBasis(model, R),
    renderPhases(model, R),
    renderOutliers(model, R),
    renderDora(model, R),
    renderPrFlow(model, R),
    renderRework(model, R),
    renderFlow(model, R),
    renderCapacity(model, R),
    renderSubtasks(model, R),
    renderPeople(model, R),
    meta.report_kind === 'dev' ? renderGoals(model, R, n.goals) : '',
    renderList(model, R, 'next_steps', 'Rule-based follow-ups from the numbers above; each states its threshold so you can disagree with it. They are prompts for a conversation, not targets.', model.next_steps, 'Nothing crossed a follow-up threshold this period.'),
    renderBlindSpots(model, R),
    renderComments(model, R, comments),
    renderGlossary(model, R),
  ].join('\n');
  const navIds = Object.keys(SECTION_TITLES).filter((id) => body.includes(`id="${id}"`) && id !== 'comments');
  const nav = navIds.map((id) => `<a href="#${id}" data-toc="${id}">${esc(SECTION_TITLES[id])}</a>`).join('');
  const period = meta.period.start || meta.period.end ? `${meta.period.start || '…'} → ${meta.period.end || '…'}` : 'All time';
  const subtitle = [meta.scope, period, meta.compare ? `compared with ${meta.compare.label || `${meta.compare.start || '…'} → ${meta.compare.end || '…'}`}` : null, meta.masked ? 'masked' : null].filter(Boolean).join(' · ');
  const footer = [
    `Data as of ${meta.data_as_of ? esc(meta.data_as_of.slice(0, 16).replace('T', ' ')) : NOT_TRACKED}`,
    `last scan ${meta.scan_time ? esc(meta.scan_time.slice(0, 16).replace('T', ' ')) : NOT_TRACKED}`,
    `estimate ruler ${meta.ruler_version ? esc(meta.ruler_version) : NOT_TRACKED}`,
    meta.generated_at ? `generated ${esc(meta.generated_at.slice(0, 16).replace('T', ' '))}` : null,
    meta.masked ? 'names and ticket keys are masked' : null,
  ]
    .filter(Boolean)
    .join(' · ');
  const safeJson = (x) => JSON.stringify(x).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');
  const a = assets();
  const script = `window.__TP_REPORT__=${safeJson(model)};\nwindow.__TP_COMMENTS__=${safeJson(comments.map(({ anchor, label, text, author, when }) => ({ anchor, label, text, author, at: when })))};\n${a.js}`;
  // Fresh per render: a nonce that repeats across reports is a CSP bypass waiting
  // to happen (anything that can replay one report's markup could run in another).
  const nonce = SAN.newNonce();
  const slots = {
    TITLE: esc(meta.title),
    SUBTITLE: esc(subtitle),
    MASKED: meta.masked ? 'true' : 'false',
    NONCE: nonce,
    TEMPLATE_VERSION,
    PALETTE: paletteCss(),
    CSS: a.css.replace(/<\/style/gi, '<\\/style'),
    NAV: nav,
    GUARDRAILS: renderGuardrails(model, R),
    BODY: body,
    FOOTER: footer,
    COMMENT_COUNT: String(comments.length),
    COMMENT_LIST: comments.length
      ? comments
          .slice()
          .sort((x, y) => (y.t || 0) - (x.t || 0))
          .map((c) => `<div class="c-item"><div class="where">${esc(c.label || c.anchor)}${c.author ? ` · ${esc(c.author)}` : ''}${c.when ? ` · ${esc(c.when.slice(0, 16).replace('T', ' '))}` : ''}</div><div class="txt">${esc(c.text)}</div><a class="small" href="${esc(anchorHref(c.anchor))}" data-goto="${esc(c.anchor)}">Go to it</a></div>`)
          .join('')
      : '<p class="muted small">No comments yet.</p>',
    SCRIPT: script.replace(/<\/script/gi, '<\\/script'),
  };
  return a.template.replace(/\{\{([A-Z_]+)\}\}/g, (all, k) => (k in slots ? slots[k] : all));
}

// Masked-output leak check: returns the planted strings (names/keys) found.
function leakCheck(html, planted) {
  const s = String(html);
  return arr(planted).filter((p) => typeof p === 'string' && p && s.toLowerCase().includes(p.toLowerCase()));
}

module.exports = { buildReportModel, renderReport, leakCheck, paletteCss, PALETTE, SECTION_KEYS, DEFAULT_GLOSSARY, _internal: { makeMasker, normPhases, cleanBase, DORA_BANDS } };
