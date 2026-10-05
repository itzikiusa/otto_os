// Deterministic, accessible SVG charts for the HTML report. Rendered
// server-side (no client JS): every chart is a <figure> holding an SVG with
// role="img" + <title>/<desc>, a legend, and a "Show as table" <details> with
// the exact numbers. Colours come from CSS classes (.s1….s6 → --cat-1…6) so
// light/dark/print follow the tokens. null values are "not tracked" — they are
// drawn as a dashed outline (never a zero bar) and spelled out in the table.
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

function table(headers, rows) {
  return `<div class="tbl-wrap"><table><thead><tr>${headers.map((h, i) => `<th scope="col"${i ? ' class="n"' : ''}>${esc(h)}</th>`).join('')}</tr></thead><tbody>${rows
    .map((r) => `<tr>${r.map((c, i) => (i ? `<td class="n">${c}</td>` : `<th scope="row">${c}</th>`)).join('')}</tr>`)
    .join('')}</tbody></table></div>`;
}

function frame(id, title, desc, svgBody, height, legend, tbl) {
  const tid = `${id}-t`;
  const did = `${id}-d`;
  return `<figure class="chart" id="${esc(id)}"><figcaption>${esc(title)}</figcaption>${legend || ''}<svg viewBox="0 0 640 ${height}" role="img" aria-labelledby="${tid} ${did}" preserveAspectRatio="xMinYMin meet"><title id="${tid}">${esc(title)}</title><desc id="${did}">${esc(desc)}</desc>${svgBody}</svg><details class="as-table"><summary>Show as table</summary>${tbl}</details></figure>`;
}

// Horizontal bars, one per row. rows: [{label, value, note?}]
function barChart({ id, title, desc, rows, unit = '', digits = 1, series = 1 }) {
  const list = Array.isArray(rows) ? rows : [];
  const max = Math.max(0, ...list.map((r) => (isNum(r.value) ? r.value : 0))) || 1;
  const LBL = 170;
  const W = 640 - LBL - 70;
  const H = 22;
  let y = 6;
  const parts = [];
  for (const r of list) {
    parts.push(`<text class="lbl" x="${LBL - 8}" y="${y + 15}" text-anchor="end">${esc(String(r.label).slice(0, 28))}</text>`);
    if (isNum(r.value)) {
      const w = Math.max(r.value > 0 ? 1 : 0, Math.round((Math.max(0, r.value) / max) * W));
      parts.push(`<rect class="s${((series - 1) % 6) + 1}" x="${LBL}" y="${y + 3}" width="${w}" height="${H - 6}" rx="3"/><text class="val" x="${LBL + w + 6}" y="${y + 15}">${esc(fmt(r.value, digits))}${unit ? ` ${esc(unit)}` : ''}</text>`);
    } else {
      parts.push(`<rect class="nt-bar" x="${LBL}" y="${y + 3}" width="${Math.round(W / 4)}" height="${H - 6}" rx="3"/><text class="lbl" x="${LBL + Math.round(W / 4) + 6}" y="${y + 15}">${NT}</text>`);
    }
    y += H;
  }
  parts.unshift(`<line class="axis" x1="${LBL}" y1="2" x2="${LBL}" y2="${y + 2}"/>`);
  const tbl = table(['', unit ? `Value (${unit})` : 'Value'], list.map((r) => [esc(r.label), cell(r.value, '', digits)]));
  return frame(id, title, desc, parts.join(''), y + 8, '', tbl);
}

// Stacked horizontal bars. series: [name]; rows: [{label, values:[number|null]}].
// A null segment is excluded from the bar and flagged "not tracked" in the table.
function stackedBar({ id, title, desc, series, rows, unit = '', digits = 1 }) {
  const names = Array.isArray(series) ? series : [];
  const list = Array.isArray(rows) ? rows : [];
  const total = (r) => (r.values || []).reduce((a, v) => a + (isNum(v) && v > 0 ? v : 0), 0);
  const max = Math.max(0, ...list.map(total)) || 1;
  const LBL = 170;
  const W = 640 - LBL - 70;
  const H = 24;
  let y = 6;
  const parts = [];
  for (const r of list) {
    parts.push(`<text class="lbl" x="${LBL - 8}" y="${y + 16}" text-anchor="end">${esc(String(r.label).slice(0, 28))}</text>`);
    let x = LBL;
    (r.values || []).forEach((v, i) => {
      if (!isNum(v) || v <= 0) return;
      const w = Math.max(1, Math.round((v / max) * W));
      parts.push(`<rect class="s${(i % 6) + 1}" x="${x}" y="${y + 3}" width="${w}" height="${H - 6}"><title>${esc(names[i] || '')}: ${esc(fmt(v, digits))}${unit ? ` ${esc(unit)}` : ''}</title></rect>`);
      x += w;
    });
    parts.push(`<text class="val" x="${x + 6}" y="${y + 16}">${esc(fmt(total(r), digits))}${unit ? ` ${esc(unit)}` : ''}</text>`);
    y += H;
  }
  const legend = `<div class="legend" aria-hidden="true">${names.map((n, i) => `<span><i class="s${(i % 6) + 1}"></i>${esc(n)}</span>`).join('')}</div>`;
  const tbl = table(['', ...names.map((n) => (unit ? `${n} (${unit})` : n))], list.map((r) => [esc(r.label), ...names.map((_, i) => cell((r.values || [])[i], '', digits))]));
  return frame(id, title, desc, parts.join(''), y + 8, legend, tbl);
}

// Vertical columns over time. points: [{label, value}]
function columnChart({ id, title, desc, points, unit = '', digits = 1 }) {
  const list = Array.isArray(points) ? points : [];
  const max = Math.max(0, ...list.map((p) => (isNum(p.value) ? p.value : 0))) || 1;
  const H = 160;
  const n = Math.max(1, list.length);
  const step = 600 / n;
  const bw = Math.max(2, Math.min(36, step * 0.7));
  const parts = [`<line class="axis" x1="20" y1="${H}" x2="620" y2="${H}"/>`];
  list.forEach((p, i) => {
    const cx = 20 + step * i + step / 2;
    if (isNum(p.value)) {
      const h = Math.round((Math.max(0, p.value) / max) * (H - 20));
      parts.push(`<rect class="s1" x="${(cx - bw / 2).toFixed(1)}" y="${H - h}" width="${bw.toFixed(1)}" height="${h}" rx="2"><title>${esc(p.label)}: ${esc(fmt(p.value, digits))}</title></rect>`);
    }
    if (n <= 16 || i % Math.ceil(n / 16) === 0) parts.push(`<text class="lbl" x="${cx.toFixed(1)}" y="${H + 14}" text-anchor="middle">${esc(String(p.label).slice(-5))}</text>`);
  });
  const tbl = table(['', unit ? `Value (${unit})` : 'Value'], list.map((p) => [esc(p.label), cell(p.value, '', digits)]));
  return frame(id, title, desc, parts.join(''), H + 22, '', tbl);
}

module.exports = { barChart, stackedBar, columnChart, fmt, esc, isNum, NOT_TRACKED: NT };
