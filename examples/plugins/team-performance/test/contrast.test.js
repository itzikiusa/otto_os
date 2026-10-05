// WCAG 2.x contrast for the token fallback set in ui/app.css: every text
// token must read at >= 4.5:1 on every surface it can sit on, in both themes.
// Values are Otto's (ui/src/lib/tokens.css), so this also guards the copy.
'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const css = fs.readFileSync(path.join(__dirname, '..', 'ui', 'app.css'), 'utf8');
const block = /\/\* tp:fallback-start \*\/([\s\S]*?)\/\* tp:fallback-end \*\//.exec(css)[1];

function parseSet(re) {
  const m = re.exec(block);
  assert.ok(m, 'token set present');
  const out = {};
  for (const d of m[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[d[1]] = d[2].trim();
  return out;
}
const THEMES = {
  dark: parseSet(/:root\[data-theme='dark'\]\s*\{([^}]*)\}/),
  light: parseSet(/:root\[data-theme='light'\]\s*\{([^}]*)\}/),
};

function rgba(v) {
  let m = /^#([0-9a-f]{6})$/i.exec(v);
  if (m) return [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16)).concat(1);
  m = /^#([0-9a-f]{3})$/i.exec(v);
  if (m) return [...m[1]].map((c) => parseInt(c + c, 16)).concat(1);
  m = /^rgba?\(([^)]+)\)$/.exec(v);
  if (m) {
    const p = m[1].split(',').map((x) => parseFloat(x));
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }
  throw new Error('unparsed colour ' + v);
}
/** Composite fg (maybe translucent) over an opaque bg. */
const over = (fg, bg) => [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3])).concat(1);
const lum = ([r, g, b]) => {
  const c = [r, g, b].map((x) => {
    const s = x / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
function ratio(a, b) {
  const [l1, l2] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

// Text tokens (body copy, links, status text) need 4.5:1 (WCAG 1.4.3).
const TEXT = ['--text', '--text-dim', '--accent-text', '--danger', '--warning', '--success', '--info'];
// Category colours are graphical marks (bars, swatches, lines): 3:1 (WCAG 1.4.11).
const CATS = ['--cat-1', '--cat-2', '--cat-3', '--cat-4', '--cat-5', '--cat-6'];
const SURFACES = ['--bg', '--surface', '--surface-2', '--surface-3'];

test('ratio() matches known WCAG values', () => {
  assert.strictEqual(Math.round(ratio(rgba('#000000'), rgba('#ffffff')) * 10) / 10, 21);
  assert.strictEqual(Math.round(ratio(rgba('#777777'), rgba('#ffffff')) * 100) / 100, 4.48);
});

for (const [name, t] of Object.entries(THEMES)) {
  test(`${name}: text tokens reach 4.5:1 on every surface`, () => {
    const fails = [];
    for (const s of SURFACES) {
      const bg = rgba(t[s]);
      for (const k of TEXT) {
        const r = ratio(over(rgba(t[k]), bg), bg);
        if (r < 4.5) fails.push(`${k} on ${s} = ${r.toFixed(2)}`);
      }
    }
    // Hover rows: table/menu text must still read on the translucent wash.
    for (const s of ['--bg', '--surface', '--surface-2']) {
      const hov = over(rgba(t['--hover']), rgba(t[s]));
      for (const k of ['--text', '--text-dim']) {
        const r = ratio(rgba(t[k]), hov);
        if (r < 4.5) fails.push(`${k} on ${s}+hover = ${r.toFixed(2)}`);
      }
    }
    assert.deepStrictEqual(fails, []);
  });

  test(`${name}: category (chart) colours reach 3:1 against every surface`, () => {
    const fails = [];
    for (const s of SURFACES) for (const k of CATS) {
      const r = ratio(rgba(t[k]), rgba(t[s]));
      if (r < 3) fails.push(`${k} on ${s} = ${r.toFixed(2)}`);
    }
    assert.deepStrictEqual(fails, []);
  });

  test(`${name}: light-theme --cat-2 is the deepened amber`, { skip: name !== 'light' }, () => {
    assert.strictEqual(t['--cat-2'].toLowerCase(), '#9a5b00');
  });

  test(`${name}: white text on the solid accent (primary buttons) reaches 4.5:1`, () => {
    const r = ratio(rgba(t['--accent-contrast']), rgba(t['--accent-solid']));
    assert.ok(r >= 4.5, `accent-contrast on accent-solid = ${r.toFixed(2)}`);
  });

  test(`${name}: direct bar labels (--bg on phase colours) reach 3:1 (bold, non-text-critical: value repeated in <desc> and table)`, () => {
    for (const k of ['--cat-1', '--cat-3', '--cat-4', '--cat-5', '--cat-6']) {
      const r = ratio(rgba(t['--bg']), rgba(t[k]));
      assert.ok(r >= 3, `${k}: ${r.toFixed(2)}`);
    }
  });
}
