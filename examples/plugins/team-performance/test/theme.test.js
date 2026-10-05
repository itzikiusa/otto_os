// lib/theme.js is generated from Otto's ui/src/lib/tokens.css by
// scripts/gen-theme.js. These tests pin the resolver (color-mix, var()
// fallbacks), check parity with tokens.css when the file is available (the
// plugin is also shipped outside the Otto repo — then parity is skipped), and
// check report.css's fallback block copies the generated light palette.
// Run (from the plugin dir): node --test test/theme.test.js
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const G = require('../scripts/gen-theme.js');
const THEME = require('../lib/theme.js');

const TOKENS = process.env.OTTO_TOKENS_CSS || G.DEFAULT_TOKENS;
const haveTokens = fs.existsSync(TOKENS);

test('resolver: color-mix with transparent / black / a colour, and var() fallbacks', () => {
  const vars = { accent: '#0a84ff', text: '#1d1d1f', soft: 'color-mix(in srgb, var(--accent) 14%, transparent)' };
  assert.equal(G.resolveValue('var(--soft)', vars), 'rgba(10, 132, 255, 0.14)');
  assert.equal(G.resolveValue('color-mix(in srgb, var(--accent) 84%, black)', vars), '#086fd6');
  assert.equal(G.resolveValue('color-mix(in srgb, var(--accent) 62%, var(--text))', vars), '#115daa');
  assert.equal(G.resolveValue('var(--missing, #fff)', vars), '#ffffff');
  assert.equal(G.resolveValue('0 1px 2px rgba(0, 0, 0, 0.05)', vars), '0 1px 2px rgba(0, 0, 0, 0.05)');
  assert.throws(() => G.resolveValue('var(--missing)', vars), /not defined/);
  assert.throws(() => G.resolveValue('var(--a)', { a: 'var(--b)', b: 'var(--a)' }), /cycle/);
});

test('generated theme: required tokens in every theme × scheme, all colours resolved', () => {
  const need = ['bg', 'bg-sidebar', 'surface', 'text', 'text-dim', 'accent', 'accent-solid', 'accent-text', 'danger', 'danger-solid', 'danger-contrast', 'warning-solid', 'warning-contrast', 'success-solid', 'success-contrast', 'cat-1', 'cat-6', 'on-cat', 'shadow-card'];
  for (const t of ['native', 'pro-dark', 'warm']) {
    for (const s of ['light', 'dark']) {
      const map = THEME.themes[t][s];
      for (const k of need) assert.ok(map[k], `${t}/${s} --${k}`);
      for (const [k, v] of Object.entries(map)) assert.doesNotMatch(v, /var\(|color-mix/, `${t}/${s} --${k} unresolved: ${v}`);
    }
  }
  assert.deepEqual(THEME.light, THEME.themes.native.light);
  assert.deepEqual(THEME.dark, THEME.themes.native.dark);
  assert.equal(THEME.scale['fs-hero'], '28px');
  assert.equal(THEME.scale['fs-xs'], '11px');
  assert.ok(Object.isFrozen(THEME.light), 'theme is immutable');
});

test('parity: lib/theme.js is exactly what tokens.css generates today', { skip: haveTokens ? false : `no tokens.css at ${TOKENS}` }, () => {
  const fresh = G.render(G.buildTheme(fs.readFileSync(TOKENS, 'utf8')));
  const cur = fs.readFileSync(path.join(__dirname, '..', 'lib', 'theme.js'), 'utf8');
  assert.equal(cur, fresh, 'lib/theme.js is stale — run node scripts/gen-theme.js');
});

test('parity: raw tokens.css values survive (light/dark tones, native surfaces, solids)', { skip: haveTokens ? false : 'no tokens.css' }, () => {
  const css = fs.readFileSync(TOKENS, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  const block = (sel) => {
    const i = css.indexOf(`${sel} {`);
    assert.ok(i >= 0, sel);
    return Object.fromEntries([...css.slice(i, css.indexOf('}', i)).matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
  };
  const nl = block("html[data-theme='native'][data-scheme='light']");
  const nd = block("html[data-theme='native'][data-scheme='dark']");
  const ls = block("html[data-scheme='light']:not([data-theme='pro-dark'])");
  for (const k of ['bg', 'bg-sidebar', 'surface', 'text', 'text-dim']) {
    assert.equal(THEME.light[k], G.formatColor(G.parseColor(nl[k])), `light --${k}`);
    assert.equal(THEME.dark[k], G.formatColor(G.parseColor(nd[k])), `dark --${k}`);
  }
  for (const k of ['danger', 'warning', 'success', 'info', 'cat-1', 'cat-2', 'cat-3', 'cat-4', 'cat-5', 'cat-6']) assert.equal(THEME.light[k], ls[k], `light tone --${k}`);
  const root = block(':root');
  for (const k of ['danger', 'warning', 'success', 'info', 'danger-solid', 'danger-contrast']) assert.equal(THEME.dark[k], root[k], `dark --${k}`);
  assert.equal(THEME.light['danger-solid'], root['danger-solid']);
  assert.equal(THEME.scale['fs-hero'], root['fs-hero']);
});

test('report.css fallback block copies the generated light palette and scale', () => {
  const css = fs.readFileSync(path.join(__dirname, '..', 'report', 'report.css'), 'utf8');
  const m = /\/\* tp:fallback-start \*\/\s*:where\(:root\) \{([\s\S]*?)\}\s*\/\* tp:fallback-end \*\//.exec(css);
  assert.ok(m, 'zero-specificity fallback block between tp:fallback markers');
  const decl = Object.fromEntries([...m[1].matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)].map((x) => [x[1], x[2].trim()]));
  assert.ok(Object.keys(decl).length > 20);
  for (const [k, v] of Object.entries(decl)) assert.equal(v, k in THEME.light ? THEME.light[k] : THEME.scale[k], `--${k}`);
});

test('contrast: generated solids carry their contrast colour at >= 4.5:1', () => {
  const lum = (hex) => {
    const [r, g, b] = G.parseColor(hex).slice(0, 3).map((c) => {
      const x = c / 255;
      return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const ratio = (a, b) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  for (const s of ['light', 'dark']) {
    for (const k of ['danger', 'warning', 'success']) {
      const map = THEME[s];
      assert.ok(ratio(map[`${k}-solid`], map[`${k}-contrast`]) >= 4.5, `${s} ${k}-solid`);
    }
  }
});
