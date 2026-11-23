import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { setup, shoot } from './threadsReviewNotesFixture';

// Review notes on a phone, with real touch: a tapped line number opens the
// note box as a sheet that stays above the on-screen keyboard.

test.use({ serviceWorkers: 'block' });
const MOBILE = /^(iphone-|android-)/;

test('phone: tap a line number → the note box is a sheet above the keyboard', async ({ page }, info) => {
  test.skip(!MOBILE.test(info.project.name), 'phones');
  // On a phone on its side the Thread details card at the head of the panel
  // leaves the diff no height, so no line number can be reached (reported to
  // the details-card owner); the note sheet itself is checked upright.
  test.fixme(info.project.name === 'iphone-landscape', 'the details card leaves the Changes diff no room on a phone on its side');
  await setup(page, { inspector: true });
  await page.goto('/#/threads/notes-alpha');
  await page.getByTestId('threads-toggle-inspector').tap();
  const changesTab = page.getByTestId('threads-tab-changes');
  if (await changesTab.isVisible()) await changesTab.tap();
  const tab = page.getByTestId('inspector-changes-tab');
  const line = tab.getByRole('button', { name: 'Note on src/a.ts new line 9', exact: true });
  await expect(line).toBeVisible({ timeout: 30_000 });
  await line.scrollIntoViewIfNeeded();
  await line.tap();
  const input = page.getByTestId('diff-note-input');
  await expect(input).toBeVisible();
  // The keyboard takes its share of the screen: the box and Save stay in sight.
  const { width: w, height: h } = page.viewportSize()!;
  await page.setViewportSize({ width: w, height: w > h ? h - 180 : Math.round(h * 0.55) });
  await expectFullyInViewport(page, input, 'note box with the keyboard up');
  await input.fill('From the phone.');
  await expectFullyInViewport(page, page.getByTestId('diff-note-save'), 'Save with the keyboard up');
  await shoot(page, `review-notes-${info.project.name}`);
  await page.getByTestId('diff-note-save').tap();
  await expect(input).toHaveCount(0);
  await page.setViewportSize({ width: w, height: h });
  await expect(tab.getByTestId('diff-note')).toContainText('From the phone.');
  await expectNoHorizontalOverflow(page);
});
