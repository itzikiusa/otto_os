import { test, expect, type Page } from '@playwright/test';

// "Send on Slack as me" (MOCK mode — ui/src/lib/api/mock-slack.ts; nothing
// reaches Slack). The PR ⋯ menu prefills a "finished reviewing" note for the
// author; ⌘K opens the same dialog blank; Settings picks the default Slack
// workspace, globally and for the current Otto workspace (which wins). Send
// is the only thing that posts, and it posts as YOU in the picked workspace.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

const sent = (page: Page) => page.evaluate(() => ((window as unknown as { __mockSlackSent?: unknown[] }).__mockSlackSent ?? []));

test('PR ⋯ → Tell the author on Slack: prefilled, pick a person, Send posts as you', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'More pull request actions' }).click();
  await page.locator('.ctx-menu').getByText('Tell the author on Slack…').click();

  const dlg = page.getByRole('dialog', { name: 'Tell the author on Slack' });
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText('#51');
  await expect(dlg.getByRole('textbox', { name: 'Message' })).toHaveValue(/finished reviewing #51/);
  // Opening sent nothing; Send waits for a recipient and a workspace.
  expect(await sent(page)).toHaveLength(0);
  await expect(dlg.getByRole('button', { name: 'Send' })).toBeDisabled();

  // Two Slack workspaces and no default yet: pick one, then the person.
  await dlg.getByRole('combobox', { name: 'Slack workspace' }).selectOption('T1');
  await dlg.getByRole('textbox', { name: 'To' }).fill('maya');
  await dlg.getByRole('button', { name: /Maya Levi/ }).click();
  await expect(dlg.getByRole('button', { name: 'Change the recipient' })).toBeVisible();
  // The footer says exactly where it goes and as whom, before Send.
  await expect(dlg).toContainText('Posts as you (dana) to Maya Levi in Acme');
  await dlg.getByRole('button', { name: 'Send' }).click();

  await expect(dlg).toHaveCount(0);
  await expect(page.locator('.toast').filter({ hasText: 'Sent to Maya Levi' })).toBeVisible();
  const log = (await sent(page)) as { team: string; channel: string; text: string }[];
  expect(log).toHaveLength(1);
  expect(log[0]).toMatchObject({ team: 'T1', channel: 'U2' });
  expect(log[0].text).toContain('finished reviewing #51');
});

test('⌘K → Send a Slack message as me, to a typed #channel; Cancel sends nothing', async ({ page }) => {
  await page.goto('/#/git');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.keyboard.press('ControlOrMeta+k');
  const bar = page.getByRole('combobox', { name: 'Ask Otto or search commands' });
  await bar.fill('Send a Slack message');
  await page.getByRole('option', { name: /Send a Slack message as me/ }).first().click();

  const dlg = page.getByRole('dialog', { name: 'Send on Slack as me' });
  await expect(dlg).toBeVisible();
  await dlg.getByRole('combobox', { name: 'Slack workspace' }).selectOption('T2');
  await dlg.getByRole('textbox', { name: 'To' }).fill('#pr-reviews');
  await dlg.getByRole('textbox', { name: 'Message' }).fill('review please: PR #51');
  await expect(dlg).toContainText('Posts as you (dana.g) to #pr-reviews in Globex');
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  await expect(dlg).toHaveCount(0);
  expect(await sent(page)).toHaveLength(0);
});

test('Settings: a global default Slack workspace, overridden per Otto workspace, preselects the dialog', async ({ page }) => {
  await page.goto('/#/settings/channels');
  const box = page.getByTestId('slack-as-me-defaults');
  await expect(box).toBeVisible({ timeout: 15_000 });
  await box.getByRole('combobox', { name: 'Default Slack workspace' }).selectOption('T1');
  await expect(box).toContainText('in Acme');
  const wsPick = box.getByRole('combobox', { name: /^In / });
  await wsPick.selectOption('T2');
  await expect(box).toContainText('in Globex');

  // The dialog opens on the workspace's choice.
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Ask Otto or search commands' }).fill('Send a Slack message');
  await page.getByRole('option', { name: /Send a Slack message as me/ }).first().click();
  const dlg = page.getByRole('dialog', { name: 'Send on Slack as me' });
  await expect(dlg.getByRole('combobox', { name: 'Slack workspace' })).toHaveValue('T2');
  await page.keyboard.press('Escape');

  // Clearing the override falls back to the global default.
  await wsPick.selectOption('');
  await expect(box).toContainText('in Acme');
});
