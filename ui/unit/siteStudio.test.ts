// Site Studio engine (ui/src/modules/design-hall/site/engine/*): schema
// validation, the renderer's escaping + link hygiene, brand tokens → theme,
// document ops, templates, audit and the co-design selection payload. The
// engine modules import each other extension-less (Vite style), so a tiny
// resolve hook maps `./x` → `./x.ts` before they are loaded.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { readFileSync } from 'node:fs';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const E = '../src/modules/design-hall/site/engine/';
const catalog = await import(E + 'catalog.ts');
const render = await import(E + 'render.ts');
const theme = await import(E + 'theme.ts');
const validate = await import(E + 'validate.ts');
const ops = await import(E + 'ops.ts');
const templates = await import(E + 'templates.ts');
const audit = await import(E + 'audit.ts');
const assist = await import(E + 'assist.ts');

const T = theme.buildTheme(null);
const ctx = (over: Record<string, unknown> = {}) => ({ theme: T, ...over });

function doc(sections: any[], extra: Record<string, unknown> = {}): any {
  return { type: 'otto-site', version: 1, title: 'Test', pages: [{ id: 'home', title: 'Home', slug: '', sections }], ...extra };
}

test('site.css is byte-identical to the Rust crate copy (the export ships the same stylesheet)', () => {
  const ui = readFileSync(new URL('../src/modules/design-hall/site/engine/site.css', import.meta.url), 'utf8');
  const rust = readFileSync(new URL('../../crates/otto-design/src/site/site.css', import.meta.url), 'utf8');
  assert.equal(ui, rust);
  // Everything is scoped: no bare element / :root / body selectors that could leak into the app.
  const selectors = ui
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, '')
    .match(/(^|\})\s*([^{}@][^{}]*)\{/g)!
    .map((m) => m.replace(/^\}?\s*/, '').replace(/\{$/, '').trim())
    .filter((sel) => !sel.startsWith('@'));
  // Split a selector list on top-level commas only (not inside :where(…)).
  const parts = (sel: string) => {
    const out: string[] = [];
    let depth = 0;
    let cur = '';
    for (const ch of sel) {
      if (ch === '(') depth++;
      if (ch === ')') depth--;
      if (ch === ',' && depth === 0) {
        out.push(cur);
        cur = '';
      } else cur += ch;
    }
    return [...out, cur];
  };
  for (const sel of selectors) for (const part of parts(sel)) assert.match(part.trim(), /^(:where\()?\.os-/, `unscoped selector: ${part}`);
});

test('the catalog: ~25 section blocks across every family, all renderable', () => {
  assert.ok(catalog.SECTIONS.length >= 25, `${catalog.SECTIONS.length} blocks`);
  for (const fam of catalog.FAMILIES) assert.ok(catalog.SECTIONS.some((s: any) => s.family === fam.id), fam.id);
  assert.ok(catalog.sectionDef('media/3d-embed'), 'a 3D embed block exists');
  const taken = new Set<string>();
  for (const def of catalog.SECTIONS) {
    const s = ops.makeSection(def.block, taken);
    for (const b of s.blocks) assert.ok(catalog.itemDef(b.block), `${def.block} child ${b.block} is a known block`);
    const html = render.renderSection(s, ctx());
    assert.match(html, /^<(section|header|footer) id="[^"]+" class="os-sec os-/, def.block);
    assert.ok(!html.includes('data-os-'), `${def.block}: no editor hooks in export mode`);
    assert.ok(!/<script/i.test(html), `${def.block}: never a script`);
    const edit = render.renderSection(s, ctx({ editable: true }));
    assert.ok(edit.includes(`data-os-section="${s.id}"`), `${def.block}: canvas hook`);
    assert.deepEqual(validate.validateSite(doc([s])), [], `${def.block} starter validates`);
  }
});

test('the renderer escapes copy and sanitizes links and images', () => {
  const s = ops.makeSection('hero/split', new Set());
  s.props.headline = '<script>alert(1)</script> & "you"';
  s.props.primary_href = 'javascript:alert(1)';
  s.props.secondary_label = 'Tiers';
  s.props.secondary_href = 'page:tiers';
  const html = render.renderSection(s, ctx({ pageHref: (id: string) => `${id}.html` }));
  assert.ok(html.includes('&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;you&quot;'));
  assert.ok(!html.includes('javascript:'));
  assert.ok(html.includes('href="tiers.html"'), 'page: links resolve through the host');
  assert.equal(render.safeHref(' JaVa\tScRiPt:alert(1)'), '#');
  assert.equal(render.safeHref('//evil.example/x'), '#');
  assert.equal(render.safeHref('mailto:a@b.c'), 'mailto:a@b.c');
  assert.equal(render.safeHref('#faq'), '#faq');
  assert.equal(render.safeSrc('data:text/html;base64,PHNjcmlwdD4='), null);
  assert.equal(render.safeSrc('data:image/png;base64,iVBORw0KGgo='), 'data:image/png;base64,iVBORw0KGgo=');
  assert.equal(render.safeSrc('http://insecure.example/a.png'), null);
  assert.equal(render.safeSrc('otto://design/IMG1', { asset: () => 'blob:x' }), 'blob:x');
  assert.equal(render.safeVideo('https://cdn.example/v.mp4'), 'https://cdn.example/v.mp4');
});

test('hidden sections stay in the document but never reach the export', () => {
  const s = ops.makeSection('cta/band', new Set());
  s.hidden = true;
  assert.equal(render.renderSection(s, ctx()), '');
  assert.match(render.renderSection(s, ctx({ editable: true })), /os-hidden/);
  s.hidden = false;
  s.responsive = { hide: ['mobile'], stack: 'media-first', mobile_align: 'center' };
  const html = render.renderSection(s, ctx());
  assert.match(html, /os-hide-mobile/);
  assert.match(html, /os-stack-first/);
  assert.match(html, /os-malign-center/);
});

test('3D embeds show the artifact (poster, version, policy) or a stand-in card', () => {
  const s = ops.makeSection('hero/split', new Set());
  s.blocks[0].props.src = 'otto://design/CARD@approved';
  const info = { title: 'Rewards Card 3D', seq: 7, policy: 'follow_approved', poster: 'assets/card.png' };
  const html = render.renderSection(s, ctx({ embed: () => info, editable: true }));
  assert.match(html, /<img class="os-embed-3d__poster" src="assets\/card.png"/);
  assert.match(html, /Rewards Card 3D · v7 · follows approved/);
  const stand = render.renderSection(ops.makeSection('media/3d-embed', new Set()), ctx({ editable: true }));
  assert.match(stand, /os-card3d/);
  assert.match(stand, /Pick a 3D artifact/);
});

test('brand kits (otto-brand/1 and the legacy typography shape) become --brand-* tokens and theme slots', () => {
  const kit = {
    $schema: 'otto-brand/1',
    color: { primary: { $value: '#0F766E' }, accent: { $value: '#F59E0B' }, ink: { $value: '#0B1220' }, surface: { $value: '#FFFFFF' }, surfaceAlt: { $value: '#ECFDF5' } },
    font: { display: { $value: '"Clash Display", system-ui' }, body: { $value: 'Inter, system-ui' } },
    radius: { card: { $value: '20px' } },
    space: { m: { $value: '16px' } },
    logos: [{ src: 'x' }],
    voice: { summary: 'Warm' },
  };
  const t = theme.buildTheme(kit, 'Teal kit');
  const byVar = Object.fromEntries(t.tokens.map((k: any) => [k.cssVar, k.value]));
  assert.equal(byVar['--brand-color-primary'], '#0F766E');
  assert.equal(byVar['--brand-color-surface-alt'], '#ECFDF5', 'camelCase names become kebab-case');
  assert.equal(byVar['--brand-font-display'], '"Clash Display", system-ui');
  assert.equal(byVar['--brand-radius-card'], '20px');
  assert.ok(!Object.keys(byVar).some((k) => k.startsWith('--brand-logos') || k.startsWith('--brand-voice')));
  assert.equal(t.slotCss.primary, 'var(--brand-color-primary)');
  assert.equal(t.slotCss['surface-alt'], 'var(--brand-color-surface-alt)');
  assert.equal(t.primary, '#0F766E');
  const css = render.themeCss(t);
  assert.match(css, /^\.os-site \{\n {2}--brand-color-primary: #0F766E;/);
  assert.match(css, /--os-primary: var\(--brand-color-primary\);/);
  // Legacy composite typography → a font token.
  const legacy = theme.buildTheme({ typography: { display: { $value: { fontFamily: 'Georgia' } } } });
  assert.equal(legacy.fontDisplay, 'Georgia');
  // Values can't break out of a declaration.
  assert.equal(theme.cssValue('red; } body { x: y'), 'red  body  x: y');
  // No kit → defaults, and a sensible surface-alt.
  assert.equal(T.primary, theme.DEFAULT_SLOTS.primary);
  assert.equal(T.surfaceAlt, theme.DEFAULT_SLOTS.surfaceAlt);
});

test('tone and contrast follow the brand colours', () => {
  assert.equal(theme.resolveBackground('token:color.ink', T).tone, 'dark');
  assert.equal(theme.resolveBackground('', T).tone, 'light');
  assert.equal(theme.resolveBackground('gradient:ink', T).preset, 'ink');
  const raw = theme.resolveBackground('#123456', T);
  assert.equal(raw.offBrand, true);
  assert.equal(theme.resolveBackground('token:color.nope', T).unknownToken, 'color.nope');
  assert.equal(theme.contrast('#000000', '#ffffff'), 21);
  const c = theme.sectionContrast({ style: { background: 'token:color.surface' } }, T);
  assert.ok(c.text! >= 4.5 && c.button! >= 4.5, JSON.stringify(c));
});

test('the validator gates structure, ids, blocks, enums and unsafe URLs', () => {
  assert.deepEqual(validate.validateSite({ type: 'otto-site', version: 1 }), [], 'the stored default (no pages) is valid');
  const bad = validate.validateSite({ type: 'nope', version: 2 });
  assert.deepEqual(bad.map((i: any) => i.path), ['type', 'version']);
  const s1 = ops.makeSection('features/grid', new Set());
  const dup = ops.clone(s1);
  const issues = validate.validateSite(doc([s1, dup]));
  assert.ok(issues.some((i: any) => /duplicate id/.test(i.message)));
  const x = ops.makeSection('hero/centered', new Set());
  x.block = 'hero/fancy';
  x.props.primary_href = 'javascript:alert(1)';
  x.style = { background: 'red', motion: 'spin' } as any;
  const got = validate.validateSite(doc([x])).map((i: any) => i.path);
  assert.ok(got.some((p: string) => p.endsWith('.block')));
  assert.ok(got.some((p: string) => p.endsWith('.props.primary_href')));
  assert.ok(got.some((p: string) => p.endsWith('.style.background')));
  assert.ok(got.some((p: string) => p.endsWith('.style.motion')));
  assert.ok(validate.validateSite(doc([], { brand: 'https://evil' })).some((i: any) => i.path === 'brand'));
  assert.deepEqual(validate.validateSite(doc([], { brand: 'otto://design/KIT1@approved' })), []);
});

test('six starter templates, all valid, readable ids, brand tokens only', () => {
  assert.deepEqual(
    templates.SITE_TEMPLATES.map((t: any) => t.id),
    ['landing', 'product-launch', 'event', 'portfolio', 'docs-home', 'waitlist'],
  );
  for (const t of templates.SITE_TEMPLATES) {
    const d = t.build('My site');
    assert.deepEqual(validate.validateSite(d), [], t.id);
    assert.equal(d.pages[0].slug, '');
    assert.ok(d.pages[0].sections.length >= 5, `${t.id} has a real page`);
    assert.ok(d.pages[0].sections.some((s: any) => s.block.startsWith('hero/')), `${t.id} has a hero (h1)`);
    for (const s of d.pages[0].sections) {
      const bg = s.style?.background ?? '';
      assert.ok(!bg.startsWith('#'), `${t.id}/${s.id}: no raw colours in templates`);
    }
    const errors = audit.auditPage(d, d.pages[0], T).filter((f: any) => f.level === 'error');
    assert.deepEqual(errors, [], `${t.id} passes contrast/alt checks with the default palette`);
  }
  assert.deepEqual(templates.starterSite(undefined, 'Blank').pages, []);
});

test('document ops never mutate their input and keep ids unique', () => {
  const d0 = templates.starterSite('landing', 'Rewards+');
  const frozen = JSON.stringify(d0);
  const hero = d0.pages[0].sections[1].id;
  const { doc: d1, id: copyId } = ops.duplicateSection(d0, hero);
  assert.equal(JSON.stringify(d0), frozen, 'input untouched');
  assert.equal(d1.pages[0].sections[2].id, copyId);
  const ids = ops.allIds(d1);
  let count = 0;
  for (const p of d1.pages) for (const s of p.sections) count += 1 + (s.blocks?.length ?? 0);
  assert.equal(ids.size, count + d1.pages.length);
  const d2 = ops.moveSection(d1, copyId!, -2);
  assert.equal(d2.pages[0].sections[0].id, copyId);
  const d3 = ops.swapLayout(d2, hero, 'hero/centered');
  const swapped = ops.findSection(d3, hero)!.section;
  assert.equal(swapped.block, 'hero/centered');
  assert.equal(swapped.props.headline, 'Every purchase moves you up.', 'copy survives a layout swap');
  assert.equal(swapped.blocks![0].block, 'embed/3d', 'media carries over');
  const { doc: d4, id: item } = ops.addItem(d3, 'features');
  assert.equal(ops.findBlock(d4, item)!.section.id, 'features');
  const d5 = ops.setStyle(d4, 'features', 'motion', 'tilt-hover');
  assert.equal(ops.findSection(d5, 'features')!.section.style!.motion, 'tilt-hover');
  assert.deepEqual(validate.validateSite(d5), []);
  const { doc: d6 } = ops.addPage(d5, 'Tiers');
  const { doc: d7 } = ops.addPage(d6, 'Tiers');
  assert.deepEqual(d7.pages.map((p: any) => p.slug), ['', 'tiers', 'tiers-2']);
  assert.deepEqual(validate.validateSite(d7), []);
});

test('"From your library" copies carry provenance as a derived_from reference', () => {
  const src = templates.starterSite('landing', 'Spring Promo').pages[0].sections.find((s: any) => s.id === 'faq');
  const copy = ops.fromLibrary(src, 'SITE123', 12, new Set(['faq']));
  assert.notEqual(copy.id, 'faq');
  assert.equal(copy.derived_from, 'otto://design/SITE123@v12#faq');
  assert.deepEqual(ops.parseDerived(copy.derived_from), { artifactId: 'SITE123', seq: 12, node: 'faq' });
  assert.deepEqual(validate.validateSite(doc([copy])), []);
});

test('parseSite reads empty and legacy docs, and reports invalid ones', () => {
  assert.deepEqual(ops.parseSite('{"type":"otto-site","version":1}').doc!.pages, []);
  assert.equal(ops.parseSite('{nope').doc, null);
  assert.ok(ops.parseSite('{"type":"otto-site","version":1,"pages":[{"id":"a b"}]}').issues.length > 0);
  assert.ok(ops.isEmptySite(ops.parseSite(null).doc!));
});

test('audit flags low contrast, off-brand colours and missing text alternatives', () => {
  const s = ops.makeSection('cta/band', new Set());
  s.style = { background: '#7A7A7A' };
  const img = ops.makeSection('media/gallery', new Set());
  img.blocks[0].props.src = 'https://cdn.example/a.jpg';
  img.blocks[0].props.alt = '';
  const d = doc([ops.makeSection('hero/split', new Set()), s, img]);
  const keys = audit.auditPage(d, d.pages[0], T).map((f: any) => f.key.split(':')[0]);
  assert.ok(keys.includes('off-brand'));
  assert.ok(keys.includes('contrast-button') || keys.includes('contrast-text'), keys.join(','));
  assert.ok(keys.includes('alt'));
});

test('the co-design selection payload names the node and stays under the assist cap', () => {
  const d = templates.starterSite('landing', 'Rewards+');
  const sel = assist.assistSelection(d, { pageId: 'home', sectionId: 'hero', blockId: null })!;
  assert.equal(sel.section!.name, 'Hero');
  assert.equal(sel.path, 'pages[0].sections[1]');
  assert.ok(sel.node, 'small sections travel with their JSON');
  const b = assist.assistSelection(d, { pageId: 'home', sectionId: 'tiers', blockId: 'tiers-2' })!;
  assert.equal(b.block!.block, 'item/tier');
  assert.equal(b.path, 'pages[0].sections[5].blocks[1]');
  assert.ok(JSON.stringify(b).length <= assist.ASSIST_SELECTION_CAP);
  assert.equal(assist.selectionLabel(d, { pageId: 'home', sectionId: 'hero', blockId: null }), 'Section: Hero');
  assert.equal(assist.assistSelection(null, { pageId: null, sectionId: null, blockId: null }), null);
});
