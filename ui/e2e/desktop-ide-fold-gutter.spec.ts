import { test, expect, type Locator, type Page } from '@playwright/test';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Fold chevrons must sit on the line that OPENS the fold. CodeMirror lays
// fold-gutter markers out with margin-top gaps in a flex column; a theme that
// centred that column pushed every marker down (onto closing braces, or past
// EOF in a short file). Covers a Go file with a folded `import ( … )` and a
// JUnit class, and checks the markers follow a fold and a document edit.

const SHOT_DIR = process.env.OTTO_E2E_FOLD_SHOTS;

const GO = `package main

import (
\t"fmt"
\t"os"
\t"strings"
)

type Item struct {
\tName string
\tTags []string
}

func main() {
\titems := []Item{{Name: "a"}}
\tfor _, it := range items {
\t\tfmt.Println(strings.ToUpper(it.Name))
\t}
\tos.Exit(0)
}

// end
`;

const JAVA = `package demo;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class FilterTest {

    @Test
    void filter_keeps_matches() {
        int a = 1;
        assertEquals(1, a);
    }

    @Test
    void filter_drops_others() {
        int b = 2;
        assertEquals(2, b);
    }

    @Test
    void filter_handles_empty() {
        int c = 0;
        assertEquals(0, c);
    }
}
`;

let ws = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  ws = await seedWorkspace(ctx, base, 'Fold gutter');
  for (const [path, content] of [['fold-e2e.go', GO], ['FoldE2eTest.java', JAVA]]) {
    const put = await ctx.put(`${base}/api/v1/scratches/file`, { data: { path, content } });
    expect(put.ok()).toBe(true);
  }
  await ctx.dispose();
});

/** For every fold marker: the text of the editor line it is drawn against
 *  (null when it sits below the last line). */
async function markerLines(ed: Locator): Promise<(string | null)[]> {
  return ed.evaluate((ed) => {
    const lines = [...ed.querySelectorAll('.cm-content .cm-line')].map((l) => {
      const r = l.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, text: (l.textContent ?? '').trim() };
    });
    return [...ed.querySelectorAll('.cm-foldGutter .cm-gutterElement')]
      // Skip CodeMirror's hidden width spacer.
      .filter((el) => el.querySelector('.cm-fold-marker') && getComputedStyle(el).visibility !== 'hidden')
      .map((el) => {
        const r = el.getBoundingClientRect();
        const mid = (r.top + r.bottom) / 2;
        return lines.find((l) => mid >= l.top && mid < l.bottom)?.text ?? null;
      });
  });
}

/** Opens a scratch and returns its (visible) editor. */
async function openScratch(page: Page, name: string, marker: string): Promise<Locator> {
  await page.getByTestId('ide-project-tree').getByText(name).dblclick();
  const ed = page.locator('.cm-editor').filter({ hasText: marker });
  await expect(ed.locator('.cm-foldGutter .cm-fold-marker').filter({ visible: true }).first()).toBeVisible();
  return ed;
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await page.goto('/#/ide?scratches=1');
  await expect(page.getByTestId('ide-target-switcher')).toContainText('Scratches');
});

test('Go: chevrons sit on the opening lines, also after folding the import', async ({ page }) => {
  const ed = await openScratch(page, 'fold-e2e.go', 'package main');
  const before = await markerLines(ed);
  if (SHOT_DIR) await page.screenshot({ path: join(SHOT_DIR, 'go.png') });
  // Line text includes inline widgets ("…" placeholder, "1 usage" lens): match prefixes.
  const opener = /^(import \(|type Item struct \{|func main\(\) \{|for _, it := range items \{)/;
  expect(before.some((t) => t?.startsWith('import ('))).toBe(true);
  expect(before.some((t) => t?.startsWith('func main() {'))).toBe(true);
  for (const t of before) expect(t, `marker against ${t}`).toMatch(opener);

  // Fold the import from its chevron: the others keep their lines, none passes EOF.
  const importMarker = ed.locator('.cm-foldGutter .cm-gutterElement').filter({ has: page.locator('.cm-fold-marker'), visible: true }).first();
  const foldedBefore = await ed.locator('.cm-foldPlaceholder').count(); // auto-fold may have run
  await importMarker.click();
  await expect(ed.locator('.cm-foldPlaceholder')).toHaveCount(foldedBefore ? 0 : 1);
  const folded = await markerLines(ed);
  if (SHOT_DIR) await page.screenshot({ path: join(SHOT_DIR, 'go-import-toggled.png') });
  expect(folded).not.toContain(null);
  const key = (ts: (string | null)[]) => ts.map((t) => t?.match(opener)?.[0] ?? t);
  expect(key(folded)).toEqual(key(before));
});

test('Java: chevrons sit on the method lines, not the closing braces', async ({ page }) => {
  const ed = await openScratch(page, 'FoldE2eTest.java', 'class FilterTest');
  const got = await markerLines(ed);
  if (SHOT_DIR) await page.screenshot({ path: join(SHOT_DIR, 'java.png') });
  for (const m of ['filter_keeps_matches', 'filter_drops_others', 'filter_handles_empty']) {
    expect(got).toContain(`void ${m}() {`);
  }
  expect(got).not.toContain(null);
  expect(got).not.toContain('}');

  // An edit above shifts the methods; the chevrons follow them.
  await ed.locator('.cm-content .cm-line', { hasText: 'class FilterTest {' }).click();
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  await page.keyboard.type('// added');
  await page.keyboard.press('Enter');
  await expect(ed.locator('.cm-content')).toContainText('// added');
  const edited = await markerLines(ed);
  expect(edited).toContain('void filter_handles_empty() {');
  expect(edited).not.toContain(null);
  expect(edited).not.toContain('}');
});
