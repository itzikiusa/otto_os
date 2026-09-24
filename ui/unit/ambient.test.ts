import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  AMBIENT_LUM,
  ambientImage,
  ambientPalette,
  ambientSamples,
  clampLuminance,
  clampPixels,
  contrast,
  luminance,
  mixSrgb,
  parseHex,
  type AmbientScheme,
  type Rgb,
} from '../src/lib/ambient.ts';

// The glass-on-ambient contract, measured against the REAL token values in
// tokens.css (parsed below) for all five theme × scheme combinations.

const css = readFileSync(new URL('../src/lib/tokens.css', import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

interface Combo {
  name: string;
  scheme: AmbientScheme;
  vars: Record<string, string>;
}

function varsFor(theme: string, scheme: AmbientScheme): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const sel = m[1];
    const root = /^\s*:root\s*$/.test(sel);
    const own = sel.includes(`data-theme='${theme}'`) && (theme === 'pro-dark' || sel.includes(`data-scheme='${scheme}'`));
    if (!root && !own) continue;
    for (const d of m[2].matchAll(/(--[\w-]+)\s*:\s*(#[0-9a-fA-F]{3,6})\s*;/g)) out[d[1]] = d[2];
  }
  return out;
}

const COMBOS: Combo[] = [
  { name: 'Native light', scheme: 'light', vars: varsFor('native', 'light') },
  { name: 'Native dark', scheme: 'dark', vars: varsFor('native', 'dark') },
  { name: 'Pro Dark', scheme: 'dark', vars: varsFor('pro-dark', 'dark') },
  { name: 'Warm light', scheme: 'light', vars: varsFor('warm', 'light') },
  { name: 'Warm dark', scheme: 'dark', vars: varsFor('warm', 'dark') },
];

function alphaOf(token: string, base: string): number {
  const re = new RegExp(`${token}:\\s*color-mix\\(in srgb, var\\(${base}\\) (\\d+)%`);
  const m = re.exec(css);
  assert.ok(m, `${token} is a color-mix of ${base}`);
  return Number(m[1]) / 100;
}
const A_SIDE = alphaOf('--glass-tint', '--bg-sidebar');
const A_BAR = alphaOf('--glass-tint-bar', '--bg');
const A_RAISED = alphaOf('--glass-tint-raised', '--surface');

const hex = (v: string | undefined): Rgb => {
  const c = parseHex(v ?? '');
  assert.ok(c, `token value ${v}`);
  return c;
};

/** Worst (lowest) contrast of `fg` on `tint@alpha` over each backdrop. */
function worstOnGlass(fg: Rgb, tint: Rgb, alpha: number, backdrops: Rgb[]): number {
  return Math.min(...backdrops.map((b) => contrast(fg, mixSrgb(tint, b, alpha))));
}

/** Extreme photo pixels after the on-device clamp. */
function photoSamples(scheme: AmbientScheme): Rgb[] {
  const raw: Rgb[] = [
    [0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255],
    [255, 255, 0], [0, 255, 255], [255, 0, 255], [128, 128, 128], [30, 60, 20],
  ];
  const data = new Uint8ClampedArray(raw.flatMap((c) => [...c, 255]));
  clampPixels(data, scheme);
  const out: Rgb[] = [];
  for (let i = 0; i < data.length; i += 4) out.push([data[i], data[i + 1], data[i + 2]]);
  return out;
}

const ACCENTS = ['#0a84ff', '#6c5ce7', '#0f9d58', '#2bb673', '#ffcc00', '#ff2d55', '#000000', '#ffffff', 'nope'];

test('tokens parse for every combination', () => {
  for (const c of COMBOS) {
    for (const k of ['--bg', '--bg-sidebar', '--surface', '--text', '--text-dim', '--accent']) {
      assert.ok(c.vars[k], `${c.name} defines ${k}`);
    }
  }
  assert.ok(A_SIDE >= 0.7 && A_BAR >= 0.7, 'chrome glass keeps at least a 70% tint over the banded backdrop');
  assert.ok(alphaOf('--glass-tint-native', '--bg-sidebar') >= 0.78, 'over native vibrancy (unbanded) the 78% rule holds');
});

test('generated palettes stay inside the scheme luminance band for any accent', () => {
  for (const scheme of ['light', 'dark'] as const) {
    const band = AMBIENT_LUM[scheme];
    for (const a of ACCENTS) {
      for (const c of ambientSamples(a, scheme)) {
        const l = luminance(c);
        assert.ok(l >= band.min && l <= band.max, `${a} ${scheme}: ${c} lum ${l.toFixed(3)} outside [${band.min}, ${band.max}]`);
      }
    }
  }
});

test('photo pixels are clamped into the band', () => {
  for (const scheme of ['light', 'dark'] as const) {
    const band = AMBIENT_LUM[scheme];
    for (const c of photoSamples(scheme)) {
      const l = luminance(c);
      assert.ok(l >= band.min && l <= band.max, `${scheme} photo pixel ${c} lum ${l.toFixed(3)}`);
    }
  }
  // Already-in-band colours are left alone.
  assert.deepEqual(clampLuminance([240, 240, 240], AMBIENT_LUM.light), [240, 240, 240]);
});

test('text and dim text on chrome glass are AA over every ambient pixel, all five combinations', () => {
  for (const c of COMBOS) {
    const backdrops = [
      ...ambientSamples(c.vars['--accent'], c.scheme),
      ...ACCENTS.flatMap((a) => ambientPalette(a, c.scheme)),
      ...photoSamples(c.scheme),
      hex(c.vars['--bg']),
    ];
    for (const [fg, name] of [[c.vars['--text'], 'text'], [c.vars['--text-dim'], 'text-dim']] as const) {
      const side = worstOnGlass(hex(fg), hex(c.vars['--bg-sidebar']), A_SIDE, backdrops);
      const bar = worstOnGlass(hex(fg), hex(c.vars['--bg']), A_BAR, backdrops);
      assert.ok(side >= 4.5, `${c.name}: --${name} on sidebar glass ${side.toFixed(2)}`);
      assert.ok(bar >= 4.5, `${c.name}: --${name} on toolbar glass ${bar.toFixed(2)}`);
    }
    // Home: --text sits directly on the backdrop (greeting, section labels).
    const raw = Math.min(...backdrops.map((b) => contrast(hex(c.vars['--text']), b)));
    assert.ok(raw >= 4.5, `${c.name}: --text on the raw backdrop ${raw.toFixed(2)}`);
  }
});

test('raised glass (menus, palette) is AA over app surfaces; --text survives any backdrop', () => {
  for (const c of COMBOS) {
    const surfaces = ['--bg', '--surface', '--surface-2', '--surface-3', '--term-bg'].map((k) => hex(c.vars[k]));
    const tint = hex(c.vars['--surface']);
    const dim = worstOnGlass(hex(c.vars['--text-dim']), tint, A_RAISED, surfaces);
    assert.ok(dim >= 4.5, `${c.name}: --text-dim on raised glass ${dim.toFixed(2)}`);
    const text = worstOnGlass(hex(c.vars['--text']), tint, A_RAISED, [[0, 0, 0], [255, 255, 255]]);
    assert.ok(text >= 4.5, `${c.name}: --text on raised glass over black/white ${text.toFixed(2)}`);
  }
});

test('ambientImage: none, a translucent wash, an opaque wallpaper, or the photo', () => {
  assert.equal(ambientImage('none', '#0a84ff', 'light'), 'none');
  const subtle = ambientImage('subtle', '#0a84ff', 'light');
  assert.match(subtle, /^radial-gradient\(/);
  assert.doesNotMatch(subtle, /linear-gradient/, 'subtle stays translucent (native vibrancy shows through)');
  assert.match(ambientImage('wallpaper', '#0a84ff', 'dark'), /linear-gradient\(#[0-9a-f]{6}, #[0-9a-f]{6}\)$/);
  assert.equal(ambientImage('wallpaper', '#0a84ff', 'dark', 'data:image/jpeg;base64,AA'), 'url("data:image/jpeg;base64,AA")');
  assert.notEqual(ambientImage('wallpaper', '#0a84ff', 'light'), ambientImage('wallpaper', '#ff2d55', 'light'), 'follows the accent');
});
