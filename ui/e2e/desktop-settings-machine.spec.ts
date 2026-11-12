import { test, expect, type Page } from '@playwright/test';

// Machine resources (MOCK mode — ui/src/lib/api/mock-machine.ts): the
// status-bar machine chip and Settings ▸ Daemon ▸ Machine resources, both fed
// by `GET /machine/status`. `localStorage.otto_mock_machine` picks what the
// mock reports: calm (default), `busy` (two background agents queued) or
// `low-disk`.

async function boot(page: Page, machine: 'calm' | 'busy' | 'low-disk'): Promise<void> {
  await page.addInitScript((m) => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_mock_machine', m);
  }, machine);
}

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1600, height: 950 });
});

const storedSettings = (page: Page) =>
  page.evaluate(async () => (await (await fetch('/api/v1/settings')).json()) as Record<string, unknown>);

test('Daemon page: Machine resources rows, the live status line, and Save', async ({ page }) => {
  await boot(page, 'calm');
  await page.goto('/#/settings/daemon');
  const card = page.getByTestId('machine-resources');
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole('heading', { name: 'Machine resources' })).toBeVisible();

  // The live line from GET /machine/status.
  await expect(page.getByTestId('machine-status-line')).toHaveText(
    'Now: load 3.1 on 8 cores · memory normal · 2 of 6 background agents · nothing queued · 142 GB free',
  );

  // Every row, with its default shown.
  await expect(card.getByRole('checkbox', { name: 'Queue background agents when this Mac is busy' })).toBeChecked();
  await expect(card.getByRole('checkbox', { name: 'Run background agents at lower priority' })).toBeChecked();
  const maxLive = card.getByLabel('Background agents running at once');
  await expect(maxLive).toHaveValue('');
  await expect(card).toContainText('Blank = automatic (6 on this Mac); 0 = no limit.');
  await expect(card.getByLabel('Build workers per agent session')).toHaveValue('auto');
  await expect(card.getByLabel('Archive finished background agent sessions after (days)')).toHaveValue('7');
  await expect(card).toContainText('Archiving deletes nothing');
  await expect(card.getByLabel('Warn when free disk drops below (GB)')).toHaveValue('');
  // The thresholds sit behind "Advanced".
  await expect(card.getByLabel('Queue while the load per core is above')).toBeHidden();
  await card.getByText('Advanced').click();
  await expect(card.getByLabel('Queue while the load per core is above')).toBeVisible();
  await expect(card.getByLabel('Longest wait for a busy Mac (seconds)')).toBeVisible();
  await expect(card).toContainText('on 8 cores, a 1-minute load above 12.0');

  // A value the daemon would quietly replace is refused before Save.
  const save = page.getByRole('button', { name: 'Save', exact: true });
  await maxLive.fill('-2');
  await expect(card).toContainText('Enter a whole number from 0 (no limit) to 256');
  await expect(save).toBeDisabled();

  // A good value saves only the changed keys.
  await maxLive.fill('3');
  await card.getByLabel('Build workers per agent session').selectOption('off');
  await expect(save).toBeEnabled();
  await save.click();
  await expect(page.locator('.toast').filter({ hasText: 'next background agent' })).toBeVisible();
  const stored = await storedSettings(page);
  expect(stored.agent_background_max_live).toBe(3);
  expect(stored.agent_build_parallelism).toBe('off');
  expect(stored).not.toHaveProperty('agent_admission_max_wait_secs');
  await expect(save).toBeDisabled();
});

test('status bar: the gauge alone while calm', async ({ page }) => {
  await boot(page, 'calm');
  await page.goto('/#/agents');
  const chip = page.getByTestId('statusbar-machine');
  await expect(chip).toBeVisible({ timeout: 15_000 });
  await expect(chip).toHaveAttribute('data-tone', 'calm');
  await expect(chip).toHaveText('');
  await expect(chip).toHaveAttribute('aria-label', /Machine: load 3\.1 on 8 cores/);
  // The bar stays one row.
  const bar = page.locator('footer.statusbar');
  expect((await bar.boundingBox())!.height).toBeLessThanOrEqual(26);
});

test('status bar: busy with a queue, details list each waiting agent', async ({ page }) => {
  await boot(page, 'busy');
  await page.goto('/#/agents');
  const chip = page.getByTestId('statusbar-machine');
  await expect(chip).toHaveAttribute('data-tone', 'busy', { timeout: 15_000 });
  await expect(chip).toHaveText('Machine busy · 2 queued');
  const bar = page.locator('footer.statusbar');
  expect((await bar.boundingBox())!.height).toBeLessThanOrEqual(26);

  await chip.click();
  const dlg = page.getByRole('dialog', { name: 'Machine' });
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText('Busy: memory pressure is high');
  await expect(dlg).toContainText('14.2 on 8 cores');
  await expect(dlg).toContainText('6 of 6 background agents');
  await expect(dlg.getByRole('listitem')).toHaveText([
    'Code review · waiting 1 min · memory pressure is high',
    'Workflow step · waiting 40 s · waiting behind other background agents',
  ]);
  // Root can jump to the settings from here.
  await dlg.getByRole('button', { name: 'Machine resource settings' }).click();
  await expect(page.getByTestId('machine-resources')).toBeVisible();
  await expect(page.getByTestId('machine-status-line')).toContainText('2 queued');
});

test('status bar: low disk turns the chip to a warning with the free space', async ({ page }) => {
  await boot(page, 'low-disk');
  await page.goto('/#/agents');
  const chip = page.getByTestId('statusbar-machine');
  await expect(chip).toHaveAttribute('data-tone', 'low-disk', { timeout: 15_000 });
  await expect(chip).toHaveText('Low disk · 12.0 GB free');
});
