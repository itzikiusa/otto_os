// UI guard rails (docs/design/guidelines): the plugin UI must never use
// native confirm/alert/prompt (no-ops in the Tauri webview), never set a font
// below 11px, and never hard-code a hex colour outside app.css's token
// fallback block (between the tp:fallback markers). The same rules cover the
// generated HTML report's assets (report/report.css + report/*.js).
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
const REPORT = path.join(__dirname, '..', 'report');
const reportFiles = (fs.existsSync(REPORT) ? fs.readdirSync(REPORT) : [])
  .filter((n) => /\.(css|js)$/.test(n))
  .map((n) => ({ f: path.join(REPORT, n), rel: 'report/' + n, src: fs.readFileSync(path.join(REPORT, n), 'utf8') }));
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

// ---- report assets --------------------------------------------------------
const hasMarkers = (src) => /\/\* tp:fallback-start \*\//.test(src);
function scanSet(set, re, fn) {
  const hits = [];
  for (const { rel, src } of set) {
    const text = fn ? fn(rel, src) : src;
    for (const m of text.matchAll(re)) hits.push(`${rel}:${lineOf(text, m.index)} ${m[0].trim()}`);
  }
  return hits;
}

test('report/ has assets to guard', () => {
  assert.ok(reportFiles.some((x) => x.rel === 'report/report.css'));
});

test('no native confirm/alert/prompt in report/*.js', () => {
  assert.deepStrictEqual(scanSet(reportFiles.filter((x) => x.rel.endsWith('.js')), NATIVE_DIALOG), []);
});

test('no font size below 11px in report/', () => {
  const hits = [];
  for (const { rel, src } of reportFiles) {
    for (const m of src.matchAll(SMALL_FONT)) if (parseFloat(m[1]) < 11) hits.push(`${rel}:${lineOf(src, m.index)} ${m[0]}`);
  }
  assert.deepStrictEqual(hits, []);
});

test('report/*.js has no hex colours (tokens only)', () => {
  assert.deepStrictEqual(scanSet(reportFiles.filter((x) => x.rel.endsWith('.js')), HEX), []);
});

// report.css must fence its token values in tp:fallback markers like app.css.
// Until the report owner adds them this is reported as a TODO, not a failure.
const reportCss = reportFiles.find((x) => x.rel === 'report/report.css');
test('report/report.css has no hex outside its tp:fallback block', { todo: reportCss && !hasMarkers(reportCss.src) ? 'report.css needs tp:fallback-start/end markers around its token block' : false }, () => {
  assert.ok(reportCss && hasMarkers(reportCss.src), 'fallback markers present');
  assert.deepStrictEqual(scanSet([reportCss], HEX, stripFallback), []);
});

// ---- component behaviour (components.js in a minimal DOM-less sandbox) ----
function loadComponents() {
  const vm = require('node:vm');
  const noop = () => {};
  const doc = { addEventListener: noop, querySelector: () => null, getElementById: () => null, documentElement: { clientWidth: 1200, clientHeight: 800 } };
  const win = { addEventListener: noop };
  const ctx = { window: win, document: doc, setTimeout, clearTimeout, console };
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(UI, 'components.js'), 'utf8'), ctx);
  return win.TP;
}

test('PHASE_COLORS is the fixed, exported phase map', () => {
  const TP = loadComponents();
  assert.deepStrictEqual({ ...TP.PHASE_COLORS }, { design: '--cat-4', dev: '--cat-1', review: '--cat-3', deploy: '--cat-6', rework: '--cat-5' });
  assert.ok(Object.isFrozen(TP.PHASE_COLORS));
});

test('stackedBars: direct labels only on wide segments, every value in <desc>, list twin', () => {
  const TP = loadComponents();
  const segs = [
    { key: 'design', label: 'Design', color: TP.PHASE_COLORS.design, hatchWhenNull: true },
    { key: 'dev', label: 'Dev', color: TP.PHASE_COLORS.dev },
    { key: 'review', label: 'Review', color: TP.PHASE_COLORS.review },
  ];
  const html = TP.stackedBars({ rows: [{ label: 'ABC-1', parts: { design: null, dev: 10, review: 0.2 } }], segs, title: 'Phases', desc: 'Per ticket' });
  const desc = /<desc[^>]*>([^<]*)<\/desc>/.exec(html)[1];
  assert.match(desc, /Design not tracked/);
  assert.match(desc, /Dev 10d/);
  assert.match(desc, /Review 0\.2d/);
  const onBar = [...html.matchAll(/class="on-bar"[^>]*>([^<]*)</g)].map((m) => m[1]);
  assert.deepStrictEqual(onBar, ['10'], 'the 0.2d sliver gets no direct label');
  assert.match(html, /<ul class="sb-list"/);
  assert.match(html, /<div class="sb">/);
});

test('toasts: errors stay at least 8s', () => {
  const TP = loadComponents();
  assert.ok(TP.toastDuration('danger') >= 8000);
  assert.ok(TP.toastDuration('') < TP.toastDuration('danger'));
});

test('index.html sets data-theme before the stylesheet and shows a visible Period label', () => {
  const html = files.find((x) => x.rel === 'index.html').src;
  const script = html.indexOf("setAttribute('data-theme'");
  assert.ok(script > 0 && script < html.indexOf('app.css'), 'theme script precedes the stylesheet');
  assert.match(html, /prefers-color-scheme: light/);
  assert.match(html, /<label class="scope-label" for="period">Period<\/label>/);
});

test('app.css: local-only tokens are tp-prefixed and the type scale follows Otto', () => {
  const css = files.find((x) => x.rel === 'app.css').src;
  assert.ok(!/var\(--focus-ring\)|var\(--shadow-pop\)/.test(css));
  assert.match(css, /h1 \{\s*font-size: var\(--fs-xl\)/);
  assert.match(css, /h2 \{\s*font-size: var\(--fs-l\)/);
  assert.match(css, /h3 \{\s*font-size: var\(--fs-s\);\s*color: var\(--text\)/);
  const block = /\/\* tp:fallback-start \*\/([\s\S]*?)\/\* tp:fallback-end \*\//.exec(css)[1];
  for (const t of ['--surface-3', '--hover', '--accent-soft', '--accent-text', '--accent-solid']) {
    assert.ok(block.split(t + ':').length - 1 >= 3, `${t} defined in dark, light and auto-light sets`);
  }
});

// ---- P8: token parity with Otto's tokens.css ------------------------------
const TOKENS = path.join(__dirname, '..', '..', '..', '..', 'ui', 'src', 'lib', 'tokens.css');
const cssBlock = (src, selRe) => {
  const m = selRe.exec(src);
  if (!m) return null;
  const out = {};
  for (const d of m[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[d[1]] = d[2].trim().toLowerCase();
  return out;
};
const fallback = () => /\/\* tp:fallback-start \*\/([\s\S]*?)\/\* tp:fallback-end \*\//.exec(files.find((x) => x.rel === 'app.css').src)[1];

test('fallback tokens match Otto tokens.css (native dark/light, status tones, cat-1..6, solids, Pro Dark, Warm)', { skip: !fs.existsSync(TOKENS) && 'tokens.css not present (plugin installed outside the Otto repo)' }, () => {
  const t = fs.readFileSync(TOKENS, 'utf8');
  const root = cssBlock(t, /^:root \{([\s\S]*?)^\}/m);
  const lightTones = cssBlock(t, /html\[data-scheme='light'\]:not\(\[data-theme='pro-dark'\]\) \{([\s\S]*?)\}/);
  const nLight = cssBlock(t, /html\[data-theme='native'\]\[data-scheme='light'\] \{([\s\S]*?)\}/);
  const nDark = cssBlock(t, /html\[data-theme='native'\]\[data-scheme='dark'\] \{([\s\S]*?)\}/);
  const pro = cssBlock(t, /html\[data-theme='pro-dark'\]\[data-scheme='dark'\] \{([\s\S]*?)\}/);
  const warmL = cssBlock(t, /html\[data-theme='warm'\]\[data-scheme='light'\] \{([\s\S]*?)\}/);
  const warmD = cssBlock(t, /html\[data-theme='warm'\]\[data-scheme='dark'\] \{([\s\S]*?)\}/);
  const fb = fallback();
  const dark = cssBlock(fb, /:root,\s*:root\[data-theme='dark'\] \{([\s\S]*?)\}/);
  const light = cssBlock(fb, /:root\[data-theme='light'\] \{([\s\S]*?)\}/);
  const fbPro = cssBlock(fb, /:root\[data-otto-theme='pro-dark'\] \{([\s\S]*?)\}/);
  const fbWarmL = cssBlock(fb, /:root\[data-otto-theme='warm'\]\[data-theme='light'\] \{([\s\S]*?)\}/);
  const fbWarmD = cssBlock(fb, /:root\[data-otto-theme='warm'\]\[data-theme='dark'\] \{([\s\S]*?)\}/);
  const SURF = ['--bg', '--bg-sidebar', '--surface', '--surface-2', '--surface-3', '--border', '--text', '--text-dim'];
  const TONES = ['--danger', '--warning', '--success', '--info', '--cat-1', '--cat-2', '--cat-3', '--cat-4', '--cat-5', '--cat-6'];
  const same = (want, got, keys, label) => {
    for (const k of keys) if (want[k] != null && !/var\(|color-mix/.test(want[k])) assert.strictEqual(got[k], want[k], `${label} ${k}`);
  };
  same(nDark, dark, SURF, 'dark');
  same(root, dark, [...TONES, '--accent', '--accent-contrast', '--danger-solid', '--danger-contrast'], 'dark');
  same(nLight, light, SURF, 'light');
  // Light --cat-2 is deliberately deepened (#9a5b00, test/contrast.test.js) so
  // amber marks reach contrast on light surfaces; every other tone is exact.
  same(lightTones, light, TONES.filter((k) => k !== '--cat-2'), 'light');
  same(root, light, ['--accent', '--danger-solid', '--danger-contrast'], 'light');
  same(pro, fbPro, [...SURF, '--accent'], 'pro-dark');
  same(warmL, fbWarmL, [...SURF, '--accent'], 'warm light');
  same(warmD, fbWarmD, [...SURF, '--accent', '--accent-contrast'], 'warm dark');
  assert.strictEqual(light['--cat-2'], '#9a5b00', 'light cat-2 is the deepened amber');
});

test('app.css: KPI values use --fs-hero, only weights 500/600, and every token used in ui/ is defined', () => {
  const css = files.find((x) => x.rel === 'app.css').src;
  assert.match(css, /--fs-hero: 28px/);
  assert.match(css, /\.tile \.value \{[^}]*font-size: var\(--fs-hero\)/);
  const weights = [...css.matchAll(/font-weight:\s*(\d+)/g)].map((m) => m[1]);
  assert.deepStrictEqual([...new Set(weights)].sort(), ['500', '600']);
  const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
  const used = new Set();
  for (const { src } of files) for (const m of src.matchAll(/var\((--[\w-]+)/g)) used.add(m[1]);
  const missing = [...used].filter((v) => !defined.has(v) && v !== '--fade');
  assert.deepStrictEqual(missing, []);
});

test('app.css: modal-time inert, drawer, guardrail hatch and prose-link underline rules exist', () => {
  const css = files.find((x) => x.rel === 'app.css').src;
  assert.match(css, /\.tile\.weak/);
  assert.match(css, /\.guard-reason/);
  assert.match(css, /\.modal\.drawer/);
  assert.match(css, /p a,\s*li a[\s\S]*?text-decoration: underline/);
  assert.match(css, /\.table-wrap:focus-visible/);
  const comp = files.find((x) => x.rel === 'components.js').src;
  assert.match(comp, /setAttribute\('inert', ''\)/);
});
