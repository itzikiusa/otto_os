import { test, expect, type WebSocketRoute } from '@playwright/test';
import type { Review } from '../src/lib/api/types';

test('completed async run discovers, expands and reloads summarizer retries and fallback', async ({ page }) => {
  test.setTimeout(90_000);
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
  // A connected event socket (a down one shows every running row as
  // "Reconnecting…"); the spec announces each review change on it the way the
  // daemon's `review_changed` does.
  let events: WebSocketRoute | undefined;
  await page.routeWebSocket(/\/ws\/events/, (socket) => {
    events = socket;
    socket.onMessage(() => {});
  });
  const reviewChanged = () =>
    events?.send(JSON.stringify({ type: 'review_changed', workspace_id: '', review_id: 'r', status: review.status }));
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
  await expect(row).toContainText(/Working \d+s/);
  await row.getByTitle('Show live terminal').click();
  await expect(row.locator('.xterm')).toBeVisible();
  await expect.poll(() => attached).toBe('/ws/term/summary-first');
  await expect(row.locator('.xterm-rows')).toContainText('Summarizing live findings');
  review = { ...review, status: 'done', agents: [{ ...review.agents[0], status: 'done', fallback: true, note: '1 final comment' }] };
  reviewChanged();
  await expect(row.getByTestId('summarizer-fallback')).toContainText('Deterministic fallback');
  await page.reload();
  await expect(page.locator('[data-sess="summary-first"]')).toContainText('fallback');
  review = { ...review, status: 'running', agents: [{ ...review.agents[0], status: 'running', fallback: false, session_id: 'summary-retry' }] };
  // A TERMINAL review is otherwise re-read only every TERMINAL_RECHECK_MS
  // (30 s, RunAgents.svelte); the retry's `review_changed` makes it due now.
  await expect(page.locator('[data-sess="summary-first"], [data-sess="summary-retry"]').first()).toBeVisible();
  reviewChanged();
  await expect(page.locator('[data-sess="summary-retry"]')).toContainText(/Working \d+s/);
  await expect(page.locator('[data-sess="historic"]')).toContainText('Earlier attempt');
  review = { ...review, status: 'cancelled' };
  reviewChanged();
  await expect(page.locator('[data-sess="summary-retry"]')).toContainText('Canceled');
});
