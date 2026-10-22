// maintainability-3-14: Excalidraw labels written HTML-escaped by an agent are
// decoded ONCE through the shared `unescapeHtml` (lib/html.ts) — quotes too, so
// a board never shows a literal `&quot;`, and `&amp;lt;` stays `&lt;`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { escapeHtml, unescapeHtml } from '../src/lib/html.ts';
import { loadSource } from './sourceHarness.ts';

test('unescapeHtml inverts escapeHtml exactly once', () => {
  const raw = `<a href="x" title='y'>&amp;</a>`;
  assert.equal(unescapeHtml(escapeHtml(raw)), raw);
  assert.equal(unescapeHtml('&amp;lt;'), '&lt;', 'decoded once, never twice');
  assert.equal(unescapeHtml('&apos;&copy;'), "'&copy;", 'unknown entities are left as written');
});

test('Excalidraw labels decode quotes, not only & < >', () => {
  const build = loadSource(new URL('../src/lib/excalidraw/excalidraw-build.ts', import.meta.url), {
    '../html': { unescapeHtml },
  });
  const els = build.buildExcalidrawElements([
    { type: 'rectangle', id: 'a', x: 0, y: 0, width: 100, height: 40, label: { text: 'Say &quot;hi&quot; &amp; &#39;bye&#39;' } },
  ]);
  const text = els.find((e: { type: string }) => e.type === 'text');
  assert.equal(text?.text, `Say "hi" & 'bye'`);
});
