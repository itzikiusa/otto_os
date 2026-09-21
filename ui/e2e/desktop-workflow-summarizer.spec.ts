import { test, expect } from '@playwright/test';
import type { Review } from '../src/lib/api/types';

test('completed async run discovers, expands and reloads summarizer retries and fallback', async ({ page }) => {
  let review: Review = { id: 'r', repo_id: 'repo', pr_number: 0, status: 'running', error: null,
    comments: [], created_at: new Date().toISOString(), agents: [
      { name: 'Summarizer', provider: 'codex', model: 'configured', status: 'running',
        note: '', comment_count: 0, session_id: 'summary-first' },
    ] };
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin);
    localStorage.setItem('otto_token', 'isolated-fixture');
  });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/v1/reviews/r') return route.fulfill({ json: review });
    if (path.startsWith('/api/v1/sessions/')) {
      const id = path.split('/').at(-1);
      return route.fulfill({ json: { id, kind: 'agent', provider: 'codex', title: id === 'historic' ? 'Earlier attempt' : 'Review summarizer',
        status: 'reconnectable', created_at: new Date(Date.now() - 15_000).toISOString(), meta: {} } });
    }
    await route.fulfill({ json: [] });
  });
  let attached = '';
  await page.routeWebSocket(/\/ws\/term\//, (socket) => {
    attached = new URL(socket.url()).pathname;
    socket.onMessage((message) => {
      if (String(message).includes('scrollback')) socket.send(JSON.stringify({ type: 'scrollback',
        data: Buffer.from('Summarizing live findings\r\n').toString('base64') }));
    });
  });
  await page.goto('/e2e/fixtures/workflow-summarizer.html');
  const row = page.locator('[data-sess="summary-first"]');
  await expect(row).toContainText('Summarizer');
  await expect(row).toContainText('codex');
  await expect(row).toContainText(/running \d+s/);
  await row.getByTitle('Show live terminal').click();
  await expect(row.locator('.xterm')).toBeVisible();
  await expect.poll(() => attached).toBe('/ws/term/summary-first');
  await expect(row.locator('.xterm-rows')).toContainText('Summarizing live findings');
  review = { ...review, status: 'done', agents: [{ ...review.agents[0], status: 'done', fallback: true, note: '1 final comment' }] };
  await expect(row.getByTestId('summarizer-fallback')).toContainText('Deterministic fallback');
  await page.reload();
  await expect(page.locator('[data-sess="summary-first"]')).toContainText('fallback');
  review = { ...review, status: 'running', agents: [{ ...review.agents[0], status: 'running', fallback: false, session_id: 'summary-retry' }] };
  await expect(page.locator('[data-sess="summary-retry"]')).toContainText(/running \d+s/);
  await expect(page.locator('[data-sess="historic"]')).toContainText('Earlier attempt');
  review = { ...review, status: 'cancelled' };
  await expect(page.locator('[data-sess="summary-retry"]')).toContainText('cancelled');
});
