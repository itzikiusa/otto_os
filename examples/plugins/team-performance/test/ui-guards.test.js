// UI guard rails (docs/design/guidelines): the plugin UI must never use
// native confirm/alert/prompt (no-ops in the Tauri webview), never set a font
// below 11px, and never hard-code a hex colour outside app.css's token
// fallback block (between the tp:fallback markers).
'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const UI = path.join(__dirname, '..', 'ui');

function walk(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...walk(p));
    else if (/\.(html|css|js)$/.test(e.name)) out.push(p);
  }
  return out;
}
const files = walk(UI).map((f) => ({ f, rel: path.relative(UI, f), src: fs.readFileSync(f, 'utf8') }));
const lineOf = (src, idx) => src.slice(0, idx).split('\n').length;
function scan(re, fn) {
  const hits = [];
  for (const { rel, src } of files) {
    const text = fn ? fn(rel, src) : src;
    for (const m of text.matchAll(re)) hits.push(`${rel}:${lineOf(text, m.index)} ${m[0].trim()}`);
  }
  return hits;
}

const NATIVE_DIALOG = /(?:^|[^\w.$])(?:confirm|alert|prompt)\s*\(|\bwindow\.(?:confirm|alert|prompt)\s*\(/gm;
const SMALL_FONT = /font-size\s*[:=]\s*["']?\s*(\d+(?:\.\d+)?)px/g;
const HEX = /(?<![&\w])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/g;

/** Blank out the fallback block (keeping line numbers) so hex there is allowed. */
const stripFallback = (rel, src) => src.replace(/\/\* tp:fallback-start \*\/[\s\S]*?\/\* tp:fallback-end \*\//g, (m) => m.replace(/[^\n]/g, ' '));

test('ui/ has files to guard', () => {
  assert.ok(files.some((x) => x.rel === 'index.html'));
  assert.ok(files.some((x) => x.rel === 'app.css'));
});

test('no native confirm/alert/prompt in ui/', () => {
  assert.deepStrictEqual(scan(NATIVE_DIALOG), []);
});

test('no font size below 11px in ui/', () => {
  const hits = [];
  for (const { rel, src } of files) {
    for (const m of src.matchAll(SMALL_FONT)) if (parseFloat(m[1]) < 11) hits.push(`${rel}:${lineOf(src, m.index)} ${m[0]}`);
  }
  assert.deepStrictEqual(hits, []);
});

test('no hex colours outside the app.css fallback block', () => {
  assert.deepStrictEqual(scan(HEX, stripFallback), []);
});

test('fallback block defines complete light and dark token sets', () => {
  const css = files.find((x) => x.rel === 'app.css').src;
  const block = /\/\* tp:fallback-start \*\/([\s\S]*?)\/\* tp:fallback-end \*\//.exec(css);
  assert.ok(block, 'fallback block present');
  const sets = { dark: /:root\[data-theme='dark'\]\s*\{([^}]*)\}/, light: /:root\[data-theme='light'\]\s*\{([^}]*)\}/ };
  const need = ['--bg', '--surface', '--surface-2', '--border', '--text', '--text-dim', '--danger', '--warning', '--success', '--info', '--cat-1'];
  for (const [name, re] of Object.entries(sets)) {
    const m = re.exec(block[1]);
    assert.ok(m, `${name} set present`);
    for (const t of need) assert.ok(m[1].includes(t + ':'), `${name} set defines ${t}`);
  }
});

test('the guards themselves catch violations', () => {
  assert.ok('if (confirm("x"))'.match(NATIVE_DIALOG));
  assert.ok('window.alert(1)'.match(NATIVE_DIALOG));
  assert.ok(!'confirmer.ask({})'.match(NATIVE_DIALOG));
  assert.ok(!'TP.confirmer.ask()'.match(NATIVE_DIALOG));
  assert.ok('color: #fff;'.match(HEX));
  assert.ok(!'&#39;'.match(HEX));
  assert.ok(!"querySelector('#ce-days')".match(HEX));
  assert.ok([...'font-size: 10px'.matchAll(SMALL_FONT)].some((m) => parseFloat(m[1]) < 11));
});
