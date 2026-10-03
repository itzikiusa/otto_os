// Self-contained HTML export of the usage report (GET /usage/report), in the
// spirit of ccusage's daily/monthly/session tables: tokens first, cost as a
// secondary column. Pure (no Svelte, no DOM) so it is unit-testable under
// `node --test`; every interpolated string goes through `esc`.

import type { UsageReport, TokenTotals } from '../../lib/api/usage.svelte';

/** HTML-escape text for element content and double-quoted attributes. */
export function esc(v: unknown): string {
  return String(v ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Full integer with thousands separators (report tables want exact counts). */
export function fmtInt(n: number): string {
  return Math.round(n || 0).toLocaleString('en-US');
}

/** "$12.34" / "<$0.01" / "$0". */
export function fmtUsd(n: number): string {
  if (!n) return '$0';
  if (n < 0.01) return '<$0.01';
  return '$' + n.toFixed(2);
}

type Row = {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
};

/** The five token columns + cost, in ccusage's order. */
export const TOKEN_HEADERS = ['Input', 'Output', 'Cache write', 'Cache read', 'Total tokens', 'Cost (est.)'];

export function tokenCells(r: Row): string[] {
  return [
    fmtInt(r.input_tokens),
    fmtInt(r.output_tokens),
    fmtInt(r.cache_write_tokens),
    fmtInt(r.cache_read_tokens),
    fmtInt(r.total_tokens),
    fmtUsd(r.cost_usd),
  ];
}

function table(caption: string, lead: string[], rows: { lead: string[]; r: Row }[], totals?: Row): string {
  if (rows.length === 0) {
    return `<section><h2>${esc(caption)}</h2><p class="dim">No usage in this window.</p></section>`;
  }
  const head = [...lead, ...TOKEN_HEADERS]
    .map((h, i) => `<th${i >= lead.length ? ' class="num"' : ''} scope="col">${esc(h)}</th>`)
    .join('');
  const body = rows
    .map(
      ({ lead: l, r }) =>
        `<tr>${l.map((c) => `<td>${esc(c)}</td>`).join('')}${tokenCells(r)
          .map((c, i) => `<td class="num${i === 5 ? ' cost' : ''}">${esc(c)}</td>`)
          .join('')}</tr>`,
    )
    .join('\n');
  const foot = totals
    ? `<tfoot><tr><th scope="row"${lead.length > 1 ? ` colspan="${lead.length}"` : ''}>Total</th>${tokenCells(totals)
        .map((c, i) => `<td class="num${i === 5 ? ' cost' : ''}">${esc(c)}</td>`)
        .join('')}</tr></tfoot>`
    : '';
  return `<section><h2>${esc(caption)}</h2><div class="scroll"><table><thead><tr>${head}</tr></thead><tbody>
${body}
</tbody>${foot}</table></div></section>`;
}

function sum(rows: Row[]): TokenTotals {
  const t: TokenTotals = { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 0, cost_usd: 0 };
  for (const r of rows) {
    t.input_tokens += r.input_tokens;
    t.output_tokens += r.output_tokens;
    t.cache_read_tokens += r.cache_read_tokens;
    t.cache_write_tokens += r.cache_write_tokens;
    t.total_tokens += r.total_tokens;
    t.cost_usd += r.cost_usd;
  }
  return t;
}

/** Build the downloadable report: one HTML file, inline CSS, no external
 *  assets, light/dark via `prefers-color-scheme`. */
export function buildReportHtml(report: UsageReport): string {
  const scope = report.scope === 'own' ? 'Your sessions' : report.otto_only ? 'Otto sessions' : 'All sessions on this Mac';
  const t = report.totals;
  const tiles: [string, number][] = [
    ['Total tokens', t.total_tokens],
    ['Input', t.input_tokens],
    ['Output', t.output_tokens],
    ['Cache write', t.cache_write_tokens],
    ['Cache read', t.cache_read_tokens],
  ];
  const sections = [
    table(
      'Daily',
      ['Date'],
      report.daily.map((d) => ({ lead: [d.day], r: d })),
      sum(report.daily),
    ),
    table(
      'Monthly',
      ['Month'],
      report.monthly.map((m) => ({ lead: [m.month], r: m })),
      sum(report.monthly),
    ),
    table(
      'By model',
      ['Provider', 'Model'],
      report.models.map((m) => ({ lead: [m.provider, m.model || 'unknown'], r: m })),
      sum(report.models),
    ),
    table(
      'Daily by model',
      ['Date', 'Provider', 'Model'],
      report.daily_models.map((d) => ({ lead: [d.day, d.provider, d.model || 'unknown'], r: d })),
    ),
    table(
      'By session',
      ['Session', 'Workspace', 'Provider / model', 'Last active'],
      report.sessions.map((s) => ({
        lead: [s.title || s.session_id, s.workspace_name || (s.workspace_id === 'external' ? 'External' : s.workspace_id), `${s.provider} · ${s.model || 'unknown'}`, s.last_active],
        r: s,
      })),
    ),
  ].join('\n');

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Otto usage report — last ${esc(report.days)} days</title>
<style>
:root { --bg: #ffffff; --fg: #1d1d1f; --dim: #6e6e73; --line: #e5e5ea; --card: #f5f5f7; --accent: #0a66d8; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #1c1c1e; --fg: #f2f2f7; --dim: #a1a1a6; --line: #3a3a3c; --card: #2c2c2e; --accent: #4c9bff; }
}
* { box-sizing: border-box; }
body { margin: 0; padding: 24px 16px 48px; background: var(--bg); color: var(--fg);
  font: 13px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; }
main { max-width: 1100px; margin: 0 auto; }
h1 { font-size: 22px; margin: 0 0 4px; }
h2 { font-size: 15px; margin: 28px 0 8px; }
.dim { color: var(--dim); }
.tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 8px; margin: 16px 0; }
.tile { background: var(--card); border-radius: 8px; padding: 10px 12px; }
.tile b { display: block; font-size: 18px; font-variant-numeric: tabular-nums; }
.tile span { color: var(--dim); font-size: 12px; }
.scroll { overflow-x: auto; }
table { border-collapse: collapse; width: 100%; font-variant-numeric: tabular-nums; }
th, td { padding: 5px 8px; border-bottom: 1px solid var(--line); text-align: start; white-space: nowrap; }
thead th { color: var(--dim); font-weight: 600; font-size: 12px; }
tfoot th, tfoot td { font-weight: 700; border-top: 2px solid var(--line); }
.num { text-align: end; }
.cost { color: var(--dim); }
footer { margin-top: 32px; font-size: 12px; }
</style>
</head>
<body>
<main>
<h1>Otto usage report</h1>
<p class="dim">${esc(scope)} · last ${esc(report.days)} days · generated ${esc(report.generated_at)}</p>
<div class="tiles">
${tiles.map(([label, v]) => `<div class="tile"><span>${esc(label)}</span><b>${esc(fmtInt(v))}</b></div>`).join('\n')}
<div class="tile"><span>Estimated cost</span><b>${esc(fmtUsd(t.cost_usd))}</b></div>
</div>
${sections}
<footer class="dim">Token counts come from the agent CLIs' own transcripts. Cost is an estimate at published list prices as of ${esc(report.priced_as_of)}; subscription plans are not billed per token.</footer>
</main>
</body>
</html>
`;
}

/** File name for the download: `otto-usage-report-30d-2026-10-03.html`. */
export function reportFileName(report: UsageReport): string {
  const day = (report.generated_at || '').slice(0, 10) || 'report';
  return `otto-usage-report-${report.days}d-${day}.html`;
}
