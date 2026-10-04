import {test, expect} from '@playwright/test';

test.use({serviceWorkers: 'block'});

test('History scrolling to the top loads each earlier cursor after initial layout settles', async ({page}) => {
  const stamp = new Date().toISOString();
  const row = {session_id: null, provider: 'claude', title: 'Scroll pagination regression', first_prompt: 'Review', cwd: '/tmp/history-scroll', repo_name: 'Synthetic', started_at: stamp, last_active_at: stamp, turns: 180, status: 'on_disk', transcript_path: '/tmp/history-scroll.jsonl', resumable: true};
  const requested: (string | null)[] = [];
  await page.route('**/api/v1/workspaces/*/history/page?*', r => r.fulfill({json: {entries: [row], next_cursor: null}}));
  await page.route('**/api/v1/workspaces/*/history/transcript?*', r => {
    const before = new URL(r.request().url()).searchParams.get('before');
    requested.push(before);
    const end = before === null ? 180 : Number(before);
    const start = Math.max(0, end - 60);
    return r.fulfill({json: {
      session_id: null, provider: 'claude', title: row.title, cwd: row.cwd, model: null,
      cursor: String(start), has_earlier: start > 0,
      turns: Array.from({length: end - start}, (_, i) => ({id: `scroll-${i + start}`, role: (i + start) % 2 ? 'assistant' : 'user', ts: stamp, blocks: [{kind: 'text', md: `Scroll checkpoint ${i + start}. ${'Review the recovery steps. '.repeat(8)}`}], duration_ms: null, model: null, system: [], reasoning_steps: 0})),
      stats: {turns: 180, tool_calls: 0, cost_usd: null, input_tokens: null, output_tokens: null, duration_ms: null, reasoning_steps: 0, thinking_steps: 0, unknown_records: 0}, subagents: [], unavailable_reason: null,
    }});
  });
  await page.goto('/#/history');
  await page.getByTestId('history-row').click();
  const conversation = page.getByTestId('history-conversation');
  await expect(conversation).toContainText('Scroll checkpoint 179.');
  // Let the explicit 1-second initial layout guard expire; this scenario is
  // actual user scrolling, separate from keyboard button activation.
  await page.waitForTimeout(1200);
  // Boot can restore the workspace after History auto-selects a scratch row.
  // Count those initial reads separately; scrolling must issue only its exact
  // earlier cursors, with no duplicate pages or reset to the newest page.
  expect(requested.length).toBeGreaterThan(0);
  expect(requested.every(before => before === null)).toBe(true);
  const startupRequests = requested.length;
  const scroller = conversation.locator('.conv-list');
  await scroller.hover();
  await page.mouse.wheel(0, -100_000);
  await expect.poll(() => requested.slice(startupRequests)).toEqual(['120']);
  await expect(conversation).toContainText('Scroll checkpoint 60.');
  await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBeGreaterThan(40);
  await page.mouse.wheel(0, -100_000);
  await expect.poll(() => requested.slice(startupRequests)).toEqual(['120', '60']);
  await expect(conversation).toContainText('Scroll checkpoint 0.');
  await expect(conversation.getByRole('button', {name: 'Load earlier messages', exact: true})).toHaveCount(0);
});
