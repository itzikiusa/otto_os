#!/usr/bin/env node
// Generates lib/theme.js — the plugin's ONE copy of Otto's design tokens — from
// Otto's ui/src/lib/tokens.css, so the report palette (lib/reportmodel.js) and
// the app.css fallback check never drift from the app.
//
//   node scripts/gen-theme.js [--tokens <path/to/tokens.css>] [--out <path>] [--check]
//
// The tokens path defaults to $OTTO_TOKENS_CSS, then ../../../ui/src/lib/tokens.css
// (the plugin's home inside the Otto repo). --check exits 1 when lib/theme.js is
// stale instead of writing it.
//
// What it does: parses the custom-property blocks of tokens.css (comments
// stripped, selector lists split), composes every theme × scheme the way the
// cascade would (base :root → light-scheme tones → dark-scheme extras → the
// theme block), resolves var() and color-mix(in srgb, …) to plain hex / rgba,
// and keeps the colour tokens a static report needs plus the type/space/radius
// scale. Tokens Otto does not define (warning/success solids, on-cat, print)
// are derived below from tokens it does define, so they follow the app too.
'use strict';
const fs = require('fs');
const path = require('path');

const DEFAULT_TOKENS = path.join(__dirname, '..', '..', '..', '..', 'ui', 'src', 'lib', 'tokens.css');
const DEFAULT_OUT = path.join(__dirname, '..', 'lib', 'theme.js');

// Colour tokens copied into every scheme map, in output order.
const COLOR_KEYS = [
  'bg', 'bg-sidebar', 'surface', 'surface-2', 'surface-3', 'border', 'border-strong', 'control-border', 'hover', 'separator',
  'text', 'text-dim',
  'accent', 'accent-solid', 'accent-contrast', 'accent-text', 'accent-soft', 'accent-soft-strong', 'accent-faint', 'accent-line',
  'danger', 'danger-solid', 'danger-contrast', 'danger-soft',
  'warning', 'warning-soft', 'success', 'success-soft', 'info', 'info-soft',
  'cat-1', 'cat-2', 'cat-3', 'cat-4', 'cat-5', 'cat-6',
  'scrim', 'shadow-card', 'shadow-xs',
];
const SCALE_RE = /^(fs|sp|radius)-|^font-(ui|mono)$/;
const THEMES = ['native', 'pro-dark', 'warm'];
const SCHEMES = ['light', 'dark'];

// ------------------------------------------------------------ parsing
function stripComments(css) {
  return String(css).replace(/\/\*[\s\S]*?\*\//g, '');
}

/** Innermost `selector { decls }` blocks (media wrappers are flattened, which is fine: only exact selectors are read). */
function parseBlocks(css) {
  const out = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  for (const m of stripComments(css).matchAll(re)) {
    const decls = {};
    for (const d of m[2].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) decls[d[1].slice(2)] = d[2].replace(/\s+/g, ' ').trim();
    const selectors = m[1].trim().split(',').map((s) => s.replace(/\s+/g, ' ').trim()).filter(Boolean);
    out.push({ selectors, decls });
  }
  return out;
}

function collect(blocks, selector) {
  const acc = {};
  for (const b of blocks) if (b.selectors.includes(selector)) Object.assign(acc, b.decls);
  return acc;
}

// ------------------------------------------------------------ colours
const NAMED = { transparent: [0, 0, 0, 0], black: [0, 0, 0, 1], white: [255, 255, 255, 1] };

function parseColor(v) {
  const s = String(v).trim().toLowerCase();
  if (s in NAMED) return NAMED[s].slice();
  let m = /^#([0-9a-f]{3,8})$/.exec(s);
  if (m) {
    let h = m[1];
    if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join('');
    if (h.length !== 6 && h.length !== 8) return null;
    const ch = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
    return ch.concat(h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1);
  }
  m = /^rgba?\(([^)]+)\)$/.exec(s);
  if (m) {
    const p = m[1].split(/[\s,/]+/).filter(Boolean).map(parseFloat);
    if (p.length < 3 || p.some((x) => !Number.isFinite(x))) return null;
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }
  return null;
}

const hex2 = (n) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, '0');
function formatColor([r, g, b, a]) {
  if (a >= 0.9995) return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
  const k = (n) => Math.max(0, Math.min(255, Math.round(n)));
  return `rgba(${k(r)}, ${k(g)}, ${k(b)}, ${+a.toFixed(3)})`;
}

/** color-mix(in srgb, A [p%], B [q%]) with premultiplied alpha, as browsers do. */
function mix(a, pa, b, pb) {
  let p1 = pa;
  let p2 = pb;
  if (p1 == null && p2 == null) p1 = p2 = 0.5;
  else if (p1 == null) p1 = 1 - p2;
  else if (p2 == null) p2 = 1 - p1;
  const sum = p1 + p2;
  const scale = sum < 1 ? sum : 1; // percentages under 100% reduce alpha
  p1 /= sum;
  p2 /= sum;
  const alpha = a[3] * p1 + b[3] * p2;
  const ch = [0, 1, 2].map((i) => (alpha ? (a[i] * a[3] * p1 + b[i] * b[3] * p2) / alpha : 0));
  return ch.concat(alpha * scale);
}

/** Split on commas that are not inside parentheses. */
function splitTop(s) {
  const parts = [];
  let depth = 0;
  let cur = '';
  for (const c of s) {
    if (c === '(') depth++;
    if (c === ')') depth--;
    if (c === ',' && depth === 0) {
      parts.push(cur.trim());
      cur = '';
    } else cur += c;
  }
  if (cur.trim()) parts.push(cur.trim());
  return parts;
}

/** Find the matching ')' for the '(' at index i. */
function closeParen(s, i) {
  let depth = 0;
  for (let j = i; j < s.length; j++) {
    if (s[j] === '(') depth++;
    else if (s[j] === ')' && --depth === 0) return j;
  }
  return -1;
}

function evalMix(inner, resolveColor) {
  const [space, a, b] = splitTop(inner);
  if (!/^in srgb$/i.test(space || '') || !a || !b) throw new Error(`unsupported color-mix(${inner})`);
  const term = (t) => {
    const m = /^(.*?)(?:\s+(-?[\d.]+)%)?$/.exec(t.trim());
    return { c: resolveColor(m[1].trim()), p: m[2] == null ? null : +m[2] / 100 };
  };
  const x = term(a);
  const y = term(b);
  if (!x.c || !y.c) throw new Error(`unresolved colour in color-mix(${inner})`);
  return mix(x.c, x.p, y.c, y.p);
}

/**
 * Resolve one token value against a flat var map: var() substitution (with
 * fallbacks), then color-mix() evaluation. Non-colour values (shadows, fonts,
 * sizes) come back with their var()s substituted.
 */
function resolveValue(raw, vars, seen = new Set()) {
  let s = String(raw);
  // var(--x[, fallback]) — innermost first.
  for (let guard = 0; guard < 64 && s.includes('var('); guard++) {
    const i = s.lastIndexOf('var(');
    const j = closeParen(s, i + 3);
    if (j < 0) throw new Error(`unbalanced var() in ${raw}`);
    const [name, ...fb] = splitTop(s.slice(i + 4, j));
    const key = name.trim().replace(/^--/, '');
    let rep;
    if (key in vars) {
      if (seen.has(key)) throw new Error(`cycle through --${key}`);
      rep = resolveValue(vars[key], vars, new Set([...seen, key]));
    } else if (fb.length) rep = fb.join(', ');
    else throw new Error(`--${key} is not defined`);
    s = s.slice(0, i) + rep + s.slice(j + 1);
  }
  const color = (t) => {
    const c = parseColor(t);
    if (c) return c;
    if (/^color-mix\(/i.test(t)) return evalMix(t.slice(t.indexOf('(') + 1, closeParen(t, t.indexOf('('))), color);
    return null;
  };
  // color-mix() anywhere in the value — innermost first.
  for (let guard = 0; guard < 64 && /color-mix\(/i.test(s); guard++) {
    const i = s.toLowerCase().lastIndexOf('color-mix(');
    const j = closeParen(s, i + 9);
    s = s.slice(0, i) + formatColor(evalMix(s.slice(i + 10, j), color)) + s.slice(j + 1);
  }
  const c = parseColor(s);
  return c ? formatColor(c) : s.replace(/\s+/g, ' ').trim();
}

// ------------------------------------------------------------ composition
function themeMaps(css) {
  const blocks = parseBlocks(css);
  const root = collect(blocks, ':root');
  const lightScheme = collect(blocks, "html[data-scheme='light']:not([data-theme='pro-dark'])");
  const darkExtras = collect(blocks, "html[data-scheme='dark']");
  const proDarkExtras = collect(blocks, "html[data-theme='pro-dark']");
  const compose = (theme, scheme) => {
    const vars = { ...root };
    if (scheme === 'light' && theme !== 'pro-dark') Object.assign(vars, lightScheme);
    if (scheme === 'dark') Object.assign(vars, darkExtras);
    if (theme === 'pro-dark') Object.assign(vars, proDarkExtras);
    Object.assign(vars, collect(blocks, `html[data-theme='${theme}'][data-scheme='${scheme}']`));
    return vars;
  };
  return { root, lightScheme, compose };
}

/** Plugin-derived tokens Otto does not define, computed from ones it does. */
function derived(map, lightTones) {
  const out = {};
  // A filled warning / success badge carrying white text: the light scheme's
  // text-safe tone is dark enough for white in every scheme (as --danger-solid is).
  out['warning-solid'] = lightTones.warning;
  out['warning-contrast'] = '#ffffff';
  out['success-solid'] = lightTones.success;
  out['success-contrast'] = '#ffffff';
  // Legacy report names.
  out['on-accent'] = map['accent-contrast'];
  // Labels drawn ON a categorical fill: white on the deep light-scheme series,
  // the page colour on the bright dark-scheme ones.
  out['on-cat'] = map.__scheme === 'light' ? '#ffffff' : map.bg;
  return out;
}

function buildTheme(css) {
  const { root, compose } = themeMaps(css);
  const themes = {};
  const lightTones = {};
  {
    const v = compose('native', 'light');
    for (const k of ['warning', 'success']) lightTones[k] = resolveValue(v[k], v);
  }
  for (const t of THEMES) {
    themes[t] = {};
    for (const s of SCHEMES) {
      const vars = compose(t, s);
      const map = {};
      for (const k of COLOR_KEYS) if (k in vars) map[k] = resolveValue(vars[k], vars);
      Object.assign(map, derived({ ...map, __scheme: s }, lightTones));
      themes[t][s] = map;
    }
  }
  const scale = {};
  for (const [k, raw] of Object.entries(root)) if (SCALE_RE.test(k)) scale[k] = resolveValue(raw, root);
  const print = {
    bg: '#ffffff', surface: '#ffffff', 'surface-2': themes.native.light['surface-2'], border: '#cccccc', text: '#000000', 'text-dim': '#444444',
  };
  return { light: themes.native.light, dark: themes.native.dark, themes, print, scale };
}

function render(theme) {
  return `// GENERATED by scripts/gen-theme.js from Otto's ui/src/lib/tokens.css — do not edit.
// Regenerate: node scripts/gen-theme.js   (test/theme.test.js checks parity)
// light / dark = the native theme; themes = every theme × scheme; print =
// the paper palette; scale = type / space / radius / font tokens.
'use strict';

const deepFreeze = (o) => {
  for (const v of Object.values(o)) if (v && typeof v === 'object') deepFreeze(v);
  return Object.freeze(o);
};

module.exports = deepFreeze(${JSON.stringify(theme, null, 2)});
`;
}

function main(argv) {
  const arg = (name) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : null;
  };
  const tokens = path.resolve(arg('--tokens') || process.env.OTTO_TOKENS_CSS || DEFAULT_TOKENS);
  const out = path.resolve(arg('--out') || DEFAULT_OUT);
  const src = render(buildTheme(fs.readFileSync(tokens, 'utf8')));
  if (argv.includes('--check')) {
    const cur = fs.existsSync(out) ? fs.readFileSync(out, 'utf8') : '';
    if (cur !== src) {
      process.stderr.write(`${path.relative(process.cwd(), out)} is stale — run node scripts/gen-theme.js\n`);
      return 1;
    }
    return 0;
  }
  fs.writeFileSync(out, src);
  process.stdout.write(`wrote ${out} from ${tokens}\n`);
  return 0;
}

if (require.main === module) process.exitCode = main(process.argv.slice(2));

module.exports = { buildTheme, render, resolveValue, parseBlocks, parseColor, formatColor, mix, DEFAULT_TOKENS, COLOR_KEYS };
