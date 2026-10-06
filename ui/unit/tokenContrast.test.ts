// Token contrast contract, measured against the REAL values in tokens.css for
// all five theme × scheme combinations:
//   • --accent-text (accent links/labels) is >= 4.5:1 on --surface-2,
//     --surface-3 and on an --accent-soft tint (a selected row) over --surface
//     and --surface-2;
//   • every TEXT token (--text, --text-dim, --accent-text) is >= 4.5:1 on
//     every ground (--bg, --bg-sidebar, --surface, --surface-2, --surface-3)
//     and on the --accent-soft / --accent-soft-strong tints over each (a
//     selected row, hovered or not — a11y P3: --text-dim on soft-strong over
//     --surface-3 was 3.5–4.0:1);
//   • the Rail's count badges (semantic colour pulled toward --text on its own
//     24% tint over --bg-sidebar) are >= 4.5:1 (were 3.7–4.0 in light themes);
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

const GROUNDS = ['--bg', '--bg-sidebar', '--surface', '--surface-2', '--surface-3'];

for (const combo of COMBOS) {
  const v = varsFor(combo.theme, combo.scheme);
  const accent = hex(v, '--accent');
  const text = hex(v, '--text');
  const SOFT = mixPct(combo.theme, combo.scheme, '--accent-soft', '--accent');
  const STRONG = mixPct(combo.theme, combo.scheme, '--accent-soft-strong', '--accent');
  const tokens: [string, Rgb][] = [
    ['--text', text],
    ['--text-dim', hex(v, '--text-dim')],
    ['--accent-text', mixSrgb(accent, text, mixPct(combo.theme, combo.scheme, '--accent-text', '--accent'))],
  ];

  test(`${combo.name}: every text token >= 4.5:1 on every ground and accent tint`, () => {
    const fails: string[] = [];
    for (const g of GROUNDS) {
      const base = hex(v, g);
      const grounds: [string, Rgb][] = [
        [g, base],
        [`--accent-soft over ${g}`, mixSrgb(accent, base, SOFT)],
        [`--accent-soft-strong over ${g}`, mixSrgb(accent, base, STRONG)],
      ];
      for (const [tn, fg] of tokens) {
        for (const [gn, ground] of grounds) {
          const r = contrast(fg, ground);
          if (r < 4.5) fails.push(`${tn} on ${gn}: ${r.toFixed(2)}`);
        }
      }
    }
    assert.deepEqual(fails, [], fails.join('; '));
  });

  // Rail badges: color-mix(--success|--warning 70%, --text) on
  // color-mix(same 24%, --bg-sidebar) — Rail.svelte .rail-badge.
  const all = varsFor(combo.theme, combo.scheme);
  if (parseHex(all['--warning'] ?? '') && parseHex(all['--success'] ?? '')) {
    test(`${combo.name}: Rail count badges >= 4.5:1`, () => {
      const sidebar = hex(v, '--bg-sidebar');
      for (const name of ['--success', '--warning']) {
        const c = hex(all, name);
        const r = contrast(mixSrgb(c, text, 0.7), mixSrgb(c, sidebar, 0.24));
        assert.ok(r >= 4.5, `${name} badge is ${r.toFixed(2)}:1 (< 4.5)`);
      }
    });
  }
}

// Semantic tones: tokens.css promises the bare --danger/--warning/--success/
// --info are TEXT-SAFE (chips, Badge tones, LoadState stale bars — 11–12 px
// text) on every ground and on their own -soft tint (14%) over each ground.
const TONES = ['--danger', '--warning', '--success', '--info'];
for (const combo of COMBOS) {
  const all = varsFor(combo.theme, combo.scheme);
  test(`${combo.name}: semantic tones >= 4.5:1 on every ground and their own -soft tint`, () => {
    const fails: string[] = [];
    for (const tone of TONES) {
      const fg = hex(all, tone);
      const pct = mixPct(combo.theme, combo.scheme, `${tone}-soft`, tone);
      for (const g of GROUNDS) {
        const base = hex(all, g);
        for (const [gn, ground] of [
          [g, base],
          [`${tone}-soft over ${g}`, mixSrgb(fg, base, pct)],
        ] as [string, Rgb][]) {
          const r = contrast(fg, ground);
          if (r < 4.5) fails.push(`${tone} on ${gn}: ${r.toFixed(2)}`);
        }
      }
    }
    assert.deepEqual(fails, [], fails.join('; '));
  });

  // Switch.svelte: OFF is an outlined track (--surface-2 fill, --text-dim
  // ring) with a --text-dim knob; ON is an --accent-solid / --success track
  // with an --accent-contrast knob (--bg on the success track: the bright dark-
  // scheme green under a white knob was 1.9:1). WCAG 1.4.11: every state cue >= 3:1.
  test(`${combo.name}: Switch off/on parts >= 3:1 (WCAG 1.4.11)`, () => {
    const fails: string[] = [];
    const dim = hex(all, '--text-dim');
    const offTrack = hex(all, '--surface-2');
    const knob = hex(all, '--accent-contrast');
    const accent = hex(all, '--accent');
    const solidMix = (all['--accent-solid'] ?? '').match(/color-mix\(in srgb, var\(--accent\) (\d+)%, black\)/);
    const onTrack = solidMix ? mixSrgb(accent, [0, 0, 0], Number(solidMix[1]) / 100) : accent;
    const check = (name: string, a: Rgb, b: Rgb) => {
      const r = contrast(a, b);
      if (r < 3) fails.push(`${name}: ${r.toFixed(2)}`);
    };
    check('off knob on off track', dim, offTrack);
    for (const g of ['--bg', '--surface', '--surface-2']) check(`off ring on ${g}`, dim, hex(all, g));
    check('on knob on accent track', knob, onTrack);
    check('on knob (--bg) on success track', hex(all, '--bg'), hex(all, '--success'));
    assert.deepEqual(fails, [], fails.join('; '));
  });
}

// S19-301: the focus ring is drawn in --accent-text (>= 4.5:1 on every ground
// above, so well past WCAG 1.4.11's 3:1). --accent-solid — the darkened FILL
// colour — is under 3:1 on dark grounds and must not draw a ring
// (ui-guards `focus-ring-token` catches local rules).
test('the global :focus-visible ring uses --accent-text', () => {
  const app = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  const rule = /(^|\n):focus-visible\s*\{([^}]*)\}/.exec(app);
  assert.ok(rule, 'app.css has a global :focus-visible rule');
  assert.match(rule[2], /outline:\s*2px solid var\(--accent-text\)/);
  for (const combo of COMBOS) {
    const v = varsFor(combo.theme, combo.scheme);
    const accent = hex(v, '--accent');
    const ring = mixSrgb(accent, hex(v, '--text'), mixPct(combo.theme, combo.scheme, '--accent-text', '--accent'));
    for (const g of GROUNDS) {
      const r = contrast(ring, hex(v, g));
      assert.ok(r >= 3, `${combo.name}: focus ring on ${g} is ${r.toFixed(2)}:1 (< 3)`);
    }
  }
});
