// Token contrast contract, measured against the REAL values in tokens.css for
// all five theme × scheme combinations:
//   • --accent-text (accent links/labels) is >= 4.5:1 on --surface-2,
//     --surface-3 and on an --accent-soft tint (a selected row) over --surface
//     and --surface-2;
//   • --control-border (input/select outlines) is >= 3:1 against --bg,
//     --surface and --surface-2 (WCAG 1.4.11) wherever it is a measured hex
//     (the light schemes).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { contrast, mixSrgb, parseHex, type Rgb } from '../src/lib/ambient.ts';

const css = readFileSync(new URL('../src/lib/tokens.css', import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

type Scheme = 'light' | 'dark';

/** Hex custom properties that apply to <html> for a theme + scheme, in
 *  cascade order: :root, the light-tone block, then the theme block. Island
 *  subtrees (`.otto-force-dark`) are not <html> and are skipped. */
function varsFor(theme: string, scheme: Scheme, all = false): Record<string, string> {
  const out: Record<string, string> = {};
  const blocks = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({ sel: m[1].trim(), body: m[2] }));
  const applies = (sel: string): number => {
    if (sel.includes('.otto-force-dark') || sel.startsWith('.') || sel.startsWith('@')) return -1;
    if (/^:root$/.test(sel)) return 0;
    if (sel === "html[data-scheme='light']:not([data-theme='pro-dark'])") return scheme === 'light' && theme !== 'pro-dark' ? 1 : -1;
    const own = sel
      .split(',')
      .some((one) => one.includes(`data-theme='${theme}'`) && (theme === 'pro-dark' || one.includes(`data-scheme='${scheme}'`)));
    return own ? 2 : -1;
  };
  for (const level of [0, 1, 2]) {
    for (const b of blocks) {
      if (applies(b.sel) !== level) continue;
      const decl = all ? /(--[\w-]+)\s*:\s*([^;]+);/g : /(--[\w-]+)\s*:\s*(#[0-9a-fA-F]{3,6})\s*;/g;
      for (const d of b.body.matchAll(decl)) out[d[1]] = d[2].trim();
    }
  }
  return out;
}

/** The `p%` of a `--token: color-mix(in srgb, var(--base) p%, …)` value as
 *  it applies to a theme + scheme (a theme block may override the :root mix). */
function mixPct(theme: string, scheme: Scheme, token: string, base: string): number {
  const value = varsFor(theme, scheme, true)[token] ?? '';
  const m = new RegExp(`^color-mix\\(in srgb, var\\(${base}\\) (\\d+)%`).exec(value);
  assert.ok(m, `${token} is a color-mix of ${base} (got ${value})`);
  return Number(m[1]) / 100;
}

const COMBOS: { name: string; theme: string; scheme: Scheme }[] = [
  { name: 'Native light', theme: 'native', scheme: 'light' },
  { name: 'Native dark', theme: 'native', scheme: 'dark' },
  { name: 'Pro Dark', theme: 'pro-dark', scheme: 'dark' },
  { name: 'Warm light', theme: 'warm', scheme: 'light' },
  { name: 'Warm dark', theme: 'warm', scheme: 'dark' },
];

const hex = (vars: Record<string, string>, name: string): Rgb => {
  const c = parseHex(vars[name] ?? '');
  assert.ok(c, `${name} resolves to a hex value (got ${vars[name]})`);
  return c;
};

for (const combo of COMBOS) {
  const v = varsFor(combo.theme, combo.scheme);
  const accent = hex(v, '--accent');
  const text = hex(v, '--text');
  const ACCENT_SOFT = mixPct(combo.theme, combo.scheme, '--accent-soft', '--accent');
  const accentText = mixSrgb(accent, text, mixPct(combo.theme, combo.scheme, '--accent-text', '--accent'));

  test(`${combo.name}: --accent-text >= 4.5:1 on surfaces and the accent tint`, () => {
    const s1 = hex(v, '--surface');
    const s2 = hex(v, '--surface-2');
    const s3 = hex(v, '--surface-3');
    const grounds: [string, Rgb][] = [
      ['--surface-2', s2],
      ['--surface-3', s3],
      ['--accent-soft over --surface', mixSrgb(accent, s1, ACCENT_SOFT)],
      ['--accent-soft over --surface-2', mixSrgb(accent, s2, ACCENT_SOFT)],
    ];
    for (const [name, ground] of grounds) {
      const ratio = contrast(accentText, ground);
      assert.ok(ratio >= 4.5, `--accent-text on ${name} is ${ratio.toFixed(2)}:1 (< 4.5)`);
    }
  });

  if (combo.scheme === 'light') {
    test(`${combo.name}: --control-border >= 3:1 on --bg / --surface / --surface-2`, () => {
      const border = hex(v, '--control-border');
      for (const name of ['--bg', '--surface', '--surface-2']) {
        const ratio = contrast(border, hex(v, name));
        assert.ok(ratio >= 3, `--control-border on ${name} is ${ratio.toFixed(2)}:1 (< 3)`);
      }
    });
  }
}
