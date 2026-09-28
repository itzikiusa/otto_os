// V10 / SF-14: the GFM markdown path parses the HTML into a DOM exactly once.
// It used to run marked → DOMParser (sanitize) → serialize → DOMParser again
// (to add `tabindex`) → serialize. Node has no DOM, so a counting fake
// DOMParser stands in; the full-fidelity golden (old triple path == new single
// path on real markdown) runs in the browser in e2e/desktop-docs-orch-perf.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

type FakeEl = { tag: string; attrs: Record<string, string>; setAttribute(n: string, v: string): void };
let parses = 0;
let blocks: FakeEl[] = [];

class FakeDOMParser {
  parseFromString(src: string) {
    parses++;
    const inner = src.replace(/^<body>/, '').replace(/<\/body>$/, '');
    // Only `pre`/`table` matter for the post-hook; the sanitizer's own
    // `querySelectorAll('*')` sees an empty tree (nothing to scrub).
    blocks = [...inner.matchAll(/<(pre|table)\b/g)].map((m) => ({
      tag: m[1],
      attrs: {} as Record<string, string>,
      setAttribute(n: string, v: string) { this.attrs[n] = v; },
    }));
    const body = {
      innerHTML: inner,
      querySelectorAll: (sel: string) => (sel === 'pre, table' ? blocks : []),
    };
    return { body, querySelectorAll: body.querySelectorAll };
  }
}
(globalThis as unknown as { DOMParser: unknown }).DOMParser = FakeDOMParser;

// A non-literal specifier keeps md.ts/sanitize.ts (DOM-typed browser code,
// already covered by svelte-check) out of this no-DOM tsconfig's program.
const MD_MODULE: string = '../src/lib/md.ts';
const { renderMarkdownGfm } = (await import(MD_MODULE)) as { renderMarkdownGfm: (md: string) => string };

test('renderMarkdownGfm parses the HTML once and focus-tags pre/table on that tree', () => {
  parses = 0;
  const md = '# T\n\n```js\nlet a = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n';
  const html = renderMarkdownGfm(md);
  assert.equal(parses, 1, 'one DOMParser pass (was 2)');
  assert.equal(blocks.length, 2);
  for (const b of blocks) assert.equal(b.attrs.tabindex, '0');
  assert.match(html, /<h1[^>]*>T<\/h1>/);
});

test('plain prose still takes a single parse', () => {
  parses = 0;
  renderMarkdownGfm('just **text**');
  assert.equal(parses, 1);
});
