import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildReportHtml, esc, fmtUsd, reportFileName, tokenCells } from '../src/modules/usage/reportHtml.ts';

const row = (o: Partial<Record<string, number>> = {}) => ({
  input_tokens: 10,
  output_tokens: 2000,
  cache_read_tokens: 300000,
  cache_write_tokens: 4000,
  total_tokens: 306010,
  cost_usd: 1.5,
  ...o,
});

function report(over: Record<string, unknown> = {}) {
  return {
    days: 7,
    generated_at: '2026-10-03T09:00:00Z',
    priced_as_of: '2026-09-25',
    scope: 'all',
    otto_only: false,
    totals: row(),
    daily: [{ day: '2026-10-03', events: 3, ...row() }],
    monthly: [{ month: '2026-10', events: 3, ...row() }],
    models: [{ provider: 'claude', model: 'claude-opus-5-5', events: 3, ...row() }],
    daily_models: [{ day: '2026-10-03', provider: 'claude', model: 'claude-opus-5-5', ...row() }],
    sessions: [
      {
        session_id: 's1',
        workspace_id: 'w1',
        provider: 'claude',
        model: 'claude-opus-5-5',
        events: 3,
        last_active: '2026-10-03 08:00:00',
        title: '<script>alert(1)</script>',
        kind: 'agent',
        workspace_name: 'Main',
        ...row(),
      },
    ],
    ...over,
  } as never;
}

test('esc escapes every HTML-significant character', () => {
  assert.equal(esc(`<a href="x">'&'</a>`), '&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;');
  assert.equal(esc(null), '');
});

test('token cells lead with tokens and end with the cost', () => {
  assert.deepEqual(tokenCells(row()), ['10', '2,000', '4,000', '300,000', '306,010', '$1.50']);
  assert.equal(fmtUsd(0), '$0');
  assert.equal(fmtUsd(0.001), '<$0.01');
});

test('the report is one self-contained, escaped HTML document', () => {
  const html = buildReportHtml(report());
  assert.ok(html.startsWith('<!doctype html>'));
  for (const h of ['Daily', 'Monthly', 'By model', 'By session']) assert.ok(html.includes(`<h2>${h}</h2>`), h);
  assert.ok(html.includes('prefers-color-scheme: dark'));
  // No external assets: no src/href pointing anywhere, no <link>/<script>.
  assert.ok(!/<link|<script|src=|href=/i.test(html));
  // Session titles are escaped, never injected.
  assert.ok(html.includes('&lt;script&gt;alert(1)&lt;/script&gt;'));
  assert.ok(html.includes('claude-opus-5-5'));
  assert.ok(html.includes('All sessions on this Mac'));
});

test('empty tables say so, and own-scope reports are labelled', () => {
  const html = buildReportHtml(report({ daily: [], sessions: [], scope: 'own' }));
  assert.ok(html.includes('No usage in this window.'));
  assert.ok(html.includes('Your sessions'));
  assert.equal(reportFileName(report()), 'otto-usage-report-7d-2026-10-03.html');
});
