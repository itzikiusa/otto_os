import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// F4/F5/F11 (perf): a 5 MB Excalidraw scene (one pasted image) saves under
// budget, its history lists without parsing documents, and N versions store
// the image once (restore rehydrates it). Needs a daemon built from this
// branch (OTTO_E2E_BIN=target/debug/ottod).

const PUT_BUDGET_MS = 300;
const VERSIONS_BUDGET_MS = 100;

function excalidraw(label: string, dataURL: string) {
  return {
    type: 'otto-canvas',
    version: 1,
    format: 'excalidraw',
    source: JSON.stringify({
      type: 'excalidraw',
      version: 2,
      elements: [{ id: label, type: 'rectangle' }],
      files: { f1: { id: 'f1', mimeType: 'image/png', dataURL } },
    }),
  };
}

test('5 MB canvas scene: save and history stay under budget', async () => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const img = `data:image/png;base64,${'Q'.repeat(5 * 1024 * 1024)}`;
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspace}/canvas/scenes`, {
    data: { title: 'Big board', doc: excalidraw('e0', img) },
  });
  expect(created.ok()).toBeTruthy();
  const scene = await created.json();

  const times: number[] = [];
  for (let i = 1; i <= 5; i++) {
    const t0 = Date.now();
    const put = await ctx.put(`${base}/api/v1/canvas/scenes/${scene.id}?summary=true`, {
      data: { doc: excalidraw(`e${i}`, img) },
    });
    times.push(Date.now() - t0);
    expect(put.status()).toBe(200);
  }
  times.sort((a, b) => a - b);
  const p50 = times[Math.floor(times.length / 2)];
  console.log(`canvas 5 MB PUT ms: ${times.join(', ')}`);
  expect(p50, `5 MB PUT p50 ${p50} ms`).toBeLessThan(PUT_BUDGET_MS);

  const t0 = Date.now();
  const res = await ctx.get(`${base}/api/v1/canvas/scenes/${scene.id}/versions`);
  const listMs = Date.now() - t0;
  expect(res.ok()).toBeTruthy();
  const versions = await res.json();
  expect(versions.length).toBeGreaterThanOrEqual(1);
  expect(versions[0].format).toBe('excalidraw');
  expect(versions[0].size).toBeGreaterThan(5 * 1024 * 1024);
  expect(listMs, `GET versions ${listMs} ms`).toBeLessThan(VERSIONS_BUDGET_MS);

  // Restore puts the original (image included) back.
  const restored = await ctx.post(`${base}/api/v1/canvas/scenes/${scene.id}/versions/${versions[0].id}/restore`);
  expect(restored.ok()).toBeTruthy();
  const body = await restored.json();
  const inner = JSON.parse(JSON.parse(body.doc_json).source);
  expect(inner.files.f1.dataURL).toBe(img);
  await ctx.dispose();
});
