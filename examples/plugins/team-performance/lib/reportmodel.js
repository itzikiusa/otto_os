// Report model + deterministic HTML renderer for the team-performance reports.
//
// buildReportModel(input) is PURE: it takes the already-computed outputs of the
// analytics passes (phases, DORA, PR flow, rework, flow, capacity, sub-tasks,
// guardrails, people) as plain objects and normalises them into one fixed-shape
// model with every section key present (SECTION_KEYS). Missing measurements stay
// null — they render as "not tracked", never as a fake 0.
//
// Masking (input.mask = true) happens HERE, before anything is rendered or
// embedded: person names become "Person A…", ticket keys "Ticket 1…", free-text
// ticket titles are dropped, and every remaining string is scrubbed of known
// names/keys. The alias dictionary is never put on the model (it holds the real
// names) — it lives in a WeakMap so renderReport can scrub the agent narrative
// with it.
//
// renderReport(model, narrative) inlines report/{template.html,report.css,
// charts.js output,report.js} + the model JSON into one self-contained HTML
// string whose only <script> is its own nonce'd script (CSP pins script-src to
// that nonce). The agent fills only the narrative slots {summary, strengths[],
// goals[]}; it never writes HTML.
'use strict';
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const C = require('../report/charts.js');

const TEMPLATE_VERSION = '1';
const SECTION_KEYS = ['kpis', 'phases', 'dora', 'pr_flow', 'rework', 'flow', 'capacity', 'substantive_subtasks', 'guardrails', 'people', 'glossary', 'meta'];
const PHASES = [
  ['design', 'Design'],
  ['dev', 'Dev'],
  ['review', 'Review'],
  ['deployment', 'Deployment'],
  ['rework', 'Rework'],
];
const KEY_RE = /\b[A-Z][A-Z0-9]+-\d+\b/g;
const KEY_FULL = /^[A-Z][A-Z0-9]+-\d+$/;
// Fields whose value is a person / a ticket key / free text that may name either.
const NAME_FIELDS = new Set(['name', 'person', 'assignee', 'author', 'reviewer', 'owner', 'parent_owner', 'display_name']);
const KEY_FIELDS = new Set(['key', 'parent_key', 'source_key', 'target_key', 'epic_key', 'reworked_key', 'by_key', 'ticket']);
const TITLE_FIELDS = new Set(['title', 'summary', 'parent_title', 'epic_name', 'epic_title', 'description']);
const { NOT_TRACKED, esc, isNum, fmt } = C;

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
const cleanBase = (u) => (typeof u === 'string' && /^https?:\/\/[^\s"'<>]+$/i.test(u.trim()) ? u.trim().replace(/\/+$/, '') : null);
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
function normKpis(list, compare) {
  const prior = obj(obj(compare).kpis);
  return arr(list).map((k, i) => {
    const o = obj(k);
    const id = str(o.id) || `kpi-${i + 1}`;
    const value = num(o.value);
    const p = num(o.prior ?? prior[id]);
    const kind = ['ratio', 'count', 'days', 'hours', 'percent', 'points'].includes(o.kind) ? o.kind : 'count';
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
  const tickets = arr(r.tickets).map((t) => {
    const o = obj(t);
    const out = { key: str(o.key), title: str(o.title), person: str(o.person), type: str(o.type) };
    for (const [id] of PHASES) out[id] = num(o[id]);
    return out;
  });
  return { rows, splits, tickets };
}

function normDora(raw) {
  const d = obj(raw);
  return {
    deployments: num(d.deployments),
    hotfixes: num(d.hotfixes),
    deploy_frequency_per_week: num(d.deploy_frequency_per_week),
    lead_time_days_median: num(d.lead_time_days_median),
    lead_time_days_p85: num(d.lead_time_days_p85),
    change_failure_rate: num(d.change_failure_rate),
    mttr_hours_median: num(d.mttr_hours_median),
    weekly: arr(d.weekly).map((w) => ({ week: str(obj(w).week), deployments: num(obj(w).deployments) })),
    failures: arr(d.failures).map((f) => ({ key: str(obj(f).key), title: str(obj(f).title), tag: str(obj(f).tag), kind: str(obj(f).kind), restore_hours: num(obj(f).restore_hours), caused_by: str(obj(f).caused_by) })),
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
    unplanned_share: num(f.unplanned_share),
    context_switch_avg: num(f.context_switch_avg),
    estimate_accuracy: {
      median_ratio: num(ea.median_ratio),
      within_band_share: num(ea.within_band_share),
      buckets: arr(ea.buckets).map((b) => ({ label: str(obj(b).label), count: num(obj(b).count) })),
    },
  };
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

function normGuardrails(raw) {
  return arr(raw).map((g) => {
    const o = obj(g);
    return { level: ['error', 'warn', 'info'].includes(o.level) ? o.level : 'warn', metric: str(o.metric), message: str(o.message) || '' };
  });
}

function normPeople(raw) {
  return arr(raw).map((p) => {
    const o = obj(p);
    return {
      name: str(o.name),
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
      tickets: arr(o.tickets).map((t) => ({ key: str(obj(t).key), title: str(obj(t).title), points: num(obj(t).points), dev_days: num(obj(t).dev_days), status: str(obj(t).status) })),
      notes: arr(o.notes).map(str).filter(Boolean),
    };
  });
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
  ['WIP', 'Tickets in active development at the same time, averaged over the period.'],
  ['Throughput', 'Tickets delivered per week.'],
  ['Unplanned share', 'Share of delivered work that was bugs / hotfixes / items added mid-sprint.'],
  ['Estimate accuracy', 'Actual dev time ÷ estimated time. 1.0 is on estimate; above 1 took longer.'],
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

  const model = {
    kpis: normKpis(src.kpis, compare),
    phases: normPhases(src.phases),
    dora: normDora(src.dora),
    pr_flow: normPrFlow(src.pr_flow),
    rework: normRework(src.rework),
    flow: normFlow(src.flow),
    capacity: normCapacity(src.capacity),
    substantive_subtasks: normSubtasks(src.substantive_subtasks),
    guardrails: normGuardrails(src.guardrails),
    people: normPeople(src.people),
    glossary,
    meta: {
      template_version: TEMPLATE_VERSION,
      title: str(metaIn.title) || 'Team performance report',
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
  model.guardrails.push(...derivedGuardrails(model));
  if (m) maskers.set(model, m);
  return model;
}

// Weak-input warnings the analytics passes may not have raised themselves.
function derivedGuardrails(model) {
  const out = [];
  const have = new Set(model.guardrails.map((g) => g.metric));
  const add = (metric, level, message) => {
    if (!have.has(metric)) out.push({ level, metric, message });
  };
  const design = model.phases.rows.find((r) => r.phase === 'design');
  if (design && design.not_tracked) add('phases.design', 'info', 'Design time is not tracked for most tickets in this period — the phase totals exclude it rather than count it as zero.');
  if (model.kpis.some((k) => k.kind === 'ratio' && !isNum(k.capacity_days))) add('kpis.capacity', 'warn', 'At least one ratio has no capacity figure. Read it as a rough signal, not as productivity.');
  if (model.capacity.length && model.capacity.every((c) => c.time_off_days == null)) add('capacity.time_off', 'warn', 'No time off is recorded for anyone, so capacity may be overstated.');
  if (model.pr_flow.prs == null) add('pr_flow', 'warn', 'Pull-request data was not available, so pickup / review / merge times are not tracked.');
  if (model.dora.deployments === 0 || model.dora.deployments == null) add('dora', 'warn', 'No deployment tags were found in the period. Deployment frequency, lead time and change failure rate are not tracked.');
  return out;
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

function makeRender(model) {
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
  return { tk, text };
}

const v = (x, unit = '', digits = 1) => (isNum(x) ? `${esc(fmt(x, digits))}${unit ? ` ${esc(unit)}` : ''}` : `<span class="nt">${NOT_TRACKED}</span>`);
const vp = (x) => (isNum(x) ? esc(pct(x)) : `<span class="nt">${NOT_TRACKED}</span>`);
const howto = (html) => `<p class="howto"><strong>How to read it.</strong> ${html}</p>`;
const tile = (label, value, ctx) => `<div class="tile"><div class="v">${value}</div><div class="l">${esc(label)}</div>${ctx ? `<div class="c">${ctx}</div>` : ''}</div>`;
const tableHtml = (headers, rows, numericFrom = 1) =>
  rows.length
    ? `<div class="tbl-wrap"><table><thead><tr>${headers.map((h, i) => `<th scope="col"${i >= numericFrom ? ' class="n"' : ''}>${esc(h)}</th>`).join('')}</tr></thead><tbody>${rows
        .map((r) => `<tr>${r.map((c, i) => `<td${i >= numericFrom ? ' class="n"' : ''}>${c}</td>`).join('')}</tr>`)
        .join('')}</tbody></table></div>`
    : '<p class="muted small">Nothing in this period.</p>';
const section = (id, title, body) => `<section class="card" id="${id}" data-anchor="${id}" data-anchor-label="${esc(title)}"><header><h2>${esc(title)}</h2></header>${body}</section>`;

const SECTION_TITLES = {
  summary: 'Summary',
  kpis: 'Key numbers',
  phases: 'Where the time goes',
  dora: 'Delivery (DORA)',
  pr_flow: 'Pull-request flow',
  rework: 'Rework',
  flow: 'Flow & investment',
  capacity: 'Capacity',
  substantive_subtasks: 'Sub-tasks with real work',
  people: 'People',
  glossary: 'Glossary',
};

function renderKpis(model, R) {
  const cards = model.kpis.map((k) => {
    let ctx = '';
    if (k.kind === 'ratio') ctx += isNum(k.capacity_days) ? `over ${esc(fmt(k.capacity_days))} capacity days` : '<span class="pill warn">capacity unknown — not a productivity figure</span>';
    if (isNum(k.delta)) {
      const dir = k.delta === 0 || !k.better ? 'flat' : (k.delta > 0) === (k.better === 'up') ? 'better' : 'worse';
      ctx += `${ctx ? ' · ' : ''}<span class="delta ${dir}">${k.delta > 0 ? '+' : ''}${esc(fmt(k.delta))} vs ${esc(model.meta.compare?.label || 'previous period')}</span>`;
    }
    if (k.note) ctx += `${ctx ? ' · ' : ''}${R.text(k.note)}`;
    const val = k.kind === 'percent' ? vp(k.value) : v(k.value, k.unit);
    return tile(k.label, val, ctx);
  });
  return section(
    'kpis',
    SECTION_TITLES.kpis,
    howto('Each tile is one number for the whole scope. A ratio is always shown with the capacity it was divided by — a lower ratio in a period with less capacity is not lower productivity. The change is against the comparison period when one was chosen.') +
      (cards.length ? `<div class="grid">${cards.join('')}</div>` : '<p class="muted small">No headline numbers were computed for this period.</p>'),
  );
}

function renderPhases(model, R) {
  const P = model.phases;
  const chart = C.barChart({
    id: 'ch-phases',
    title: 'Median days per phase',
    desc: `Median business days spent in each phase. ${P.rows.filter((r) => r.not_tracked).map((r) => r.label).join(', ') || 'No'} phase(s) are not tracked.`,
    rows: P.rows.map((r) => ({ label: r.label, value: r.median_days })),
    unit: 'd',
  });
  const rows = P.rows.map((r) => [esc(r.label), v(r.median_days, 'd'), v(r.p85_days, 'd'), v(r.total_days, 'd'), isNum(r.tickets_tracked) && isNum(r.tickets_total) ? `${r.tickets_tracked} / ${r.tickets_total} (${esc(pct(r.coverage))})` : `<span class="nt">${NOT_TRACKED}</span>`]);
  const splits = P.splits.length ? `<h3>Phase detail</h3>${tableHtml(['Part', 'Median', 'Total'], P.splits.map((s) => [esc(s.label || s.phase), v(s.median_days, 'd'), v(s.total_days, 'd')]))}` : '';
  const tix = P.tickets.length
    ? `<h3>Tickets</h3>${tableHtml(
        ['Ticket', 'Title', 'Person', ...PHASES.map(([, l]) => l)],
        P.tickets.map((t) => [R.tk(t.key), t.title ? esc(t.title) : '', esc(t.person || ''), ...PHASES.map(([id]) => v(t[id], 'd'))]),
        3,
      )}`
    : '';
  return section(
    'phases',
    SECTION_TITLES.phases,
    howto('Ticket time is split into design, dev, review, deployment and rework, in business days. <em>Coverage</em> is how many tickets had evidence for the phase; "not tracked" means no evidence, not zero time. Medians resist one long ticket; p85 shows the slow tail.') +
      chart +
      tableHtml(['Phase', 'Median', 'p85', 'Total', 'Coverage'], rows) +
      splits +
      tix,
  );
}

function renderDora(model, R) {
  const d = model.dora;
  const tiles = [
    tile('Deployment frequency', v(d.deploy_frequency_per_week, '/ week'), isNum(d.deployments) ? `${esc(fmt(d.deployments, 0))} deployments${isNum(d.hotfixes) ? `, ${esc(fmt(d.hotfixes, 0))} hotfix` : ''}` : ''),
    tile('Lead time for changes', v(d.lead_time_days_median, 'd'), isNum(d.lead_time_days_p85) ? `p85 ${esc(fmt(d.lead_time_days_p85))} d` : ''),
    tile('Change failure rate', vp(d.change_failure_rate), 'hotfix or bug after deploy'),
    tile('Time to restore', v(d.mttr_hours_median, 'h'), 'median'),
  ];
  const chart = d.weekly.length ? C.columnChart({ id: 'ch-deploys', title: 'Deployments per week', desc: `Deployment tags per week, ${d.weekly.length} weeks.`, points: d.weekly.map((w) => ({ label: w.week, value: w.deployments })), digits: 0 }) : '';
  const fails = d.failures.length ? `<h3>Failures</h3>${tableHtml(['Ticket', 'Kind', 'Tag', 'Caused by', 'Restore'], d.failures.map((f) => [R.tk(f.key) + (f.title ? ` ${esc(f.title)}` : ''), esc(f.kind || ''), esc(f.tag || ''), R.tk(f.caused_by), v(f.restore_hours, 'h')]), 4)}` : '';
  return section(
    'dora',
    SECTION_TITLES.dora,
    howto('A deployment is a git tag whose name contains "deployed", "hf" or "hotfix". Change failure rate counts deployments followed by a hotfix or a bug traced to the change. These are team numbers — they describe the delivery system, not any one person.') +
      `<div class="grid">${tiles.join('')}</div>${chart}${fails}`,
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
  const by = p.by_person.length
    ? `<h3>By person</h3>${tableHtml(['Person', 'PRs', 'Reviews given', 'Pickup (median)', 'Size (median)'], p.by_person.map((x) => [esc(x.person || ''), v(x.prs, '', 0), v(x.reviews_given, '', 0), v(x.pickup_hours_median, 'h'), v(x.size_lines_median, '', 0)]))}`
    : '';
  return section(
    'pr_flow',
    SECTION_TITLES.pr_flow,
    howto(`Pickup is how long a PR waits for its first reviewer; long pickup usually means reviewers are overloaded, not that authors are slow. Small PRs review faster. ${isNum(p.unreviewed_share) ? `${esc(pct(p.unreviewed_share))} of PRs merged without a review.` : ''}${p.coverage_note ? ` ${R.text(p.coverage_note)}` : ''}`) +
      `<div class="grid">${tiles.join('')}</div>${by}`,
  );
}

function renderRework(model, R) {
  const r = model.rework;
  const pairRows = (list) => list.map((x) => [R.tk(x.key), R.tk(x.by_key), esc(x.person || ''), v(x.lines, '', 0), v(x.days, 'd')]);
  return section(
    'rework',
    SECTION_TITLES.rework,
    howto('<em>Rework in</em>: your recent code was rewritten by a later ticket — that time is charged back to the original ticket. <em>Rework out</em>: your ticket rewrote someone else\'s recent code. Jira rework tickets (fixes, follow-ups, reopens) are listed below; their estimates do not count as new delivered scope.') +
      `<div class="grid">${tile('Rework rate', vp(r.rate), 'share of changed lines that rewrote recent code')}${tile('Days charged back', v(r.days_charged, 'd'), 'to original tickets')}</div>` +
      `<h3>Rework in (original ticket ← later ticket)</h3>${tableHtml(['Original', 'Rewritten by', 'Person', 'Lines', 'Days'], pairRows(r.in), 3)}` +
      `<h3>Rework out</h3>${tableHtml(['Ticket', 'Rewrote', 'Person', 'Lines', 'Days'], pairRows(r.out), 3)}` +
      `<h3>Jira-detected rework</h3>${tableHtml(
        ['Ticket', 'Reworks', 'Why', 'Points', 'Scope'],
        r.jira.map((x) => [R.tk(x.key) + (x.title ? ` ${esc(x.title)}` : ''), R.tk(x.source_key), esc(x.reason || ''), v(x.points, '', 1), x.excluded_from_scope ? 'not new scope' : 'counted']),
        3,
      )}`,
  );
}

function renderFlow(model) {
  const f = model.flow;
  const tiles = [
    tile('WIP', v(f.wip_avg), isNum(f.wip_per_person) ? `${esc(fmt(f.wip_per_person))} per person` : 'average in progress'),
    tile('Unplanned work', vp(f.unplanned_share), 'bugs, hotfixes, mid-sprint adds'),
    tile('Context switching', v(f.context_switch_avg), 'tickets touched per person-day'),
    tile('Estimate accuracy', v(f.estimate_accuracy.median_ratio, '×', 2), isNum(f.estimate_accuracy.within_band_share) ? `${esc(pct(f.estimate_accuracy.within_band_share))} within band` : 'actual ÷ estimate, median'),
  ];
  const charts = [
    f.throughput.length ? C.columnChart({ id: 'ch-throughput', title: 'Tickets delivered per week', desc: `Throughput over ${f.throughput.length} weeks.`, points: f.throughput.map((w) => ({ label: w.week, value: w.count })), digits: 0 }) : '',
    f.investment.length ? C.barChart({ id: 'ch-invest', title: 'Investment mix (share of delivered work)', desc: 'Share of delivered work per ticket type or epic.', rows: f.investment.map((x) => ({ label: x.label, value: isNum(x.share) ? x.share * 100 : null })), unit: '%', digits: 0, series: 3 }) : '',
    f.estimate_accuracy.buckets.length ? C.barChart({ id: 'ch-accuracy', title: 'Estimate accuracy distribution', desc: 'Number of tickets per actual-to-estimate band.', rows: f.estimate_accuracy.buckets.map((b) => ({ label: b.label, value: b.count })), digits: 0, series: 4 }) : '',
  ].join('');
  return section(
    'flow',
    SECTION_TITLES.flow,
    howto('High WIP and frequent context switching slow everything down even when everyone is busy. Investment mix shows where effort went; a large unplanned share means planned work was squeezed. Estimate accuracy above 1× means work took longer than estimated.') +
      `<div class="grid">${tiles.join('')}</div>${charts}`,
  );
}

function renderCapacity(model) {
  const rows = model.capacity.map((c) => [esc(c.person || ''), v(c.working_days, 'd'), v(c.time_off_days, 'd'), v(c.capacity_days, 'd'), v(c.dev_days, 'd'), v(c.delivered_points), isNum(c.points_per_capacity_day) ? `${esc(fmt(c.points_per_capacity_day, 2))} <span class="muted">over ${v(c.capacity_days, 'd')}</span>` : `<span class="nt">${NOT_TRACKED}</span>`]);
  return section(
    'capacity',
    SECTION_TITLES.capacity,
    howto('Capacity days = working days minus recorded time off. Points per capacity day is a pace signal for planning, not a performance score: it ignores reviews, support, mentoring and design work, and swings with how tickets were sized.') +
      tableHtml(['Person', 'Working', 'Time off', 'Capacity', 'Dev time', 'Points', 'Points / capacity day'], rows),
  );
}

function renderSubtasks(model, R) {
  const rows = model.substantive_subtasks.map((s) => [esc(s.person || ''), R.tk(s.key) + (s.title ? ` ${esc(s.title)}` : ''), R.tk(s.parent_key) + (s.parent_owner ? ` <span class="muted">(${esc(s.parent_owner)})</span>` : ''), esc(s.reason || ''), v(s.dev_days, 'd'), v(s.commits, '', 0)]);
  return section(
    'substantive_subtasks',
    SECTION_TITLES.substantive_subtasks,
    howto('Most sub-tasks are checklists and are ignored. These ones carried real work — their own dev time or commits, under someone else\'s story, or under a story with no timing of its own — so the work is credited to the person who did it.') +
      tableHtml(['Person', 'Sub-task', 'Parent', 'Why it counts', 'Dev time', 'Commits'], rows, 4),
  );
}

function renderPeople(model, R) {
  if (!model.people.length) return section('people', SECTION_TITLES.people, '<p class="muted small">No per-person breakdown in this report.</p>');
  const cards = model.people.map((p, i) => {
    const id = `person-${i + 1}`;
    const kv = [
      ['Capacity', `${v(p.capacity_days, 'd')}${isNum(p.time_off_days) ? ` <span class="muted">(${esc(fmt(p.time_off_days))} d off)</span>` : ''}`],
      ['Delivered', `${v(p.delivered_points, 'pts')}${isNum(p.points_per_capacity_day) ? ` <span class="muted">· ${esc(fmt(p.points_per_capacity_day, 2))} / capacity day</span>` : ''}`],
      ['Cycle time (median)', v(p.cycle_time_days_median, 'd')],
      ['Design time', v(p.design_days, 'd')],
      ['Dev time', v(p.dev_days, 'd')],
      ['PRs / reviews given', `${v(p.prs, '', 0)} / ${v(p.reviews_given, '', 0)}`],
      ['Rework in / out (lines)', `${v(p.rework_in_lines, '', 0)} / ${v(p.rework_out_lines, '', 0)}`],
      ['Estimate accuracy', v(p.estimate_ratio_median, '×', 2)],
    ];
    const tix = p.tickets.length ? `<details class="as-table"><summary>Tickets (${p.tickets.length})</summary>${tableHtml(['Ticket', 'Status', 'Points', 'Dev time'], p.tickets.map((t) => [R.tk(t.key) + (t.title ? ` ${esc(t.title)}` : ''), esc(t.status || ''), v(t.points), v(t.dev_days, 'd')]), 2)}</details>` : '';
    const notes = p.notes.length ? `<ul class="small">${p.notes.map((n) => `<li>${R.text(n)}</li>`).join('')}</ul>` : '';
    return `<div class="person" id="${id}" data-anchor="${id}" data-anchor-label="${esc(p.name || id)}"><header><h3>${esc(p.name || id)}${p.role ? ` <span class="pill">${esc(p.role)}</span>` : ''}</h3></header><dl class="kv">${kv.map(([k, x]) => `<dt>${esc(k)}</dt><dd>${x}</dd>`).join('')}</dl>${notes}${tix}</div>`;
  });
  return section('people', SECTION_TITLES.people, howto('One card per person. Every figure sits next to the capacity it came from; compare people only with that context, and never on points alone.') + `<div class="grid wide">${cards.join('')}</div>`);
}

function renderGlossary(model) {
  return section('glossary', SECTION_TITLES.glossary, `<dl class="glossary">${model.glossary.map((g) => `<dt>${esc(g.term)}</dt><dd>${esc(g.definition)}</dd>`).join('')}</dl>`);
}

const ICONS = {
  warn: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1.5 15 14H1L8 1.5Zm-.75 4.5v4h1.5V6h-1.5Zm0 5v1.5h1.5V11h-1.5Z"/></svg>',
  error: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1a7 7 0 1 1 0 14A7 7 0 0 1 8 1Zm-.75 3.5v5h1.5v-5h-1.5Zm0 6v1.5h1.5V10.5h-1.5Z"/></svg>',
  info: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1a7 7 0 1 1 0 14A7 7 0 0 1 8 1Zm-.75 6v5h1.5V7h-1.5Zm0-2.5V6h1.5V4.5h-1.5Z"/></svg>',
};
const LEVEL_TEXT = { error: 'Unreliable inputs', warn: 'Read with care', info: 'Note' };

function renderGuardrails(model, R) {
  const levels = ['error', 'warn', 'info'];
  return levels
    .map((lvl) => {
      const items = model.guardrails.filter((g) => g.level === lvl);
      if (!items.length) return '';
      return `<div class="banner ${lvl}" role="${lvl === 'info' ? 'note' : 'alert'}" id="guardrails-${lvl}">${ICONS[lvl]}<div><strong>${LEVEL_TEXT[lvl]}:</strong> ${items.length} metric${items.length > 1 ? 's have' : ' has'} weak inputs.<ul>${items
        .map((g) => `<li>${g.metric ? `<strong>${esc(g.metric)}</strong> — ` : ''}${R.text(g.message)}</li>`)
        .join('')}</ul></div></div>`;
    })
    .join('');
}

function renderNarrative(narrative, R, scrub) {
  const n = obj(narrative);
  const clean = (s) => (typeof s === 'string' ? scrub(s.trim()) : '');
  const summary = clean(n.summary);
  const strengths = arr(n.strengths).map(clean).filter(Boolean);
  const goals = arr(n.goals).map(clean).filter(Boolean);
  if (!summary && !strengths.length && !goals.length) return '';
  const para = summary ? summary.split(/\n{2,}/).map((p) => `<p>${R.text(p)}</p>`).join('') : '';
  const list = (title, items) => (items.length ? `<h3>${title}</h3><ul>${items.map((x) => `<li>${R.text(x)}</li>`).join('')}</ul>` : '');
  return section('summary', SECTION_TITLES.summary, `<div class="narrative">${para}${list('Strengths', strengths)}${list('Goals', goals)}<p class="muted small">Written by an agent from the numbers below — check it against them.</p></div>`);
}

function renderReport(model, narrative = {}) {
  if (!model || typeof model !== 'object' || !SECTION_KEYS.every((k) => k in model)) throw new Error('renderReport: not a report model');
  const R = makeRender(model);
  const m = maskers.get(model);
  // Masked: scrub with the real dictionary when we have it; always drop any
  // remaining Jira-shaped key from agent text.
  const scrub = model.meta.masked ? (s) => (m ? m.scrub(s) : s).replace(KEY_RE, 'a ticket') : (s) => s;
  const meta = model.meta;
  const body = [
    renderNarrative(narrative, R, scrub),
    renderKpis(model, R),
    renderPhases(model, R),
    renderDora(model, R),
    renderPrFlow(model, R),
    renderRework(model, R),
    renderFlow(model, R),
    renderCapacity(model, R),
    renderSubtasks(model, R),
    renderPeople(model, R),
    renderGlossary(model),
  ].join('\n');
  const navIds = Object.keys(SECTION_TITLES).filter((id) => body.includes(`id="${id}"`));
  const nav = navIds.map((id) => `<a href="#${id}">${esc(SECTION_TITLES[id])}</a>`).join('');
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
  const json = JSON.stringify(model).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');
  const a = assets();
  const script = `window.__TP_REPORT__=${json};\n${a.js}`;
  const nonce = crypto.createHash('sha256').update(script).update(body).digest('base64').replace(/[^A-Za-z0-9]/g, '').slice(0, 24);
  const slots = {
    TITLE: esc(meta.title),
    SUBTITLE: esc(subtitle),
    MASKED: meta.masked ? 'true' : 'false',
    NONCE: nonce,
    TEMPLATE_VERSION,
    CSS: a.css.replace(/<\/style/gi, '<\\/style'),
    NAV: nav,
    GUARDRAILS: renderGuardrails(model, R),
    BODY: body,
    FOOTER: footer,
    SCRIPT: script.replace(/<\/script/gi, '<\\/script'),
  };
  return a.template.replace(/\{\{([A-Z_]+)\}\}/g, (all, k) => (k in slots ? slots[k] : all));
}

// Masked-output leak check: returns the planted strings (names/keys) found.
function leakCheck(html, planted) {
  const s = String(html);
  return arr(planted).filter((p) => typeof p === 'string' && p && s.toLowerCase().includes(p.toLowerCase()));
}

module.exports = { buildReportModel, renderReport, leakCheck, SECTION_KEYS, DEFAULT_GLOSSARY, _internal: { makeMasker, normPhases } };
