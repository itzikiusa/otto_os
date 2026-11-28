import { test, expect } from '@playwright/test';
import { idePane } from './ide-panes';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';
import { bootWriteIde, CONFIG_FILES, getWriteSeed, writeShot } from './ide-write-fixtures';

// A .properties file on a phone: coloured like on the desktop, the text starts next to the
// line numbers (no empty breakpoint column eating a narrow screen), no page-wide scroll.

test('phone: a .properties file is coloured and starts next to the line numbers', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iPhone portrait');
  test.setTimeout(150_000);
  const s = await getWriteSeed();
  writeFileSync(join(s.root, 'application.properties'), CONFIG_FILES['application.properties']);
  await bootWriteIde(page);
  await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);
  await idePane(page, 'files');
  const file = page.getByTestId('ide-project-tree').locator('button.tree-item[title="application.properties"], button.tree-item[title$="/application.properties"]').first();
  await expect(file).toBeVisible({ timeout: 15_000 });
  await file.tap();
  const line = page.locator('.ide-editor-area .cm-line', { hasText: 'spring.datasource.url=' }).first();
  await expect(line.locator('span[class^="ͼ"]').first()).toBeVisible({ timeout: 15_000 });
  const classes = await line.evaluate((el) => {
    const of = (t: string) => [...el.querySelectorAll('span')].find((s) => s.textContent === t)?.className ?? '';
    return [of('spring.datasource.url'), of('='), of('jdbc:mysql://')];
  });
  for (const c of classes) expect(c).not.toBe('');
  expect(new Set(classes).size, JSON.stringify(classes)).toBe(3);
  const band = await page.locator('.ide-editor-area .cm-editor').first().evaluate((ed) => {
    const nums = ed.querySelector('.cm-lineNumbers')!.getBoundingClientRect().right;
    const r = document.createRange();
    r.selectNodeContents(ed.querySelector('.cm-line')!);
    return Math.round(r.getBoundingClientRect().left - nums);
  });
  expect(band).toBeLessThanOrEqual(30);
  await expectNoHorizontalOverflow(page);
  await writeShot(page, 'phone-properties');
});
