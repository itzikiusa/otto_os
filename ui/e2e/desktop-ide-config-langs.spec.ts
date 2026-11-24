import { test, expect, type Page } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { bootWriteIde, CONFIG_FILES, CONFIG_PLACEHOLDER as PLACEHOLDER, getWriteSeed, openTreeFile, writeShot } from './ide-write-fixtures';

// Config files in the IDE (owner, 2026-11-24, with a screenshot of a Java .properties file in
// white text and an empty band beside the line numbers: "can we add support for .properties?
// … .yaml .yml … .toml … or any weird lang like this"). Each opens with its own language,
// tokens of different kinds get different colours, the text starts next to the line
// numbers (no breakpoint or lint column), and Structure lists the keys.

const editor = (page: Page) => page.locator('.ide-editor-area .cm-editor').first();
const line = (page: Page, text: string | RegExp) => editor(page).locator('.cm-line', { hasText: text }).first();

/** The highlight class of the token "text" on the first line holding lineText. */
async function tokenClass(page: Page, lineText: string | RegExp, text: string): Promise<string> {
  const l = line(page, lineText);
  await l.scrollIntoViewIfNeeded();
  return l.evaluate((el, t) => {
    // The token's own span (rainbow brackets may nest spans inside it).
    const span = [...el.querySelectorAll('span')].find((s) => s.textContent === t && [...s.classList].some((c) => c.startsWith('ͼ')));
    return [...(span?.classList ?? [])].filter((c) => c.startsWith('ͼ')).join(' ');
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

async function seedConfigFiles(): Promise<void> {
  const s = await getWriteSeed();
  for (const [name, text] of Object.entries(CONFIG_FILES)) writeFileSync(join(s.root, name), text);
}

async function expectDistinct(page: Page, probe: string | RegExp, tokens: [string | RegExp, string][]): Promise<void> {
  // The language pack loads with the file type: wait for its first token class (the rainbow
  // bracket layer adds plain spans before that).
  await expect(line(page, probe).locator('span[class^="ͼ"]').first()).toBeVisible({ timeout: 15_000 });
  const classes: Record<string, string> = {};
  for (const [lineText, tok] of tokens) classes[tok] = await tokenClass(page, lineText, tok);
  for (const [k, c] of Object.entries(classes)) expect(c, k + ' in ' + (await line(page, tokens.find(([, t]) => t === k)![0]).innerHTML())).not.toBe('');
  expect(new Set(Object.values(classes)).size, JSON.stringify(classes)).toBe(tokens.length);
}

async function expectNoUnusedGutters(page: Page): Promise<void> {
  await editor(page).locator('.cm-line').first().scrollIntoViewIfNeeded();
  // The IDE gutters mount after the first paint: wait until the band settles.
  await expect.poll(() => textBand(page), { timeout: 10_000 }).toBeLessThanOrEqual(30);
  await expect(editor(page).locator('.cm-gutter-lint')).toHaveCount(0);
  const bp = editor(page).locator('.cm-ide-bp-gutter');
  if (await bp.count()) expect((await bp.boundingBox())?.width ?? 0).toBeLessThanOrEqual(1);
}

async function structureRows(page: Page): Promise<string[]> {
  await page.getByTestId('ide-tree-tab-structure').click();
  const rows = page.locator('.struct-row');
  await expect(rows.first()).toBeVisible({ timeout: 15_000 });
  return (await rows.allTextContents()).map((t) => t.replace(/\s+/g, ' ').trim());
}

const CASES: { file: string; probe: string | RegExp; tokens: [string | RegExp, string][]; structure?: (rows: string[]) => void; jump?: [RegExp, number] }[] = [
  {
    file: 'application.properties',
    probe: 'spring.datasource.url=',
    tokens: [
      ['spring.datasource.url=', 'spring.datasource.url'],
      ['spring.datasource.url=', PLACEHOLDER],
      ['spring.datasource.url=', '='],
      ['# Games service', '# Games service: database and Kafka'],
      ['spring.datasource.url=', 'jdbc:mysql://'],
      ['spring.kafka.consumer.properties=', 'false'],
      ['spring.kafka.consumer.properties=', '500'],
    ],
    structure: (rows) => {
      expect(rows.some((r) => /^group spring ?:2/.test(r)), rows.join(' | ')).toBe(true);
      expect(rows.some((r) => /spring\.kafka\.consumer\.properties ?:4/.test(r))).toBe(true);
      expect(rows.some((r) => /server\.port ?:5/.test(r))).toBe(true);
      expect(rows.some((r) => /continue on/.test(r)), 'a continuation is not a key').toBe(false);
    },
    jump: [/server\.port ?:5/, 5],
  },
  {
    file: 'deploy.yaml',
    probe: 'apiVersion: apps/v1',
    tokens: [['apiVersion: apps/v1', 'apiVersion'], ['apiVersion: apps/v1', 'apps/v1'], ['# Deployment', '# Deployment'], ['replicas: 3', '3'], ['labels: &labels', '&labels'], ['---', '---']],
    structure: (rows) => {
      expect(rows[0], rows.join(' | ')).toMatch(/^doc Document 1 ?:1/);
      expect(rows.some((r) => /^doc Document 2 ?:14/.test(r))).toBe(true);
      expect(rows.some((r) => /prop replicas ?:9/.test(r))).toBe(true);
    },
    jump: [/prop replicas ?:9/, 9],
  },
  {
    file: 'Cargo.toml',
    probe: '[package]',
    tokens: [['[package]', 'package'], ['name = "otto"', 'name'], ['name = "otto"', '"otto"'], ['# the edition', '# the edition'], ['serde = ', '{']],
    structure: (rows) => {
      expect(rows[0], rows.join(' | ')).toMatch(/^group package ?:1/);
      expect(rows.some((r) => /^group dependencies ?:7/.test(r))).toBe(true);
      expect(rows.some((r) => /prop tokio ?:9/.test(r))).toBe(true);
    },
    jump: [/prop tokio ?:9/, 9],
  },
  { file: 'Dockerfile', probe: 'FROM golang', tokens: [['FROM golang', 'FROM'], ['FROM golang', 'build'], ['# build stage', '# build stage'], ['ENV CGO', 'CGO_ENABLED'], ['RUN go build', '-o']] },
  { file: 'prod.env', probe: 'export DB_URL', tokens: [['export DB_URL', 'export'], ['export DB_URL', 'DB_URL'], ['API_KEY=', '"abc123"'], ['# local settings', '# local settings'], ['RETRIES=3', '3']] },
  { file: 'service.overrides', probe: 'host = db.internal', tokens: [['host = db.internal', 'host'], ['host = db.internal', 'db.internal'], ['# per-host', '# per-host overrides'], ['[cache]', 'cache'], ['port = 5432', '5432']] },
];

for (const c of CASES) {
  test(c.file + ': its own language, distinct token colours, text next to the line numbers' + (c.structure ? ', Structure lists the keys' : ''), async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await seedConfigFiles();
    await bootWriteIde(page);
    await openTreeFile(page, c.file);
    await expectDistinct(page, c.probe, c.tokens);
    await expectNoUnusedGutters(page);
    if (c.structure) {
      c.structure(await structureRows(page));
      if (c.jump) {
        await page.locator('.struct-row').filter({ hasText: c.jump[0] }).first().click();
        await expect(page.getByTestId('ide-status-cursor')).toHaveText(new RegExp('^Ln ' + c.jump[1] + ','), { timeout: 10_000 });
      }
    }
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test('screenshots: .properties, YAML and TOML (' + scheme + ')', async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await seedConfigFiles();
    await bootWriteIde(page, scheme);
    for (const [file, probe] of [['application.properties', 'spring.datasource.url='], ['deploy.yaml', 'apiVersion: apps/v1'], ['Cargo.toml', '[package]']] as const) {
      await page.getByTestId('ide-tree-tab-files').click();
      await openTreeFile(page, file);
      await expect(line(page, probe).locator('span[class^="ͼ"]').first()).toBeVisible({ timeout: 15_000 });
      await expect.poll(() => textBand(page), { timeout: 10_000 }).toBeLessThanOrEqual(30);
      await page.getByTestId('ide-tree-tab-structure').click();
      await expect(page.locator('.struct-row').first()).toBeVisible({ timeout: 15_000 });
      await writeShot(page, file.replace(/\W+/g, '-') + '-' + scheme);
    }
  });
}
