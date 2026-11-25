// P2-009: a streaming answer re-creates the <img> of its open tail on every
// flush; each used to fetch /fs/raw and mint a Blob URL of its own, revoked
// only when the block unmounted (one leaked URL per image per 32 ms).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { assetUrls } from '../src/modules/agents/conversation/assetUrls.ts';

test('one fetch and one Blob URL per path, all revoked together', async () => {
  const fetched: string[] = [];
  const revoked: string[] = [];
  const a = assetUrls(async (p) => { fetched.push(p); return `blob:${p}`; }, (u) => revoked.push(u));
  const urls = await Promise.all([a.get('/w/a.png'), a.get('/w/a.png'), a.get('/w/b.png'), a.get('/w/a.png')]);
  assert.deepEqual(urls, ['blob:/w/a.png', 'blob:/w/a.png', 'blob:/w/b.png', 'blob:/w/a.png']);
  assert.deepEqual(fetched, ['/w/a.png', '/w/b.png']);
  a.revokeAll();
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(revoked.sort(), ['blob:/w/a.png', 'blob:/w/b.png']);
});

test('a URL that lands after the block is gone is revoked too; a failed fetch revokes nothing', async () => {
  const revoked: string[] = [];
  let land: (u: string) => void = () => {};
  const a = assetUrls((p) => (p === 'bad' ? Promise.reject(new Error('404')) : new Promise<string>((r) => (land = r))), (u) => revoked.push(u));
  const late = a.get('/w/late.png');
  await a.get('bad').catch(() => {});
  a.revokeAll();
  land('blob:late');
  await late;
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(revoked, ['blob:late']);
});

test('a failed fetch is tried again on the next render (P3-012)', async () => {
  let calls = 0;
  const a = assetUrls(async (p) => { calls++; if (calls === 1) throw new Error('503'); return `blob:${p}`; }, () => {});
  await a.get('/w/a.png').catch(() => {});
  assert.equal(await a.get('/w/a.png'), 'blob:/w/a.png');
  assert.equal(calls, 2);
});
