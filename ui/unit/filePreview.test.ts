// The shared file preview's pure rules (lib/components/filePreviewKind.ts,
// lib/render/previewText.ts) and the IDE's Editor | Split | Preview store
// (modules/ide/editor/idePreviewMode.svelte.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as kinds from '../src/lib/components/filePreviewKind.ts';
import { buildHtmlPreviewDoc, parseDelimitedTable } from '../src/lib/render/previewText.ts';

const {
  availablePreviewModes, defaultPreviewMode, effectivePreviewMode, filePreviewKind, headingOrdinalAtLine,
  htmlSandboxFor, parsePreviewModes, PREVIEW_LARGE_CHARS, previewKindForPath, previewNeedsManualRender,
  previewTextUpdate, resolvePreviewHref,
} = kinds;

test('filePreviewKind: the renderer each file gets, by name', () => {
  const cases: [string, string | null][] = [
    ['/r/README.md', 'markdown'], ['/r/doc.MARKDOWN', 'markdown'], ['/r/page.mdx', 'markdown'],
    ['/r/index.html', 'html'], ['/r/a.htm', 'html'], ['/r/logo.svg', 'svg'],
    ['/r/a.png', 'image'], ['/r/a.JPG', 'image'], ['/r/a.jpeg', 'image'], ['/r/a.gif', 'image'], ['/r/a.webp', 'image'],
    ['/r/a.avif', 'image'], ['/r/favicon.ico', 'image'], ['/r/a.bmp', 'image'], ['/r/spec.pdf', 'pdf'],
    ['/r/data.json', 'json'], ['/r/map.geojson', 'json'], ['/r/flow.mmd', 'mermaid'], ['/r/flow.mermaid', 'mermaid'],
    ['/r/t.csv', 'csv'], ['/r/t.tsv', 'tsv'], ['/r/l.jsonl', 'jsonl'], ['/r/l.ndjson', 'jsonl'],
    ['/r/main.go', null], ['/r/Makefile', null], ['/r/md', null], ['/r/a.md.go', null], ['/r/x.html?v=1', 'html'],
  ];
  for (const [p, k] of cases) assert.equal(filePreviewKind(p), k, p);
});

test('previewKindForPath keeps the Threads viewer kinds exactly (csv covers tsv, json only .json)', () => {
  assert.equal(previewKindForPath('/a/README.md'), 'markdown');
  assert.equal(previewKindForPath('/a/x.json'), 'json');
  assert.equal(previewKindForPath('/a/x.geojson'), 'code');
  assert.equal(previewKindForPath('/a/x.tsv'), 'csv');
  assert.equal(previewKindForPath('/a/x.svg'), 'svg');
  assert.equal(previewKindForPath('/a/x.webp'), 'image');
  assert.equal(previewKindForPath('/a/index.html'), 'code');
  assert.equal(previewKindForPath('/a/main.go'), 'code');
});

test('resolvePreviewHref: relative links and images resolve against the file folder', () => {
  const f = '/repo/docs/guide/README.md';
  assert.deepEqual(resolvePreviewHref(f, 'other.md', '/repo'), { path: '/repo/docs/guide/other.md', anchor: null });
  assert.deepEqual(resolvePreviewHref(f, './img/a.png', '/repo'), { path: '/repo/docs/guide/img/a.png', anchor: null });
  assert.deepEqual(resolvePreviewHref(f, '../api.md#auth-flow', '/repo'), { path: '/repo/docs/api.md', anchor: 'auth-flow' });
  assert.deepEqual(resolvePreviewHref(f, '../../../../../etc/x', '/repo'), { path: '/etc/x', anchor: null });
  // A leading `/` is the project root (GitHub README convention) …
  assert.deepEqual(resolvePreviewHref(f, '/CONTRIBUTING.md', '/repo'), { path: '/repo/CONTRIBUTING.md', anchor: null });
  // … unless it already is an absolute path inside the project.
  assert.deepEqual(resolvePreviewHref(f, '/repo/a.md', '/repo/'), { path: '/repo/a.md', anchor: null });
  assert.deepEqual(resolvePreviewHref(f, 'my%20notes.md?raw=1', '/repo'), { path: '/repo/docs/guide/my notes.md', anchor: null });
  for (const ext of ['https://x.dev/a', 'mailto:a@b.c', '//cdn.x/a.png', 'data:image/png;base64,AA', '#install', '', '   ']) {
    assert.equal(resolvePreviewHref(f, ext, '/repo'), null, ext);
  }
});

test('modes: defaults, what the control offers, and what the pane shows', () => {
  assert.equal(defaultPreviewMode('markdown'), 'source');
  assert.equal(defaultPreviewMode('html'), 'source');
  assert.equal(defaultPreviewMode('image'), 'preview');
  assert.equal(defaultPreviewMode('pdf'), 'preview');
  assert.equal(defaultPreviewMode('csv'), 'preview', 'CSV keeps opening as its table');
  assert.equal(defaultPreviewMode('jsonl'), 'source');
  assert.deepEqual(availablePreviewModes('markdown', false), ['source', 'split', 'preview']);
  assert.deepEqual(availablePreviewModes('markdown', true), ['source', 'preview']);
  assert.deepEqual(availablePreviewModes('image', false), [], 'binary files always preview');
  assert.deepEqual(availablePreviewModes(null, false), []);
  assert.equal(effectivePreviewMode('markdown', 'split', true), 'source', 'no Split on a phone');
  assert.equal(effectivePreviewMode('markdown', 'split', false), 'split');
  assert.equal(effectivePreviewMode('image', 'source', false), 'preview');
  assert.equal(effectivePreviewMode(null, 'preview', false), 'source');
});

test('parsePreviewModes drops junk and corrupt storage', () => {
  assert.deepEqual(parsePreviewModes('{"markdown":"split","html":"preview","go":"preview","json":"sideways"}'), { markdown: 'split', html: 'preview' });
  assert.deepEqual(parsePreviewModes('not json'), {});
  assert.deepEqual(parsePreviewModes('[1,2]'), {});
  assert.deepEqual(parsePreviewModes(null), {});
});

test('large files render on demand; small ones live', () => {
  assert.equal(previewNeedsManualRender('markdown', PREVIEW_LARGE_CHARS), false);
  assert.equal(previewNeedsManualRender('markdown', PREVIEW_LARGE_CHARS + 1), true);
  assert.equal(previewNeedsManualRender('html', 2_000_000), true);
  assert.equal(previewNeedsManualRender('json', 5_000_000), false, 'the JSON tree is lazy by itself');
  assert.equal(previewNeedsManualRender('image', 9_000_000), false);
});

test('previewTextUpdate: debounce typing, take a new file at once, idle while hidden', () => {
  const on = { shown: true, visible: true };
  assert.equal(previewTextUpdate('', '', '/a.md', '# hi', on), 'now', 'first show');
  assert.equal(previewTextUpdate('/a.md', '# hi', '/b.md', '# b', on), 'now', 'another file');
  assert.equal(previewTextUpdate('/a.md', '# hi', '/a.md', '# hi!', on), 'debounce', 'typing');
  assert.equal(previewTextUpdate('/a.md', '# hi', '/a.md', '# hi', on), 'same');
  assert.equal(previewTextUpdate('/a.md', '# hi', '/a.md', '# hi!', { shown: true, visible: false }), 'skip', 'hidden window');
  assert.equal(previewTextUpdate('/a.md', '# hi', '/a.md', '# hi!', { shown: false, visible: true }), 'skip', 'Source mode');
});

test('HTML frame: never same-origin; scripts only on request, still no network', () => {
  assert.equal(htmlSandboxFor(false), '');
  assert.equal(htmlSandboxFor(true), 'allow-scripts');
  for (const s of [htmlSandboxFor(false), htmlSandboxFor(true)]) {
    assert.doesNotMatch(s, /allow-same-origin|allow-top-navigation|allow-popups|allow-forms/);
  }
  const off = buildHtmlPreviewDoc('<p>x</p>');
  assert.match(off, /default-src 'none'/);
  assert.doesNotMatch(off, /script-src/);
  const on = buildHtmlPreviewDoc('<p>x</p>', { scripts: true });
  assert.match(on, /script-src 'unsafe-inline'/);
  assert.doesNotMatch(on, /connect-src|script-src[^;"]*https/);
  // Byte-identical to the Workbench preview's former inline document.
  assert.equal(
    off,
    "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: blob: https:; style-src 'unsafe-inline' https:; font-src data: https:\"><p>x</p>",
  );
});

test('headingOrdinalAtLine: the section the cursor is in, skipping fences and front matter', () => {
  const md = [
    '---', '# yaml comment', '---', // 1-3 front matter
    '# Title', //                      4  → 0
    'intro', '```sh', '# not a heading', '```', // 5-8
    'Setext', '======', //             9-10 → 1
    '- item', '---', //                11-12 (rule, not setext after a list item)
    '## Usage', //                     13 → 2
    'text', //                         14
  ].join('\n');
  assert.equal(headingOrdinalAtLine(md, 1), -1);
  assert.equal(headingOrdinalAtLine(md, 4), 0);
  assert.equal(headingOrdinalAtLine(md, 8), 0, 'a # line inside a fence is code');
  assert.equal(headingOrdinalAtLine(md, 10), 1, 'setext heading');
  assert.equal(headingOrdinalAtLine(md, 12), 1, '--- after a list item is a rule');
  assert.equal(headingOrdinalAtLine(md, 14), 2);
});

test('parseDelimitedTable (moved from threadsHelpers) still splits quoted cells', () => {
  assert.deepEqual(parseDelimitedTable('a,b\n"x, y",2\n'), { headers: ['a', 'b'], rows: [['x, y', '2']] });
  assert.deepEqual(parseDelimitedTable('a\tb\n1\t2', '\t'), { headers: ['a', 'b'], rows: [['1', '2']] });
});

function previewStore(stack = false, globals: Record<string, unknown> = {}) {
  const layout = { stack };
  const mod = loadSource(new URL('../src/modules/ide/editor/idePreviewMode.svelte.ts', import.meta.url), {
    '../../../lib/components/filePreviewKind': kinds,
    '../../../lib/components/cm-ide-manifest': { isIdeManifestPath: (p: string) => /package\.json$/.test(p) },
    '../shell/ideLayout.svelte': { ideLayout: layout },
  }, globals);
  return { store: mod.idePreviewMode, layout, key: mod.IDE_PREVIEW_MODE_KEY as string };
}

test('idePreviewMode: per-kind memory, table kinds, manifests and the phone layout', () => {
  const { store, layout } = previewStore();
  assert.equal(store.layoutFor('/r/README.md'), 'none', 'Markdown opens as source');
  assert.equal(store.layoutFor('/r/logo.png'), 'only', 'images always preview');
  assert.equal(store.layoutFor('/r/t.csv'), 'table', 'CSV keeps its table as Preview');
  assert.equal(store.kindOf('/r/package.json'), null, 'manifests keep their own analyzer switch');
  assert.equal(store.kindOf('untitled:1'), null);

  store.set('/r/README.md', 'split');
  assert.equal(store.layoutFor('/r/docs/OTHER.md'), 'split', 'remembered per kind, not per file');
  assert.equal(store.layoutFor('/r/index.html'), 'none');
  assert.equal(store.paneClass({ path: '/r/a.md', loading: false, error: null }), 'pv-split');
  assert.equal(store.paneClass({ path: '/r/a.md', loading: true, error: null }), '', 'no layout while loading');
  assert.equal(store.paneClass(null), '');

  store.toggle('/r/a.md');
  assert.equal(store.modeFor('/r/a.md'), 'preview');
  assert.equal(store.paneClass({ path: '/r/a.md', loading: false, error: null }), 'pv-only');
  store.toggle('/r/a.md');
  assert.equal(store.modeFor('/r/a.md'), 'source');

  store.set('/r/main.go', 'preview');
  assert.equal(store.modeFor('/r/main.go'), 'source', 'code files have no preview');
  store.set('/r/logo.png', 'source');
  assert.equal(store.modeFor('/r/logo.png'), 'preview', 'binary files cannot switch to source');

  layout.stack = true;
  store.set('/r/a.md', 'split');
  assert.equal(store.modeFor('/r/a.md'), 'source', 'Split is not offered on a phone');
  store.openSide('/r/a.md');
  assert.equal(store.modeFor('/r/a.md'), 'preview', '"to the side" is Preview when stacked');
  assert.deepEqual(store.available('/r/a.md'), ['source', 'preview']);
});

test('idePreviewMode persists to localStorage and a new window reads it back', () => {
  const mem = new Map<string, string>();
  const localStorage = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v), removeItem: (k: string) => void mem.delete(k) };
  const first = previewStore(false, { localStorage });
  first.store.set('/r/a.html', 'preview');
  assert.deepEqual(JSON.parse(mem.get(first.key) ?? '{}'), { html: 'preview' });
  const reopened = previewStore(false, { localStorage });
  assert.equal(reopened.store.modeFor('/r/other.html'), 'preview');
  // Storage that throws (private mode) leaves the defaults working.
  const broken = previewStore(false, { localStorage: { getItem() { throw new Error('denied'); }, setItem() { throw new Error('denied'); } } });
  broken.store.set('/r/a.md', 'preview');
  assert.equal(broken.store.modeFor('/r/a.md'), 'preview');
});
