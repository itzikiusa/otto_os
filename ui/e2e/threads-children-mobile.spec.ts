import { test, expect, type Page } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Threads inside a thread on a phone (MOCK mode — mock-runtime.ts): the
// "Ask another agent" button is reachable in the composer, a child's finished
// turn shows in its parent as one line with Open, Open shows the child in
// place with a "‹ parent" chip that leads back, and nothing scrolls sideways.

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
  childDone: (sid: string, line: string) => void;
};

test.beforeEach(async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
});

async function seed(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    m.seed({ id: 'rt_ph_lead', title: 'Plan the release', provider: 'claude', last_active_at: '2026-11-12T10:09:00Z' });
    m.seed({ id: 'rt_ph_kid', title: 'Check the changelog', provider: 'codex', parent_id: 'rt_ph_lead', last_active_at: '2026-11-12T10:08:00Z', meta: { child_of: { id: 'rt_ph_lead', by: 'person' } } });
    m.say('rt_ph_lead', ['Here is the release plan.']);
    m.say('rt_ph_kid', ['The changelog misses two fixes.']);
    location.hash = '#/threads/rt_ph_lead';
  });
}

test('a child thread on a phone: its result line in the parent, Open, and back', async ({ page }) => {
  await seed(page);
  const lead = page.getByTestId('runtime-thread-rt_ph_lead');
  await expect(lead).toBeVisible({ timeout: 15_000 });
  await expectFullyInViewport(page, lead.getByTestId('runtime-ask-agent'));

  await page.evaluate(() => (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.childDone('rt_ph_kid', 'The changelog misses two fixes.'));
  const notice = lead.getByTestId('runtime-child-notice-rt_ph_kid');
  await expect(notice).toContainText('finished: The changelog misses two fixes.');
  await expectNoHorizontalOverflow(page);

  await notice.getByRole('button', { name: 'Open' }).click();
  await expect(page.getByTestId('runtime-thread-rt_ph_kid')).toBeVisible();
  const back = page.getByTestId('threads-back-to-parent');
  await expect(back).toContainText('Plan the release');
  await expectFullyInViewport(page, back);
  await expectNoHorizontalOverflow(page);

  await back.click();
  await expect(page.getByTestId('runtime-thread-rt_ph_lead')).toBeVisible();
});

test('Ask another agent opens its agent menu on a phone', async ({ page }) => {
  await seed(page);
  const lead = page.getByTestId('runtime-thread-rt_ph_lead');
  await expect(lead).toBeVisible({ timeout: 15_000 });
  await lead.getByTestId('runtime-ask-agent').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await expectFullyInViewport(page, picker);
  await picker.getByTestId('model-picker-provider-codex').click();
  await picker.getByRole('button', { name: 'Default model' }).click();
  await expect(lead.getByTestId('runtime-ask-targets')).toContainText('Codex');
  await expectNoHorizontalOverflow(page);
});
