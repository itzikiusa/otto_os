import { test, expect, type Page } from '@playwright/test';
import { bootWriteIde, COMMON_MK, MAKEFILE, openTreeFile, writeShot } from './ide-write-fixtures';

// Makefiles in the IDE (owner, 2026-11-24, with a screenshot of his go_acceptance_tests
// Makefile in white text): a real language (definitions, references, targets, comments,
// recipes as shell), text next to the line numbers (no empty debug/fold/lint gutters),
// a ▶ per target that runs `make <target>`, the Structure tab, ⌘-click / ⌘B to a
// definition (also in an included .mk), and a hover that shows the definition.

const editor = (page: Page) => page.locator('.ide-editor-area .cm-editor').first();
const line = (page: Page, text: string | RegExp) => editor(page).locator('.cm-line', { hasText: text }).first();

/** The highlight class of the token `text` on the first line holding `lineText`. */
async function tokenClass(page: Page, lineText: string | RegExp, text: string): Promise<string> {
  const l = line(page, lineText);
  await l.scrollIntoViewIfNeeded();
  return l.evaluate((el, t) => {
    const span = [...el.querySelectorAll('span')].find((s) => s.textContent === t && s.children.length === 0);
    return span?.className ?? '';
  }, text);
}

/** Px from the line numbers' right edge to the first character of the text. */
async function textBand(page: Page): Promise<number> {
  return editor(page).evaluate((ed) => {
    const nums = ed.querySelector('.cm-lineNumbers')!.getBoundingClientRect().right;
    const r = document.createRange();
    r.selectNodeContents(ed.querySelector('.cm-line')!);
    return Math.round(r.getBoundingClientRect().left - nums);
  });
}

async function cmdClick(page: Page, lineText: string, word: string): Promise<void> {
  const l = line(page, lineText);
  await l.scrollIntoViewIfNeeded();
  const box = await l.evaluate((el, w) => {
    const r = document.createRange();
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const i = n.textContent!.indexOf(w);
      if (i >= 0) {
        r.setStart(n, i + 1);
        r.setEnd(n, i + 2);
        const b = r.getBoundingClientRect();
        return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
      }
    }
    return null;
  }, word);
  expect(box, `${word} on "${lineText}"`).not.toBeNull();
  await page.mouse.move(box!.x, box!.y);
  await page.keyboard.down('Meta');
  await page.mouse.click(box!.x, box!.y);
  await page.keyboard.up('Meta');
}

/** The cursor sits on the first line of `text` that starts with `prefix`. */
async function expectCursorOn(page: Page, prefix: string, text = MAKEFILE): Promise<void> {
  const n = text.split('\n').findIndex((l) => l.startsWith(prefix)) + 1;
  expect(n, prefix).toBeGreaterThan(0);
  await expect(page.getByTestId('ide-status-cursor')).toHaveText(new RegExp(`^Ln ${n},`), { timeout: 10_000 });
}

for (const scheme of ['dark', 'light'] as const) {
  test(`a Makefile reads like code: distinct tokens, text next to the numbers (${scheme})`, async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await bootWriteIde(page, scheme);
    await openTreeFile(page, 'Makefile');
    await expect(page.getByTestId('ide-status-strip').or(page.locator('.ide-editor-area')).first()).toBeVisible();

    // The language pack loads with the file type: wait for its first token.
    await expect(line(page, /^APP_NAME\s+:=/).locator('span').first()).toBeVisible({ timeout: 15_000 });
    const classes = {
      definition: await tokenClass(page, /^APP_NAME\s+:=/, 'APP_NAME'),
      // `$(` and `)` (the brackets also carry the rainbow-bracket class)
      reference: await tokenClass(page, 'go build $(GO_BUILD_FLAGS)', '$'),
      target: await tokenClass(page, 'all: lint test build', 'all'),
      comment: await tokenClass(page, '# go_acceptance_tests', '# go_acceptance_tests — build, lint and run the acceptance suite.'),
      recipe: await tokenClass(page, 'go build $(GO_BUILD_FLAGS)', 'go'),
      directive: await tokenClass(page, 'include common.mk', 'include'),
    };
    for (const [k, c] of Object.entries(classes)) expect(c, k).not.toBe('');
    expect(new Set(Object.values(classes)).size, JSON.stringify(classes)).toBe(Object.keys(classes).length);
    // The function name inside $(shell …) is apart from a variable name.
    expect(await tokenClass(page, /^GIT_COMMIT\s+:=/, 'shell')).not.toBe(await tokenClass(page, /^GO_BUILD_FLAGS\s+=/, 'GIT_COMMIT'));
    // …and a reference's name is coloured (not plain text).
    expect(await tokenClass(page, /^GO_BUILD_FLAGS\s+=/, 'GIT_COMMIT')).not.toBe('');

    // No empty debug / fold / lint gutters: the text starts next to the line numbers…
    await editor(page).locator('.cm-line').first().scrollIntoViewIfNeeded();
    const mkBand = await textBand(page);
    expect(mkBand).toBeLessThanOrEqual(34);
    await expect(editor(page).locator('.cm-foldGutter')).toHaveCount(0);
    await expect(editor(page).locator('.cm-gutter-lint')).toHaveCount(0);
    await writeShot(page, `makefile-${scheme}`);
    // …while a Go file keeps the gutters it uses (run ▶, folds, breakpoints).
    await openTreeFile(page, 'cmd/main.go');
    expect(await textBand(page)).toBeGreaterThan(mkBand + 30);
    await expect(editor(page).locator('.cm-foldGutter')).toHaveCount(1);
    await writeShot(page, `go-${scheme}`);
  });
}

test('Makefile: Structure lists targets then variables; ⌘-click, ⌘B and hover find definitions (also in an included .mk)', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  // A Makefile's outline and ▶ targets come from the buffer: no symbols / gutter request.
  const analysis: string[] = [];
  page.on('request', (r) => {
    if (/\/ide\/(?:lsp\/symbols|gutter)\b/.test(r.url()) && /Makefile|\.mk/.test(r.postData() ?? '')) analysis.push(r.url());
  });
  await bootWriteIde(page);
  await openTreeFile(page, 'Makefile');

  // Structure: targets, then variables; a click jumps.
  await page.getByTestId('ide-tree-tab-structure').click();
  const rows = page.locator('.struct-row');
  await expect(rows.filter({ hasText: 'docker-build' })).toHaveCount(1, { timeout: 15_000 });
  const texts = (await rows.allTextContents()).map((t) => t.replace(/\s+/g, ' ').trim());
  const firstVar = texts.findIndex((t) => t.startsWith('var '));
  expect(firstVar).toBeGreaterThan(0);
  expect(texts.slice(0, firstVar).every((t) => t.startsWith('target '))).toBe(true);
  expect(texts.slice(firstVar).every((t) => t.startsWith('var '))).toBe(true);
  expect(texts.some((t) => t.startsWith('var GIT_COMMIT'))).toBe(true);
  await rows.filter({ hasText: /^\s*target\s*deploy/ }).first().click();
  await expectCursorOn(page, 'deploy: docker-build');

  // ⌘-click a reference → its assignment.
  await cmdClick(page, 'go build $(GO_BUILD_FLAGS)', 'GO_BUILD_FLAGS');
  await expectCursorOn(page, 'GO_BUILD_FLAGS  =');
  // ⌘-click a prerequisite → its rule.
  await cmdClick(page, 'deploy: docker-build', 'docker-build');
  await expectCursorOn(page, 'docker-build: build');
  // ⌘B (Go to definition) on a $(call …) name → the define.
  const callLine = line(page, '$(call banner');
  await callLine.scrollIntoViewIfNeeded();
  await cmdClickless(page, '$(call banner', 'banner');
  await page.keyboard.press('Meta+b');
  await expectCursorOn(page, 'define banner');

  // Hover shows the definition line.
  const ref = line(page, '@echo "Building $(APP_NAME)');
  await ref.scrollIntoViewIfNeeded();
  await hoverWord(page, '@echo "Building $(APP_NAME)', 'APP_NAME');
  const tip = page.getByTestId('makefile-def-hover');
  await expect(tip).toBeVisible({ timeout: 10_000 });
  await expect(tip).toContainText('APP_NAME       := go_acceptance_tests');
  await expect(tip).toContainText('line 4');
  await writeShot(page, 'makefile-hover');
  await page.mouse.move(5, 5);

  // A variable defined only in the included common.mk opens that file at its line.
  await cmdClick(page, 'golangci-lint run', 'LINT_VERSION');
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText('common.mk', { timeout: 15_000 });
  await expectCursorOn(page, 'LINT_VERSION', COMMON_MK);
  expect(analysis, 'no /ide/lsp/symbols or /ide/gutter request for a Makefile').toEqual([]);
});

test('Makefile: a ▶ per target runs make <target> (no Debug, no breakpoints)', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await bootWriteIde(page);
  await openTreeFile(page, 'Makefile');
  const buildLine = await line(page, 'build: ## Build the binary').evaluate((el) => {
    const view = (el.closest('.cm-editor') as HTMLElement);
    return [...view.querySelectorAll('.cm-line')].indexOf(el);
  });
  expect(buildLine).toBeGreaterThan(0);
  const run = editor(page).locator('[data-gutter-run][aria-label="Run make build"]');
  await expect(run).toBeVisible({ timeout: 15_000 });
  await expect(editor(page).locator('[data-gutter-run][aria-label="Run make %.o"]')).toHaveCount(0);
  // Clicking a line's gutter sets no breakpoint in a Makefile.
  const bpGutter = editor(page).locator('.cm-ide-bp-gutter');
  const lineBox = (await line(page, /^APP_NAME\s+:=/).boundingBox())!;
  const gBox = (await bpGutter.boundingBox())!;
  await page.mouse.click(gBox.x + gBox.width / 2, lineBox.y + lineBox.height / 2);
  await expect(editor(page).locator('[data-testid="ide-bp-marker"]')).toHaveCount(0);

  await run.click();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByText("Run 'make build'")).toBeVisible();
  await expect(menu.getByText(/^Debug/)).toHaveCount(0);
  await menu.getByText("Run 'make build'").click();
  await expect(page.getByTestId('ide-run-panel')).toContainText('Building go_acceptance_tests', { timeout: 30_000 });
  await writeShot(page, 'makefile-run');
});

async function wordBox(page: Page, lineText: string, word: string): Promise<{ x: number; y: number }> {
  const box = await line(page, lineText).evaluate((el, w) => {
    const r = document.createRange();
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const i = n.textContent!.indexOf(w);
      if (i >= 0) {
        r.setStart(n, i + 1);
        r.setEnd(n, i + 2);
        const b = r.getBoundingClientRect();
        return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
      }
    }
    return null;
  }, word);
  expect(box).not.toBeNull();
  return box!;
}

/** Put the cursor inside `word` (a plain click). */
async function cmdClickless(page: Page, lineText: string, word: string): Promise<void> {
  const b = await wordBox(page, lineText, word);
  await page.mouse.click(b.x, b.y);
}

async function hoverWord(page: Page, lineText: string, word: string): Promise<void> {
  const b = await wordBox(page, lineText, word);
  await page.mouse.move(b.x, b.y);
}
