// Deterministic, accessible charts for the HTML report. Rendered server-side
// (no client JS): every chart is a <figure> holding ONE SVG with role="img" +
// <title>/<desc> (the desc carries the data in words), and a "Show as table"
// <details> with the exact numbers. Colours come from CSS classes (.s1….s6 →
// --cat-1…6) so light/dark/print follow the palette.
//
// Horizontal charts keep every piece of text OUTSIDE the SVG: row labels sit in
// an HTML column, value / segment labels are HTML spans positioned over the
// track by percentage. The SVG itself only draws shapes and is stretched
// horizontally (preserveAspectRatio="none"), so the chart stays readable at
// phone width instead of shrinking its text to 6 px.
//
// null values are "not tracked" — drawn as a dashed outline (never a zero bar)
// and spelled out in the table.
'use strict';

const NT = 'not tracked';
const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const isNum = (v) => typeof v === 'number' && Number.isFinite(v);
const fmt = (v, digits = 1) => {
  if (!isNum(v)) return NT;
  const d = Math.abs(v) >= 100 ? 0 : digits;
  return v.toLocaleString('en-US', { maximumFractionDigits: d, minimumFractionDigits: 0 });
};
const cell = (v, unit, digits) => (isNum(v) ? `${fmt(v, digits)}${unit ? ` ${esc(unit)}` : ''}` : `<span class="nt">${NT}</span>`);
const withUnit = (v, unit, digits) => (isNum(v) ? `${fmt(v, digits)}${unit ? ` ${unit}` : ''}` : NT);

// Fixed phase → colour mapping, shared by every phase chart and legend.
const PHASE_CLASS = { design: 's4', dev: 's1', review: 's3', deployment: 's6', deploy: 's6', rework: 's5' };
const sClass = (i) => `s${(i % 6) + 1}`;

// Track geometry: the SVG is 1000 units wide; bars use at most BAR_MAX so the
// value label after the longest bar still fits. A segment is labelled in place
// only when it is wider than 28 px at the NOMINAL desktop track width.
const VB = 1000;
const BAR_MAX = 820;
const NOMINAL_TRACK_PX = 440;
const MIN_LABEL_PX = 28;
const ROW = 26;

function table(headers, rows) {
  return `<div class="tbl-wrap"><table><thead><tr>${headers.map((h, i) => `<th scope="col"${i ? ' class="n"' : ''}>${esc(h)}</th>`).join('')}</tr></thead><tbody>${rows
    .map((r) => `<tr>${r.map((c, i) => (i ? `<td class="n">${c}</td>` : `<th scope="row">${c}</th>`)).join('')}</tr>`)
    .join('')}</tbody></table></div>`;
}

function svgOpen(id, title, desc, h, extra = '') {
  const tid = `${id}-t`;
  const did = `${id}-d`;
  return `<svg viewBox="0 0 ${VB} ${h}" role="img" aria-labelledby="${esc(tid)} ${esc(did)}" preserveAspectRatio="none"${extra}><title id="${esc(tid)}">${esc(title)}</title><desc id="${esc(did)}">${esc(desc)}</desc>`;
}

function hFrame({ id, title, desc, rows, shapes, overlay, legend, tbl, axis }) {
  const h = Math.max(1, rows.length) * ROW;
  const labels = rows.map((r) => `<div class="hc-l" title="${esc(r)}">${esc(r)}</div>`).join('');
  return `<figure class="chart hchart" id="${esc(id)}"><figcaption>${esc(title)}</figcaption>${legend || ''}<div class="hc"><div class="hc-labels" aria-hidden="true">${labels}</div><div class="hc-track">${svgOpen(
    id,
    title,
    desc,
    h,
    ` style="height:${h}px"`,
  )}${shapes}</svg><div class="hc-over" aria-hidden="true">${overlay}</div>${axis || ''}</div></div><details class="as-table"><summary>Show as table</summary>${tbl}</details></figure>`;
}

const pctX = (u) => `${((u / VB) * 100).toFixed(2)}%`;

// Horizontal bars, one per row. rows: [{label, value}]
function barChart({ id, title, desc, rows, unit = '', digits = 1, series = 1, cls }) {
  const list = Array.isArray(rows) ? rows : [];
  const max = Math.max(0, ...list.map((r) => (isNum(r.value) ? r.value : 0))) || 1;
  const shapes = [];
  const over = [];
  list.forEach((r, i) => {
    const y = i * ROW;
    const k = r.cls || cls || sClass(series - 1);
    if (isNum(r.value)) {
      const w = Math.max(r.value > 0 ? 2 : 0, Math.round((Math.max(0, r.value) / max) * BAR_MAX));
      shapes.push(`<rect class="${esc(k)}" x="0" y="${y + 5}" width="${w}" height="${ROW - 10}"/>`);
      over.push(`<span class="hc-v" style="left:calc(${pctX(w)} + 6px);top:${y}px">${esc(withUnit(r.value, unit, digits))}</span>`);
    } else {
      shapes.push(`<rect class="nt-bar" x="1" y="${y + 5}" width="${BAR_MAX / 4}" height="${ROW - 10}" vector-effect="non-scaling-stroke"/>`);
      over.push(`<span class="hc-v nt" style="left:calc(${pctX(BAR_MAX / 4)} + 6px);top:${y}px">${NT}</span>`);
    }
  });
  const data = list.map((r) => `${r.label}: ${withUnit(r.value, unit, digits)}`).join('; ');
  const tbl = table(['', unit ? `Value (${unit})` : 'Value'], list.map((r) => [esc(r.label), cell(r.value, '', digits)]));
  return hFrame({ id, title, desc: `${desc || ''} ${data}`.trim(), rows: list.map((r) => String(r.label)), shapes: shapes.join(''), overlay: over.join(''), tbl });
}

// Stacked horizontal bars. series: [name]; rows: [{label, values:[number|null]}];
// classes: optional per-series CSS class (e.g. the fixed phase colours). A null
// segment is excluded from the bar and flagged "not tracked" in the table.
function stackedBar({ id, title, desc, series, rows, unit = '', digits = 1, classes }) {
  const names = Array.isArray(series) ? series : [];
  const list = Array.isArray(rows) ? rows : [];
  const klass = (i) => (Array.isArray(classes) && classes[i]) || sClass(i);
  const total = (r) => (r.values || []).reduce((a, v) => a + (isNum(v) && v > 0 ? v : 0), 0);
  const max = Math.max(0, ...list.map(total)) || 1;
  const shapes = [];
  const over = [];
  list.forEach((r, ri) => {
    const y = ri * ROW;
    let x = 0;
    (r.values || []).forEach((v, i) => {
      if (!isNum(v) || v <= 0) return;
      const w = Math.max(2, Math.round((v / max) * BAR_MAX));
      shapes.push(`<rect class="${esc(klass(i))}" x="${x}" y="${y + 4}" width="${w}" height="${ROW - 8}"/>`);
      if ((w / VB) * NOMINAL_TRACK_PX > MIN_LABEL_PX) over.push(`<span class="hc-seg" style="left:${pctX(x)};width:${pctX(w)};top:${y}px">${esc(fmt(v, digits))}</span>`);
      x += w;
    });
    if (!total(r) && !(r.values || []).some(isNum)) {
      shapes.push(`<rect class="nt-bar" x="1" y="${y + 4}" width="${BAR_MAX / 4}" height="${ROW - 8}" vector-effect="non-scaling-stroke"/>`);
      over.push(`<span class="hc-v nt" style="left:calc(${pctX(BAR_MAX / 4)} + 6px);top:${y}px">${NT}</span>`);
    } else over.push(`<span class="hc-v" style="left:calc(${pctX(x)} + 6px);top:${y}px">${esc(withUnit(total(r), unit, digits))}</span>`);
  });
  const legend = `<div class="legend" aria-hidden="true">${names.map((n, i) => `<span><i class="${esc(klass(i))}"></i>${esc(n)}</span>`).join('')}</div>`;
  const data = list.map((r) => `${r.label}: ${names.map((n, i) => `${n} ${withUnit((r.values || [])[i], unit, digits)}`).join(', ')}`).join('; ');
  const tbl = table(['', ...names.map((n) => (unit ? `${n} (${unit})` : n)), unit ? `Total (${unit})` : 'Total'], list.map((r) => [esc(r.label), ...names.map((_, i) => cell((r.values || [])[i], '', digits)), cell(total(r) || null, '', digits)]));
  return hFrame({ id, title, desc: `${desc || ''} ${data}`.trim(), rows: list.map((r) => String(r.label)), shapes: shapes.join(''), overlay: over.join(''), legend, tbl });
}

// Strip plot: one row per series, one tick per observation (a PR), so the
// spread and the slow tail are visible — not just the median. rows:
// [{label, values:[number]}]. Values above the shared cap (p95 of all) are
// pinned to the right edge and counted in the table.
function stripPlot({ id, title, desc, rows, unit = 'h', digits = 1 }) {
  const list = (Array.isArray(rows) ? rows : []).map((r) => ({ label: String(r.label), values: (r.values || []).filter((v) => isNum(v) && v >= 0) }));
  const all = list.flatMap((r) => r.values).sort((a, b) => a - b);
  const q = (xs, k) => (xs.length ? xs[Math.min(xs.length - 1, Math.floor((xs.length - 1) * k + 0.5))] : null);
  const cap = Math.max(1, q(all, 0.95) || 1);
  const shapes = [];
  const over = [];
  list.forEach((r, ri) => {
    const y = ri * ROW;
    shapes.push(`<line class="axis" x1="0" y1="${y + ROW / 2}" x2="${BAR_MAX}" y2="${y + ROW / 2}" vector-effect="non-scaling-stroke"/>`);
    for (const v of r.values) {
      const x = Math.round((Math.min(v, cap) / cap) * BAR_MAX);
      shapes.push(`<line class="tick${v > cap ? ' over' : ''}" x1="${x}" y1="${y + 5}" x2="${x}" y2="${y + ROW - 5}" vector-effect="non-scaling-stroke"/>`);
    }
    const med = q(r.values.slice().sort((a, b) => a - b), 0.5);
    if (isNum(med)) {
      const x = Math.round((Math.min(med, cap) / cap) * BAR_MAX);
      shapes.push(`<line class="median" x1="${x}" y1="${y + 2}" x2="${x}" y2="${y + ROW - 2}" vector-effect="non-scaling-stroke"/>`);
      over.push(`<span class="hc-v" style="left:calc(${pctX(BAR_MAX)} + 6px);top:${y}px">median ${esc(withUnit(med, unit, digits))}</span>`);
    } else over.push(`<span class="hc-v nt" style="left:6px;top:${y}px">${NT}</span>`);
  });
  const axis = `<div class="hc-axis" aria-hidden="true"><span style="left:0">0</span><span style="left:${pctX(BAR_MAX / 2)}">${esc(withUnit(cap / 2, unit, 0))}</span><span style="left:${pctX(BAR_MAX)}">${esc(withUnit(cap, unit, 0))}+</span></div>`;
  const stats = list.map((r) => {
    const s = r.values.slice().sort((a, b) => a - b);
    return { label: r.label, n: s.length, med: q(s, 0.5), p85: q(s, 0.85), max: s.length ? s[s.length - 1] : null, over: s.filter((v) => v > cap).length };
  });
  const data = stats.map((s) => `${s.label}: ${s.n} observations, median ${withUnit(s.med, unit, digits)}, p85 ${withUnit(s.p85, unit, digits)}, max ${withUnit(s.max, unit, digits)}`).join('; ');
  const tbl = table(['', 'Count', `Median (${unit})`, `p85 (${unit})`, `Max (${unit})`, 'Beyond axis'], stats.map((s) => [esc(s.label), String(s.n), cell(s.med, '', digits), cell(s.p85, '', digits), cell(s.max, '', digits), String(s.over)]));
  return hFrame({ id, title, desc: `${desc || ''} ${data}`.trim(), rows: list.map((r) => r.label), shapes: shapes.join(''), overlay: over.join(''), tbl, axis });
}

// Vertical columns over time. points: [{label, value}]. Labels are sparse HTML
// under the plot for the same narrow-screen reason as the horizontal charts.
function columnChart({ id, title, desc, points, unit = '', digits = 1 }) {
  const list = Array.isArray(points) ? points : [];
  const max = Math.max(0, ...list.map((p) => (isNum(p.value) ? p.value : 0))) || 1;
  const H = 140;
  const n = Math.max(1, list.length);
  const step = VB / n;
  const bw = Math.max(4, step * 0.66);
  const shapes = [`<line class="axis" x1="0" y1="${H}" x2="${VB}" y2="${H}" vector-effect="non-scaling-stroke"/>`];
  const labels = [];
  const every = Math.ceil(n / 8);
  list.forEach((p, i) => {
    const cx = step * i + step / 2;
    if (isNum(p.value)) {
      const h = Math.round((Math.max(0, p.value) / max) * (H - 8));
      shapes.push(`<rect class="s1" x="${(cx - bw / 2).toFixed(1)}" y="${H - h}" width="${bw.toFixed(1)}" height="${h}"/>`);
    }
    if (i % every === 0) labels.push(`<span style="left:${pctX(cx)}">${esc(String(p.label).slice(-5))}</span>`);
  });
  const data = list.map((p) => `${p.label}: ${withUnit(p.value, unit, digits)}`).join('; ');
  const tbl = table(['', unit ? `Value (${unit})` : 'Value'], list.map((p) => [esc(p.label), cell(p.value, '', digits)]));
  return `<figure class="chart vchart" id="${esc(id)}"><figcaption>${esc(title)} <span class="muted">· max ${esc(withUnit(max, unit, digits))}</span></figcaption><div class="vc">${svgOpen(id, title, `${desc || ''} ${data}`.trim(), H, ` style="height:${H}px"`)}${shapes.join(
    '',
  )}</svg><div class="hc-axis" aria-hidden="true">${labels.join('')}</div></div><details class="as-table"><summary>Show as table</summary>${tbl}</details></figure>`;
}

// Tiny trend line for a tile. Decorative SVG (aria-hidden) + a visually hidden
// text alternative listing the values, so it never needs its own table.
function sparkline({ values, labels, label = 'Trend', unit = '', digits = 1 }) {
  const pts = (Array.isArray(values) ? values : []).map((v) => (isNum(v) ? v : null));
  const nums = pts.filter(isNum);
  if (nums.length < 2) return '';
  const lo = Math.min(...nums);
  const hi = Math.max(...nums);
  const span = hi - lo || 1;
  const W = 80;
  const H = 22;
  const step = pts.length > 1 ? W / (pts.length - 1) : W;
  const segs = [];
  let cur = [];
  pts.forEach((v, i) => {
    if (!isNum(v)) {
      if (cur.length) segs.push(cur);
      cur = [];
      return;
    }
    cur.push(`${(i * step).toFixed(1)},${(H - 3 - ((v - lo) / span) * (H - 6)).toFixed(1)}`);
  });
  if (cur.length) segs.push(cur);
  const lastI = pts.length - 1 - [...pts].reverse().findIndex(isNum);
  const lx = (lastI * step).toFixed(1);
  const ly = (H - 3 - ((pts[lastI] - lo) / span) * (H - 6)).toFixed(1);
  const lbl = Array.isArray(labels) ? labels : [];
  const words = pts.map((v, i) => `${lbl[i] ? `${lbl[i]} ` : ''}${withUnit(v, unit, digits)}`).join(', ');
  return `<span class="spark"><svg class="spark-svg" viewBox="0 0 ${W} ${H}" aria-hidden="true" focusable="false">${segs
    .map((s) => `<polyline points="${s.join(' ')}"/>`)
    .join('')}<circle cx="${lx}" cy="${ly}" r="2"/></svg><span class="sr-only">${esc(label)}: ${esc(words)}</span></span>`;
}

module.exports = { barChart, stackedBar, stripPlot, columnChart, sparkline, fmt, esc, isNum, PHASE_CLASS, NOT_TRACKED: NT };
