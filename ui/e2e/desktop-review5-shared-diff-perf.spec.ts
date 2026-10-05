import {test, expect} from '@playwright/test';
import {heapAfterGC, watchLongTasks, longTasks} from './perf';

test.use({serviceWorkers: 'block'});
test('shared diff measures equal sparse and rewritten large inputs with bounded mounted rows', async ({page, browserName}, info) => {
  test.skip(browserName !== 'chromium', 'CDP allocation measurements');
  test.setTimeout(90_000);
  await page.goto('/e2e/fixtures/review-diff.html');
  await page.waitForFunction(() => 'reviewDiff' in window);
  const apply = (before: string, after: string) => page.evaluate(async ({before, after}) => {
    const host = window as unknown as {reviewDiff: (a: string, b: string) => Promise<number>};
    return host.reviewDiff(before, after);
  }, {before, after});
  await apply('warm before', 'warm after'); await apply('', '');
  const baseline = await heapAfterGC(page), samples: unknown[] = [];
  await watchLongTasks(page);
  const equal = Array.from({length: 50_000}, (_, i) => `line ${i}: preserved source bytes`).join('\n');
  const sparse = equal.replace('line 10:', 'changed 10:').replace('line 49980:', 'changed 49980:');
  const before = Array.from({length: 10_000}, (_, i) => `before ${i}`).join('\n');
  const after = Array.from({length: 10_000}, (_, i) => `rewritten ${i}`).join('\n');
  for (const [kind, left, right] of [['equal-50k', equal, equal], ['sparse-50k', equal, sparse], ['rewrite-10k', before, after]]) {
    const paintMs = await apply(left, right);
    const rows = await page.locator('.dv-srow').count();
    expect(rows).toBeLessThanOrEqual(500);
    samples.push({kind, paintMs, rows, heapBytes: await heapAfterGC(page)});
    if (kind === 'rewrite-10k') {
      for (let i = 1; i < 20; i++) {
        await page.getByRole('button', {name: 'Next changes', exact: true}).click();
        await expect(page.locator('.dv-srow')).toHaveCount(500);
      }
      await expect(page.locator('.dv')).toContainText('rewritten 9999');
      await expect(page.getByRole('button', {name: 'Next changes', exact: true})).toBeDisabled();
      for (const [side, expected] of [['before', before], ['after', after]]) {
        const downloaded = page.waitForEvent('download');
        await page.getByRole('button', {name: `Download ${side}`, exact: true}).click();
        const stream = await (await downloaded).createReadStream();
        const chunks: Buffer[] = [];
        for await (const chunk of stream) chunks.push(Buffer.from(chunk));
        expect(Buffer.concat(chunks).toString()).toBe(expected);
      }
    }
    await apply('', '');
    samples.push({kind: `${kind}-released`, heapBytes: await heapAfterGC(page)});
  }
  await info.attach('shared-diff-measurements', {body: JSON.stringify({scope: 'Actual shared DiffView in an isolated mounted fixture; client rendering and JS heap only. Paint duration includes two animation frames. Timings are observations; row and full-download correctness are assertions.', baselineHeapBytes: baseline, samples, longTaskMs: await longTasks(page)}, null, 2), contentType: 'application/json'});
});
