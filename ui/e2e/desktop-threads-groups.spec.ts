import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads sidebar and runtime timeline (desktop-browser, MOCK mode —
// ui/src/lib/api/mock-runtime.ts serves the rows and plays live events):
//   1. a personal agent's runs fold into "Agent runs", other modules' helper
//      threads into "Pull requests" / "Connections" — out of the projects and
//      History — and the page opens on the most recent of the person's own;
//   2. a thread that finishes a turn while unfocused gets the unread dot,
//      cleared on focus;
//   3. scrolled up, new messages raise the "N new messages" pill, which
//      jumps back down;
//   4. Edit on a queued message keeps what is being typed.
// ─────────────────────────────────────────────────────────────────────────────

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
  finish: (sid: string) => void;
  queue: (sid: string, text: string) => void;
};

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
  await page.setViewportSize({ width: 1400, height: 900 });
});

const at = (h: number): string => `2026-11-12T${String(h).padStart(2, '0')}:00:00Z`;

/** Seed runtime rows into the mock, then open Threads (it loads them on mount). */
async function openThreads(page: Page, rows: Record<string, unknown>[]): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate((rs) => {
    for (const r of rs) (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.seed(r);
  }, rows);
  await page.evaluate(() => { location.hash = '#/threads'; });
  await expect(page.getByTestId('threads-sidebar')).toBeVisible({ timeout: 15_000 });
}

const hooks = <K extends keyof Hooks>(page: Page, name: K, ...args: Parameters<Hooks[K]>): Promise<void> =>
  page.evaluate(([n, a]) => {
    const h = (window as unknown as { __mockRuntime: Record<string, (...x: unknown[]) => unknown> }).__mockRuntime;
    h[n as string](...(a as unknown[]));
  }, [name, args] as const);

const run = (id: string, h: number, extra: Record<string, unknown> = {}) => ({
  id,
  title: `Agent: Scout ${id}`,
  last_active_at: at(h),
  meta: { source: 'personal_agent', personal_agent: 'pa_scout', personal_agent_name: 'Scout', run_id: `run_${id}`, trigger: 'schedule' },
  ...extra,
});

test('agent runs and helpers fold into their own groups; the page opens on the newest own thread', async ({ page }) => {
  await openThreads(page, [
    { id: 'rt_old', title: 'Older idea', last_active_at: at(7) },
    { id: 'rt_new', title: 'Newest own thread', last_active_at: at(9) },
    { id: 'rt_chat', title: 'Chat: Scout', last_active_at: at(8), meta: { source: 'personal_agent', personal_agent: 'pa_scout', personal_agent_chat: true } },
    run('rt_run1', 10),
    run('rt_run2', 11),
    run('rt_run0', 6, { status: 'closed' }),
    { id: 'rt_pr', title: 'PR #51 agent', last_active_at: at(12), meta: { source: 'pr-agent' } },
    { id: 'rt_diag', title: 'Diagnose prod-db', last_active_at: at(12), meta: { source: 'connection-diagnose' } },
  ]);

  // Not a newer run or helper: the most recent of the person's own threads.
  await expect(page.getByTestId('runtime-thread-rt_new')).toBeVisible({ timeout: 15_000 });

  const active = page.getByTestId('threads-active-list');
  for (const id of ['rt_new', 'rt_old', 'rt_chat']) await expect(active.getByTestId(`thread-row-${id}`)).toBeVisible();
  for (const id of ['rt_run1', 'rt_run2', 'rt_pr', 'rt_diag']) await expect(active.getByTestId(`thread-row-${id}`)).toHaveCount(0);

  const runs = page.getByTestId('threads-group-agent-runs');
  await expect(runs.locator('.og-count')).toHaveText('3');
  await runs.getByRole('button', { name: /Agent runs/ }).click();
  const runRows = runs.locator('[data-testid^="thread-row-"], [data-testid^="thread-history-"]');
  await expect(runRows).toHaveCount(3);
  // Newest first; the closed run sits here too, not in History.
  await expect(runRows.nth(0)).toHaveAttribute('data-testid', 'thread-row-rt_run2');
  await expect(runRows.nth(2)).toHaveAttribute('data-testid', 'thread-history-rt_run0');

  const prs = page.getByTestId('threads-group-helpers-pull-requests');
  await expect(prs.locator('.og-count')).toHaveText('1');
  const conns = page.getByTestId('threads-group-helpers-connections');
  await expect(conns.locator('.og-count')).toHaveText('1');
  await conns.getByRole('button', { name: /Connections/ }).click();
  await expect(conns.getByTestId('thread-row-rt_diag')).toBeVisible();

  await page.getByRole('button', { name: /^History/ }).click();
  const history = page.getByTestId('threads-history-list');
  for (const id of ['rt_run0', 'rt_run1', 'rt_pr', 'rt_diag']) await expect(history.getByTestId(`thread-history-${id}`)).toHaveCount(0);
});

test('a turn that ends off screen leaves an unread dot, cleared on focus', async ({ page }) => {
  await openThreads(page, [
    { id: 'rt_a', title: 'Thread A', last_active_at: at(9) },
    { id: 'rt_b', title: 'Thread B', last_active_at: at(8) },
  ]);
  await expect(page.getByTestId('runtime-thread-rt_a')).toBeVisible({ timeout: 15_000 });

  await hooks(page, 'finish', 'rt_b');
  await hooks(page, 'finish', 'rt_a');
  await expect(page.getByTestId('thread-unread-rt_b')).toBeVisible();
  // The focused thread's own turn is seen.
  await expect(page.getByTestId('thread-unread-rt_a')).toHaveCount(0);

  await page.getByTestId('thread-row-rt_b').locator('.tr-main').click();
  await expect(page.getByTestId('runtime-thread-rt_b')).toBeVisible();
  await expect(page.getByTestId('thread-unread-rt_b')).toHaveCount(0);
});

test('scrolled up, new messages raise a pill that jumps to the latest', async ({ page }) => {
  await openThreads(page, [{ id: 'rt_a', title: 'Long thread', last_active_at: at(9) }]);
  const thread = page.getByTestId('runtime-thread-rt_a');
  await expect(thread).toBeVisible({ timeout: 15_000 });
  await hooks(page, 'say', 'rt_a', Array.from({ length: 40 }, (_, i) => `Answer ${i + 1}\n\nWith a second paragraph to take some room.`));
  await expect(thread.getByText('Answer 40', { exact: true })).toBeVisible();

  const scroller = thread.locator('.rt-scroll');
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(200);
  // Following: the reader is at the bottom, so no pill.
  await expect(page.getByTestId('runtime-new-pill')).toHaveCount(0);

  // The timeline learns the reader left the bottom from the scroll event, which
  // arrives a frame later (and a follow may still be in flight): wait until
  // the scroll has settled at the top before more messages arrive.
  await expect(async () => {
    await scroller.evaluate((el) => new Promise<void>((done) => {
      el.scrollTop = 0;
      requestAnimationFrame(() => requestAnimationFrame(() => done()));
    }));
    expect(await scroller.evaluate((el) => el.scrollTop)).toBe(0);
  }).toPass({ timeout: 5_000 });
  await hooks(page, 'say', 'rt_a', ['Late answer one', 'Late answer two']);
  const pill = page.getByTestId('runtime-new-pill');
  await expect(pill).toHaveText(/2 new messages/);
  await pill.click();
  await expect(pill).toHaveCount(0);
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(40);
  await expect(thread.getByText('Late answer two', { exact: true })).toBeInViewport();
});

test('Edit on a queued message keeps what is being typed', async ({ page }) => {
  await openThreads(page, [{ id: 'rt_a', title: 'Queue thread', last_active_at: at(9) }]);
  const thread = page.getByTestId('runtime-thread-rt_a');
  await expect(thread).toBeVisible({ timeout: 15_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('half a thought');

  await hooks(page, 'queue', 'rt_a', 'queued one');
  const queuedRow = thread.getByTestId('runtime-queued');
  await expect(queuedRow).toContainText('queued one');
  await queuedRow.getByRole('button', { name: 'Edit' }).click();

  await expect(composer).toHaveValue('half a thought\nqueued one');
  await expect(queuedRow).toHaveCount(0);
});
