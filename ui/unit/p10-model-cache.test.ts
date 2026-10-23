// ui-performance-6-02: the Design Hall 3D model cache (studio3d/sources.ts)
// is bounded, never revokes a blob a live scene holds, keys `@latest` by the
// resolved version, and is emptied on Design Hall teardown.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function setup() {
  const fetched: string[] = [];
  const revoked: string[] = [];
  const heads: Record<string, string> = {};
  let n = 0;
  const mod = loadSource(
    new URL('../src/modules/design-hall/studio3d/sources.ts', import.meta.url),
    {
      '../../../lib/api/client': { authedBlobUrl: async (p: string) => `blob:legacy${p}` },
      '../../../lib/api/design': {
        getArtifact: async (id: string) => ({ artifact: { id, approved_version_id: null, head_version_id: heads[id] ?? `${id}-h1` } }),
        fetchContent: async (id: string, o: { version?: string }) => {
          fetched.push(`${id}@${o.version}`);
          return { blobUrl: `blob:${id}/${o.version}/${++n}` };
        },
        listArtifacts: async () => [],
      },
      '../model': {
        parseOttoUri: (s: string) => {
          const m = /^otto:\/\/design\/([^@#]+)(?:@(\w+))?/.exec(s);
          if (!m) return null;
          const sel = m[2] ?? 'approved';
          return { artifactId: m[1], selector: /^v\d+$/.test(sel) ? { kind: 'version', seq: Number(sel.slice(1)) } : { kind: sel } };
        },
      },
    },
    { URL: { revokeObjectURL: (u: string) => revoked.push(u) } },
  );
  return { mod, fetched, revoked, heads };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

test('a held model is shared, kept past the bound, and revoked only after its last release', async () => {
  const { mod, fetched, revoked } = setup();
  const a = mod.modelScope();
  const b = mod.modelScope();
  const u1 = await a.resolve('otto://design/m@latest');
  const u2 = await b.resolve('otto://design/m@latest');
  assert.equal(u1, u2);
  assert.equal(fetched.length, 1, 'fetched once for both scenes');
  // Fill the cache far past its bound with unheld models.
  for (let i = 0; i < 20; i++) {
    const s = mod.modelScope();
    await s.resolve(`otto://design/x${i}@latest`);
    s.release();
  }
  await settle();
  assert.ok(mod.modelCacheSize() <= 13, `bounded: ${mod.modelCacheSize()}`);
  assert.ok(revoked.length >= 7, 'evicted entries are revoked');
  assert.ok(!revoked.includes(u1), 'never revoke a blob a live scene holds');
  a.release();
  b.release();
  // Released and now the oldest: the next insert evicts and revokes it.
  const s = mod.modelScope();
  await s.resolve('otto://design/y@latest');
  await settle();
  assert.ok(revoked.includes(u1));
});

test('@latest is keyed by the resolved head version: a new version is a new entry', async () => {
  const { mod, fetched, heads } = setup();
  const s = mod.modelScope();
  await s.resolve('otto://design/m@latest');
  await s.resolve('otto://design/m@latest');
  assert.deepEqual(fetched, ['m@m-h1']);
  heads.m = 'm-h2';
  await s.resolve('otto://design/m@latest');
  assert.deepEqual(fetched, ['m@m-h1', 'm@m-h2']);
});

test('releaseModelCache revokes unheld blobs now and held ones on their release', async () => {
  const { mod, revoked } = setup();
  const held = mod.modelScope();
  const hu = await held.resolve('otto://design/h@v3');
  const free = mod.modelScope();
  const fu = await free.resolve('otto://design/f@v1');
  free.release();
  mod.releaseModelCache();
  await settle();
  assert.equal(mod.modelCacheSize(), 0);
  assert.ok(revoked.includes(fu));
  assert.ok(!revoked.includes(hu));
  held.release();
  await settle();
  assert.ok(revoked.includes(hu));
});

test('a scope released before its model resolves does not pin it', async () => {
  const { mod } = setup();
  const s = mod.modelScope();
  const p = s.resolve('otto://design/late@latest');
  s.release();
  await assert.rejects(p);
});
