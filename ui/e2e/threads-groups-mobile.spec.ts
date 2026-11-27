import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow, showThreadList } from './helpers';

// The Threads list on a phone (MOCK mode — mock-runtime.ts): a personal
// agent's runs fold into "Agent runs", a pull request's agent into "Pull
// requests", and a thread opens from a group (pushed over the list on a phone).

test.beforeEach(async ({ page }, ti) => {
  test.skip(!/^(iphone|android-phone)/.test(ti.project.name), 'phones only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
});

test('the sidebar drawer shows the folded groups and opens a thread from one', async ({ page }) => {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { seed?: unknown } }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => string } }).__mockRuntime;
    m.seed({ id: 'rt_mine', title: 'My thread', last_active_at: '2026-11-12T09:00:00Z' });
    m.seed({
      id: 'rt_run1', title: 'Agent: Scout', last_active_at: '2026-11-12T10:00:00Z',
      meta: { source: 'personal_agent', personal_agent: 'pa_scout', personal_agent_name: 'Scout', run_id: 'run_1', trigger: 'schedule' },
    });
    m.seed({ id: 'rt_pr', title: 'PR #51 agent', last_active_at: '2026-11-12T11:00:00Z', meta: { source: 'pr-agent' } });
    location.hash = '#/threads';
  });
  await showThreadList(page);
  const sidebar = page.getByTestId('threads-sidebar');
  await expectFullyInViewport(page, sidebar, 'threads list page');
  await expect(sidebar.getByTestId('threads-group-helpers-pull-requests')).toBeVisible();
  const runs = sidebar.getByTestId('threads-group-agent-runs');
  await expect(runs.locator('.og-count')).toHaveText('1');
  await runs.getByRole('button', { name: /Agent runs/ }).click();
  await expectNoHorizontalOverflow(page);

  await runs.getByTestId('thread-row-rt_run1').locator('.tr-main').click();
  if ((page.viewportSize()!.width <= 640 || page.viewportSize()!.height <= 500)) await expect(sidebar).toBeHidden();
  await expect(page.getByTestId('runtime-thread-rt_run1')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});
