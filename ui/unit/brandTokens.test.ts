import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  brandCssText,
  brandCssVars,
  brandPalette,
  brandStyle,
  contrast,
  contrastLevel,
  cssIdent,
  diffTokens,
  flatTokens,
  fontRoleForStyle,
  inkOf,
  isValidAsset,
  normalizeBrandDoc,
  normalizeHex,
  onColor,
  parseBrandDoc,
  parseTokenRef,
  renameKey,
  resolveToken,
  rulesForKit,
  serializeBrandDoc,
  tokenLabel,
  tokenRefs,
  uniqueName,
  validateBrandDoc,
} from '../src/modules/design-hall/brand/tokens.ts';
import { STARTER_KITS, starterKit } from '../src/modules/design-hall/brand/starters.ts';
import { brandStarter } from '../src/modules/design-hall/model.ts';

const acme = {
  $schema: 'otto-brand/1',
  name: 'Acme Brand Kit',
  color: {
    primary: { $value: '#5b3df5', $description: 'CTAs' },
    accent: { $value: '#FFB547' },
    ink: { $value: '#14122B' },
    surfaceAlt: { $value: '#fff' },
    bad: { $value: 'red' },
  },
  font: { display: { $value: '"Inter", system-ui, sans-serif', weights: [800] }, heading: { $value: 'Nope' } },
  type: { display: { size: 64, line: 72, weight: 800 }, body: { size: 17, line: 28, weight: 400 } },
  radius: { card: { $value: 14 } },
  space: { '4': { $value: 16 }, half: { $value: 2.5 } },
  logos: [],
};

test('brandCssVars uses the contract names (same as the server CSS export)', () => {
  const vars = brandCssVars(acme);
  assert.equal(vars['--brand-color-primary'], '#5B3DF5');
  assert.equal(vars['--brand-color-surface-alt'], '#FFFFFF');
  assert.equal(vars['--brand-color-bad'], undefined, 'invalid colours are skipped');
  assert.equal(vars['--brand-font-display'], '"Inter", system-ui, sans-serif');
  assert.equal(vars['--brand-font-heading'], undefined, 'unknown font roles are skipped');
  assert.equal(vars['--brand-type-display-size'], '64px');
  assert.equal(vars['--brand-type-display-line'], '72px');
  assert.equal(vars['--brand-type-display-weight'], '800');
  assert.equal(vars['--brand-radius-card'], '14px');
  assert.equal(vars['--brand-space-4'], '16px');
  assert.equal(vars['--brand-space-half'], '2.5px');
  assert.deepEqual(brandCssVars(null), {});
  assert.deepEqual(brandCssVars({ type: 'otto-brand' }), {});
  assert.match(brandCssText(acme), /^:root \{\n {2}--brand-color-primary: #5B3DF5;\n/);
  assert.equal(brandCssText({}, '.kit'), '.kit {\n}\n');
  assert.ok(brandStyle(acme).startsWith('--brand-color-primary: #5B3DF5; '));
});

test('resolveToken covers every group, sub-properties and css vars', () => {
  assert.equal(resolveToken(acme, 'token:color.primary'), '#5B3DF5');
  assert.equal(resolveToken(acme, 'color.accent'), '#FFB547');
  assert.equal(resolveToken(acme, 'token:font.display'), '"Inter", system-ui, sans-serif');
  assert.equal(resolveToken(acme, 'token:radius.card'), '14px');
  assert.equal(resolveToken(acme, 'token:space.4'), '16px');
  assert.equal(resolveToken(acme, 'token:type.display.size'), '64px');
  assert.equal(resolveToken(acme, 'token:type.display.weight'), '800');
  assert.equal(resolveToken(acme, 'token:type.display'), '800 64px/72px "Inter", system-ui, sans-serif');
  // body has no body font → falls back to the first font.
  assert.equal(resolveToken(acme, 'token:type.body'), '400 17px/28px "Inter", system-ui, sans-serif');
  assert.equal(resolveToken(acme, 'var(--brand-color-ink)'), '#14122B');
  assert.equal(resolveToken(acme, '--brand-radius-card'), '14px');
  assert.equal(resolveToken(acme, 'token:color.missing'), null);
  assert.equal(resolveToken(acme, 'token:shadow.soft'), null);
  assert.equal(resolveToken(acme, 'token:color.primary.size'), null);
  assert.equal(resolveToken({ type: { h1: { size: 32, line: 40, weight: 700 } } }, 'token:type.h1'), '700 32px/40px system-ui, sans-serif');
});

test('parseTokenRef and tokenRefs mirror the server', () => {
  assert.deepEqual(parseTokenRef('token:type.display.line'), { group: 'type', name: 'display', sub: 'line' });
  assert.deepEqual(parseTokenRef('color.primary'), { group: 'color', name: 'primary', sub: null });
  assert.equal(parseTokenRef('token:color'), null);
  assert.equal(parseTokenRef('token:nope.x'), null);
  const site = '{"bg":"token:color.primary","r":"token:radius.card","h":"token:type.display.size","x":"mytoken:color.ink"}';
  assert.deepEqual(tokenRefs(site, acme), ['color.primary', 'radius.card', 'type.display']);
  const html = '.cta{background:var(--brand-color-primary)} h1{font-size:var(--brand-type-display-size)} a{--my--brand-color-ink:1} .x{color:var(--brand-color-unknown)}';
  assert.deepEqual(tokenRefs(html, acme), ['color.primary', 'type.display']);
});

test('cssIdent kebab-cases like the Rust exporter', () => {
  assert.equal(cssIdent('surfaceAlt'), 'surface-alt');
  assert.equal(cssIdent('surface_alt'), 'surface-alt');
  assert.equal(cssIdent('XL'), 'xl');
  assert.equal(cssIdent('brandBG2'), 'brand-bg2');
  assert.equal(cssIdent('h2'), 'h2');
  assert.equal(tokenLabel('surface-alt'), 'Surface alt');
  assert.equal(tokenLabel('surfaceAlt'), 'Surface alt');
});

test('contrast: WCAG ratios, levels, ink and on-colour', () => {
  assert.equal(contrast('#000', '#fff'), 21);
  assert.equal(contrast('#fff', '#FFFFFF'), 1);
  const primary = contrast('#5B3DF5', '#FFFFFF')!;
  assert.ok(primary > 6 && primary < 6.3, String(primary));
  assert.ok(contrast('#FFB547', '#FFFFFF')! < 3);
  assert.equal(contrast('red', '#fff'), null);
  assert.equal(contrastLevel(7), 'AAA');
  assert.equal(contrastLevel(4.5), 'AA');
  assert.equal(contrastLevel(3.1), 'AA-large');
  assert.equal(contrastLevel(2), 'fail');
  assert.deepEqual(inkOf(acme), { name: 'ink', hex: '#14122B' });
  assert.deepEqual(inkOf({ color: { a: { $value: '#777' }, b: { $value: '#222' } } }), { name: 'b', hex: '#222222' });
  assert.deepEqual(inkOf({}), { name: null, hex: '#000000' });
  assert.equal(onColor('#5B3DF5', '#14122B'), '#FFFFFF');
  assert.equal(onColor('#FFB547', '#14122B'), '#14122B');
});

test('diffTokens reports changed / added / removed with canonical values', () => {
  const next = structuredClone(acme) as typeof acme & { space: Record<string, { $value: number }> };
  next.color.primary.$value = '#0F9D8A';
  delete (next.type as Record<string, unknown>).body;
  next.space.md = { $value: 16 };
  assert.deepEqual(diffTokens(acme, next), [
    { token: 'color.primary', change: 'changed', before: '#5B3DF5', after: '#0F9D8A' },
    { token: 'space.md', change: 'added', before: null, after: '16px' },
    { token: 'type.body', change: 'removed', before: '17/28/400', after: null },
  ]);
  assert.deepEqual(diffTokens(acme, acme), []);
  assert.equal(flatTokens(acme)['type.display'], '64/72/800');
});

test('validateBrandDoc mirrors the server rules', () => {
  assert.deepEqual(validateBrandDoc({ $schema: 'otto-brand/1', name: 'X', color: {}, font: {}, radius: {}, space: {}, logos: [] }), []);
  assert.deepEqual(validateBrandDoc({ type: 'otto-brand', version: 1 }), [], 'Phase 0 markers are accepted');
  const issues = validateBrandDoc({
    $schema: 'otto-brand/2',
    color: { primary: { $value: 'blue' }, 'bad name': { $value: '#fff' } },
    font: { heading: { $value: 'Inter' }, body: { $value: 'Inter; } x {' } },
    type: { display: { size: -1, line: 72, weight: 1200 } },
    radius: { card: { $value: '14px' } },
    logos: [{ name: '', kind: 'wordmark', asset: 'https://x/logo.svg' }],
    voice: { do: 'be nice' },
  });
  for (const needle of ['$schema', 'color.primary', 'color.bad name', 'font.heading', 'font.body', 'type.display.size', 'type.display.weight', 'radius.card', 'logos[0].name', 'logos[0].kind', 'logos[0].asset', 'voice.do']) {
    assert.ok(issues.some((i) => i.includes(needle)), `${needle} missing from ${JSON.stringify(issues)}`);
  }
  assert.deepEqual(validateBrandDoc([1]), ['the document must be a JSON object']);
  assert.ok(isValidAsset('otto://design/01ABC@approved'));
  assert.ok(isValidAsset(`blob:${'a'.repeat(64)}`));
  assert.ok(!isValidAsset('otto://design/@bad'));
});

test('normalizeBrandDoc tidies loose values and migrates the Phase 0 scaffold', () => {
  const legacy = {
    type: 'otto-brand',
    version: 1,
    name: 'Old kit',
    color: { primary: { $type: 'color', $value: '#4F46E5' }, ink: '#111827' },
    typography: {
      display: { $type: 'typography', $value: { fontFamily: 'system-ui', fontSize: '64px', fontWeight: 800, lineHeight: '72px' } },
      body: { $type: 'typography', $value: { fontFamily: 'Georgia; }', fontSize: '17px', fontWeight: 400, lineHeight: '28px' } },
    },
    radius: { card: { $type: 'dimension', $value: '14px' } },
    voice: { summary: 'Warm.' },
    meta: { keep: true },
  };
  const doc = normalizeBrandDoc(legacy);
  assert.equal(doc.$schema, 'otto-brand/1');
  assert.equal(doc.name, 'Old kit');
  assert.deepEqual(doc.type, { display: { size: 64, line: 72, weight: 800 }, body: { size: 17, line: 28, weight: 400 } });
  assert.deepEqual(doc.font, { display: { $value: 'system-ui' }, body: { $value: 'Georgia' } });
  assert.deepEqual(doc.radius, { card: { $value: 14 } });
  assert.deepEqual(doc.color.ink, { $value: '#111827' });
  assert.deepEqual(doc.logos, []);
  assert.equal((doc as unknown as Record<string, unknown>).version, undefined);
  assert.equal((doc as unknown as Record<string, unknown>).typography, undefined);
  assert.deepEqual((doc as unknown as Record<string, unknown>).meta, { keep: true });
  assert.deepEqual(Object.keys(doc).slice(0, 3), ['$schema', 'name', 'color']);
  assert.deepEqual(validateBrandDoc(doc), []);
  assert.equal(parseBrandDoc('nope'), null);
  assert.equal(parseBrandDoc('[1]'), null);
  assert.equal(parseBrandDoc('{}', 'Fallback')?.name, 'Fallback');
  assert.ok(serializeBrandDoc(doc).endsWith('}\n'));
});

test('every starter kit validates and has readable primaries', () => {
  assert.equal(STARTER_KITS.length, 3);
  for (const k of STARTER_KITS) {
    const doc = k.build(`${k.label} kit`);
    assert.deepEqual(validateBrandDoc(doc), [], k.id);
    assert.equal(doc.name, `${k.label} kit`);
    const p = brandPalette(doc);
    assert.ok(contrast(p.primary, '#FFFFFF')! >= 4.5, `${k.id} primary on white`);
    assert.ok(contrast(p.ink, p.surface)! >= 7, `${k.id} ink on surface`);
    assert.equal(doc.logos.length, 3);
  }
  assert.equal(starterKit('nope').id, 'vivid');
  assert.deepEqual(validateBrandDoc(brandStarter('Generic')), [], 'the generic New → Brand kit starter validates too');
});

test('editing helpers keep order and pick unique names', () => {
  assert.deepEqual(Object.keys(renameKey({ a: 1, b: 2, c: 3 }, 'b', 'x')), ['a', 'x', 'c']);
  const same = { a: 1, b: 2 };
  assert.equal(renameKey(same, 'a', 'b'), same, 'never clobbers an existing key');
  assert.equal(uniqueName('color', ['color', 'color-2']), 'color-3');
  assert.equal(uniqueName('new', []), 'new');
  assert.equal(normalizeHex('#abc'), '#AABBCC');
  assert.equal(normalizeHex('#12345'), null);
  assert.equal(fontRoleForStyle('H2'), 'display');
  assert.equal(fontRoleForStyle('codeBlock'), 'mono');
  assert.equal(fontRoleForStyle('caption'), 'body');
  const rules = [
    { key: 'a', rule: 'Amber is never text' },
    { key: 'b', rule: 'Keep contrast at AA for body copy' },
    { key: 'c', rule: 'Prefer the calm variant direction' },
    { key: 'd', rule: 'Use the surface alt background behind cards' },
  ];
  assert.deepEqual(rulesForKit(rules, { color: { amber: { $value: '#FFB547' }, surfaceAlt: { $value: '#fff' } } }).map((r) => r.key), ['a', 'b', 'd']);
  assert.deepEqual(rulesForKit(rules, {}).map((r) => r.key), ['b']);
  const pal = brandPalette({ color: { brand: { $value: '#123456' } } });
  assert.equal(pal.primary, '#123456');
  assert.equal(pal.surface, '#FFFFFF');
});
