import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

let workspaceId = '';
let skillName = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  skillName = `access-audit-${Date.now()}`;
  const result = await ctx.post(`${base}/api/v1/library/skills`, { data: {
    name: skillName, category: 'review', description: 'Shared Markdown accessibility fixture',
    body: '---\ndescription: Shared Markdown accessibility fixture\n---\n\n## מדריך בדיקה\n\nהריצו `./q --help` ואז המשיכו.\n\n```sh\n./q --help\nprintf "%s\\n" "hello"\n```\n\n- בדיקה ראשונה\n- בדיקה שנייה\n\n1. פתחו מסוף\n2. העתיקו את הפקודה\n\n> ציטוט: יש לשמור על סדר הפקודה.\n',
  }});
  expect(result.ok(), await result.text()).toBeTruthy();
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspaceId);
});

const variants = [
  { name: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900, direction: 'ltr' },
  { name: 'native-dark', theme: 'native', scheme: 'dark', width: 1440, height: 900, direction: 'ltr' },
  { name: 'warm-light-phone', theme: 'warm', scheme: 'light', width: 375, height: 812, direction: 'rtl' },
  { name: 'warm-dark', theme: 'warm', scheme: 'dark', width: 1440, height: 900, direction: 'ltr' },
  { name: 'pro-dark-tablet', theme: 'pro-dark', scheme: 'dark', width: 1024, height: 768, direction: 'rtl' },
];
for (const variant of variants) {
  test(`Markdown preserves command order and logical spacing: ${variant.name}`, async ({ page }, info) => {
    await page.setViewportSize({ width: variant.width, height: variant.height });
    await page.addInitScript(v => {
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.direction);
    }, variant);
    await page.goto('/#/skills-eval');
    await page.getByRole('searchbox', { name: 'Search skills' }).fill(skillName);
    await page.getByTestId('skill-row').click();
    const preview = page.getByTestId('skill-preview');
    await expect(preview.locator('pre')).toContainText('./q --help');
    await preview.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath('markdown.png') });
    for (const selector of ['pre', 'code']) {
      for (const element of await preview.locator(selector).all()) {
        await expect(element).toHaveCSS('direction', 'ltr');
        await expect(element).toHaveCSS('unicode-bidi', 'isolate');
      }
    }
    if (variant.direction === 'rtl') {
      await expect(preview.locator('ul')).toHaveCSS('padding-right', '22px');
      await expect(preview.locator('ul')).toHaveCSS('padding-left', '0px');
      await expect(preview.locator('blockquote')).toHaveCSS('border-right-width', '3px');
      await expect(preview.locator('blockquote')).toHaveCSS('border-left-width', '0px');
    }
    await expectNoHorizontalOverflow(page);
  });
}

async function openSession(page: Page) {
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible();
  await page.keyboard.press('Meta+t');
  const dialog = page.getByRole('dialog', { name: 'New session', exact: true });
  await expect(dialog).toBeVisible();
  return dialog;
}

test('nested folder sheet owns Tab and Escape, then restores the Browse trigger', async ({ page }, info) => {
  const session = await openSession(page);
  const browse = session.getByRole('button', { name: 'Browse…' }).first();
  await browse.focus();
  await browse.press('Enter');
  const picker = page.getByRole('dialog', { name: 'Choose working directory' });
  await expect(picker.getByRole('button', { name: 'Use this folder' })).toBeVisible();
  await picker.getByRole('button', { name: 'Use this folder' }).focus();
  await page.keyboard.press('Tab');
  await expect(picker.getByRole('button', { name: 'Close', exact: true })).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(picker.getByRole('button', { name: 'Use this folder' })).toBeFocused();
  await expectFullyInViewport(page, picker);
  await page.screenshot({ path: info.outputPath('nested-folder.png') });
  await page.keyboard.press('Escape');
  await expect(picker).toBeHidden();
  await expect(session).toBeVisible();
  await expect(browse).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(session).toBeHidden();
});

test('phone Navigator keeps its focus after the workspace sheet closes', async ({ page }, info) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/#/home');
  const trigger = page.getByRole('button', { name: 'Open navigator', exact: true });
  await trigger.focus();
  await trigger.press('Enter');
  const drawer = page.getByRole('dialog', { name: 'Navigator', exact: true });
  const add = drawer.getByRole('button', { name: 'Add workspace', exact: true });
  await add.focus();
  await add.press('Enter');
  const sheet = page.getByRole('dialog', { name: 'Add Workspace', exact: true });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('button', { name: 'Close', exact: true }).focus();
  await page.keyboard.press('Shift+Tab');
  await expect.poll(() => sheet.evaluate(el => el.contains(document.activeElement))).toBe(true);
  await page.screenshot({ path: info.outputPath('drawer-sheet.png') });
  await page.keyboard.press('Escape');
  await expect(sheet).toBeHidden();
  await expect(drawer).toBeVisible();
  await expect(add).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await expect(trigger).toBeFocused();
});

test('phone confirmation keeps every long choice inside the sheet', async ({ page }, info) => {
  await page.setViewportSize({ width: 375, height: 667 });
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible();
  // Exercise the real shared choice surface with the labels used by bulk tab
  // close, without starting or ending actual agent sessions.
  await page.evaluate(async () => {
    const modulePath = '/src/lib/confirm.svelte.ts';
    const { confirmer } = await import(modulePath);
    void confirmer.choose('Closing these tabs ends 123 sessions. '.repeat(20), {
      title: 'Close 123 sessions?',
      options: [
        { label: 'Archive 123 sessions', value: 'archive', kind: 'primary' },
        { label: 'Delete 123 sessions', value: 'delete', kind: 'danger' },
      ],
    });
  });
  const sheet = page.getByRole('dialog', { name: 'Close 123 sessions?' });
  await expect(sheet).toBeVisible();
  for (const button of await sheet.locator('footer button').all()) {
    await expectFullyInViewport(page, button);
    const inside = await button.evaluate(el => {
      const box = el.getBoundingClientRect();
      const parent = el.closest('.sheet')!.getBoundingClientRect();
      return box.left >= parent.left && box.right <= parent.right;
    });
    expect(inside, 'action remains inside its sheet').toBe(true);
  }
  await page.screenshot({ path: info.outputPath('long-confirm.png') });
  const cancel = sheet.getByRole('button', { name: 'Cancel', exact: true });
  if (info.project.use.hasTouch) await cancel.tap();
  else await cancel.click();
  await expect(sheet).toBeHidden();
});
