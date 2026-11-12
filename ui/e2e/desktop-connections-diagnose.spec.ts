import { test, expect } from '@playwright/test';
import { openPage } from './helpers';
import { seedDiagnose } from './connections-diagnose-mocks';

// The routes in connections-diagnose-mocks.ts must see every daemon fetch.
test.use({ serviceWorkers: 'block' });

// "Diagnose with an agent": an ended SSH tab's failed Test → Diagnose → the
// consent dialog → an approval-required thread embedded beside the terminal,
// without leaving the page.
test('a failed SSH connection opens an embedded, approval-required diagnosis thread after consent', async ({ page }) => {
  const f = await seedDiagnose(page);
  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: f.name }).click();

  const panel = page.getByTestId('ssh-exit-panel');
  await expect(panel.getByTestId('ssh-exit-message')).toContainText('Could not resolve', { timeout: 20_000 });
  const before = page.url();
  await panel.getByTestId('conn-diagnose').click();

  // Consent first: what runs, where, and what it will not do.
  const consent = page.getByTestId('diagnose-consent');
  await expect(consent).toContainText('ON THIS MAC');
  await expect(consent).toContainText(`${f.name} (ssh, fixture.invalid)`);
  await expect(consent).toContainText('will not log in to the remote host');
  await consent.getByText('Show the checks').click();
  await expect(consent).toContainText('nc -vz -w 5 fixture.invalid 2222');
  expect(f.created).toHaveLength(0);

  await page.getByTestId('diagnose-start').click();
  const aside = page.getByTestId('diagnose-aside');
  await expect(aside).toContainText(`Diagnosing ${f.name}`);
  await expect(aside.locator('[data-testid="runtime-thread-rt-diag-1"]')).toBeVisible({ timeout: 15_000 });
  expect(page.url()).toBe(before);

  await expect.poll(() => f.prompts.length).toBe(1);
  const create = f.created[0];
  expect(create.access_mode).toBe('approval-required');
  expect(create.source).toBe('connection-diagnose');
  expect(create.origin).toEqual({ kind: 'connection', id: f.connId, label: f.name });
  const text = String(f.prompts[0].text);
  expect(text).toContain('fixture.invalid');
  expect(text).toContain('Could not resolve hostname');
  expect(f.prompts[0].access_mode).toBe('approval-required');

  // Close keeps the thread: Diagnose again shows it with no new consent or thread.
  await aside.getByRole('button', { name: 'Close — the thread is kept' }).click();
  await expect(aside).toHaveCount(0);
  await panel.getByTestId('conn-diagnose').click();
  await expect(page.getByTestId('diagnose-aside')).toBeVisible();
  await expect(page.getByTestId('diagnose-consent')).toHaveCount(0);
  expect(f.created).toHaveLength(1);
  await f.dispose();
});
