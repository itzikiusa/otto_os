// error-handling-6-05 (Design Hall): a brand kit that fails to LOAD is an
// error with Retry (not a note quoting the raw exception), and the checks
// that judge colours against the kit are skipped while only the default
// palette stands in.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { loadSource } from './sourceHarness.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const E = '../src/modules/design-hall/site/engine/';
const theme = await import(E + 'theme.ts');
const audit = await import(E + 'audit.ts');

function resolver(getArtifact: () => Promise<unknown>) {
  return loadSource(new URL('../src/modules/design-hall/site/siteApi.ts', import.meta.url), {
    '../../../lib/api/client': {}, './engine/theme': { buildTheme: () => ({ name: 'default' }) },
    '../../../lib/api/design': { getArtifact, fetchContent: async () => ({ text: '{}' }) },
  });
}

test('a kit that fails to load is an error in the user’s terms, not a raw note', async () => {
  // An ApiError-shaped 401 (duck-typed by loadErrorText across the vm realm).
  const { resolveBrand, brandLabel } = resolver(async () => {
    throw { status: 401, message: '{"code":"unauthorized"}' };
  });
  const r = await resolveBrand({ projectKitId: 'kit', artifactId: 'site' });
  assert.equal(r.note, null);
  assert.ok(r.error, 'error is set');
  assert.equal(r.error, 'Your session expired. Sign in again, then retry.');
  assert.equal(r.theme.name, 'default', 'the preview falls back to the default palette');
  assert.equal(brandLabel(r), null);
});

test('no kit linked is a note, not an error', async () => {
  const { resolveBrand } = resolver(async () => ({}));
  const r = await resolveBrand({ projectKitId: null, artifactId: 'site', docBrand: undefined });
  assert.equal(r.error, null);
  assert.match(r.note, /default palette/);
});

test('brand: false skips the contrast and off-brand checks; the rest still run', () => {
  const T = theme.buildTheme(null);
  const d = {
    type: 'otto-site', version: 1, title: 'T',
    pages: [{ id: 'home', title: 'Home', slug: '', sections: [
      { id: 'h', block: 'hero/centered', props: { headline: '' }, style: { background: '#ffff00' } },
    ] }],
  };
  const all = audit.auditPage(d, d.pages[0], T).map((f: { key: string }) => f.key.split(':')[0]);
  assert.ok(all.includes('off-brand'));
  const noKit = audit.auditPage(d, d.pages[0], T, { brand: false }).map((f: { key: string }) => f.key.split(':')[0]);
  for (const rule of ['off-brand', 'unknown-token', 'contrast-text', 'contrast-button']) assert.ok(!noKit.includes(rule), rule);
  assert.ok(noKit.includes('empty-headline'), 'non-colour checks still run');
});
