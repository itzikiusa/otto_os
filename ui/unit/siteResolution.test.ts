import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

for (const selector of ['latest', 'approved', 'v3']) {
  test(`site brand @${selector} resolves the same version as export`, async () => {
    const versions: unknown[] = [];
    const artifact = { id: 'kit', title: 'Kit', approved_version_id: 'v1-id', head_version_id: 'v2-id' };
    const { resolveBrand } = loadSource(new URL('../src/modules/design-hall/site/siteApi.ts', import.meta.url), {
      '../../../lib/api/client': {}, './engine/theme': { buildTheme: (doc: unknown) => doc },
      '../../../lib/api/design': { getArtifact: async (_: string, options?: { version: string }) => {
        if (options) versions.push(options.version);
        return { artifact, head: { id: 'v2-id', seq: 2 }, approved: { id: 'v1-id', seq: 1 }, content: '{"name":"Kit"}' };
      } },
    });
    const result = await resolveBrand({ projectKitId: null, docBrand: `otto://design/kit@${selector}`, artifactId: 'site' });
    assert.deepEqual(versions, [selector === 'latest' ? 'v2-id' : selector === 'approved' ? 'v1-id' : 'v3']);
    assert.equal(result.seq, selector === 'latest' ? 2 : selector === 'approved' ? 1 : 3);
  });
}

test('brand content beyond inline cap loads the exact resolved version', async () => {
  const downloads: unknown[][] = [];
  const { resolveBrand } = loadSource(new URL('../src/modules/design-hall/site/siteApi.ts', import.meta.url), {
    '../../../lib/api/client': {}, './engine/theme': { buildTheme: (doc: unknown) => doc },
    '../../../lib/api/design': {
      getArtifact: async () => ({ artifact: { id: 'kit', title: 'Kit', approved_version_id: 'v1' }, approved: { seq: 1 }, content: '{"partial":', content_truncated: true }),
      fetchContent: async (...args: unknown[]) => { downloads.push(args); return { text: '{"name":"Complete kit"}' }; },
    },
  });
  const result = await resolveBrand({ projectKitId: 'kit', artifactId: 'site' });
  assert.equal(downloads.length, 1);
  assert.equal((downloads[0][1] as { version: string }).version, 'v1');
  assert.equal(result.theme.name, 'Complete kit');
  assert.equal(result.note, null);
});
