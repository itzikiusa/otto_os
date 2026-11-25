import { test, expect } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// A streaming answer's DOM (desktop-browser, MOCK mode: mock-runtime.ts
// `stream` plays one message growing, then done). P1-007 / P2-009:
//   • a settled block keeps its DOM node while the rest streams in, and the
//     finished answer keeps it too (no end-of-stream swap of the whole answer);
//   • the diagram is drawn once, on the same element;
//   • each local image is fetched once and gets one Blob URL — also the one in
//     a loose list, which sits in the re-rendered tail until the list ends.
// ─────────────────────────────────────────────────────────────────────────────

const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');

const code = Array.from({ length: 24 }, (_, i) => `export const v${i} = ${i};`).join('\n');
const answer = [
  '## Plan',
  'The first paragraph settles early and must keep its node. '.repeat(6).trim(),
  '![shot](/Users/dev/otto/shot.png)',
  '```ts\n' + code + '\n```',
  '```mermaid\ngraph TD\n  A[UI] --> B[ottod]\n```',
  ...Array.from({ length: 8 }, (_, i) => `Paragraph ${i}: ${'more prose to stream through. '.repeat(5).trim()}`),
  '- one\n\n- ![list](/Users/dev/otto/list.png)\n\n- three',
  'The end.',
].join('\n\n');

test('a streamed answer keeps its settled DOM, draws its diagram once and fetches each image once', async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
    const w = window as unknown as { __blobs: number };
    w.__blobs = 0;
    const make = URL.createObjectURL.bind(URL);
    URL.createObjectURL = (o: Blob | MediaSource) => { w.__blobs++; return make(o); };
  });
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { stream?: unknown } }).__mockRuntime?.stream === 'function');
  // The mock answers /api/v1 in the page; /fs/raw answers a PNG here, counted.
  await page.evaluate((png) => {
    const w = window as unknown as { __fsRaw: number };
    w.__fsRaw = 0;
    const inner = window.fetch;
    const bytes = Uint8Array.from(atob(png), (c) => c.charCodeAt(0));
    window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input instanceof Request ? input.url : input).includes('/fs/raw')) {
        w.__fsRaw++;
        return Promise.resolve(new Response(bytes, { headers: { 'Content-Type': 'image/png' } }));
      }
      return inner(input, init);
    }) as typeof window.fetch;
  }, PNG.toString('base64'));
  await page.evaluate(() => {
    (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => void } }).__mockRuntime.seed({ id: 'rt_md', title: 'Stream render', last_active_at: '2026-11-25T10:00:00Z' });
    location.hash = '#/threads/rt_md';
  });
  const thread = page.getByTestId('runtime-thread-rt_md');
  await expect(thread).toBeVisible({ timeout: 15_000 });

  const play = (text: string, streaming: boolean) =>
    page.evaluate(([t, s]) => (window as unknown as { __mockRuntime: { stream: (...a: unknown[]) => void } }).__mockRuntime.stream('rt_md', 'msg_md', t, s), [text, streaming] as const);
  const md = thread.locator('.rt-assistant .md');
  let marked = false;
  for (let n = 120; n < answer.length; n += 120) {
    await play(answer.slice(0, n), true);
    // One flush apart (the live view batches at 32 ms).
    await page.evaluate(() => new Promise((r) => setTimeout(r, 35)));
    if (!marked && (await md.locator('h2').count()) && (await md.locator('.diagram-block svg').count())) {
      await md.evaluate((el) => {
        (el.querySelector('h2') as unknown as { __mark: number }).__mark = 1;
        (el.querySelector('.diagram-block') as unknown as { __mark: number }).__mark = 1;
      });
      marked = true;
    }
  }
  expect(marked, 'the heading and the drawn diagram settled while streaming').toBe(true);
  await play(answer, false);
  await expect(md.locator('img[data-asset][src^="blob:"]')).toHaveCount(2);

  const kept = await md.evaluate((el) => ({
    heading: (el.querySelector('h2') as unknown as { __mark?: number } | null)?.__mark === 1,
    diagram: (el.querySelector('.diagram-block') as unknown as { __mark?: number } | null)?.__mark === 1,
    svg: !!el.querySelector('.diagram-block svg'),
  }));
  const fsRaw = await page.evaluate(() => (window as unknown as { __fsRaw: number }).__fsRaw);
  const blobs = await page.evaluate(() => (window as unknown as { __blobs: number }).__blobs);
  console.log('STREAM-RENDER', JSON.stringify({ kept, fsRaw, blobs }));
  expect(kept).toEqual({ heading: true, diagram: true, svg: true });
  expect(fsRaw, '/fs/raw fetches').toBe(2);
  expect(blobs, 'Blob URLs made').toBe(2);
});
