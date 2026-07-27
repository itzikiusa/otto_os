import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Commit-graph lane LEGIBILITY (desktop-browser only).
//
// Regression (2026-07-27): the gutter drew each node's own lane only BELOW the
// node — nothing from the top of the row down to it. Pass-through lanes ('vert')
// were drawn full-height, so the branch you were actually following rendered as
// a dashed line while lanes you didn't care about rendered solid. On a 28px row
// with r=4 that left a 10px blank in EVERY row (a 36% duty cycle).
//
// The lane's own column is now split around the node: the upper half is drawn
// when a lane above feeds into this commit, the lower half when it has a parent.
// A branch TIP therefore has no ink above it and a ROOT none below — those two
// are correct, everything between them must be continuous.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-lanes-repo';
let repoDir = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  repoDir = mkdtempSync(join(tmpdir(), 'otto-e2e-lanes-'));
  // Every commit gets its own minute. Without this the whole history shares one
  // timestamp and `git log` orders equal-dated commits arbitrarily, so which row
  // is the root stops being deterministic.
  let clock = 0;
  const stamp = () => {
    const d = `2026-01-01T12:${String(clock++).padStart(2, '0')}:00`;
    return { GIT_AUTHOR_DATE: d, GIT_COMMITTER_DATE: d };
  };
  const git = (...a: string[]) =>
    execFileSync('git', ['-C', repoDir, ...a], {
      stdio: 'ignore',
      env: { ...process.env, ...stamp() },
    });
  const commit = (file: string, body: string, msg: string) => {
    writeFileSync(join(repoDir, file), body);
    git('add', '.');
    git('commit', '-q', '-m', msg);
  };
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');

  // A mainline with a tag, a feature branch that runs alongside it, and a merge
  // back — i.e. at least two concurrent lanes plus a merge node, which is the
  // shape the lane renderer has to get right.
  commit('app.txt', 'v1\n', 'init');
  commit('app.txt', 'v2\n', 'main: second');
  git('tag', 'v1.0.0');
  git('checkout', '-q', '-b', 'feature/lanes');
  commit('feat.txt', 'a\n', 'feature: first');
  commit('feat.txt', 'ab\n', 'feature: second');
  git('checkout', '-q', 'main');
  commit('app.txt', 'v3\n', 'main: third');
  git('merge', '-q', '--no-ff', 'feature/lanes', '-m', 'Merge feature/lanes');
  commit('app.txt', 'v4\n', 'main: after merge');

  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

async function openGraph(page: import('@playwright/test').Page): Promise<void> {
  await openPage(page, 'git');
  const existingTab = page.locator('.git-tab-name', { hasText: REPO_NAME });
  if (await existingTab.count()) {
    await existingTab.first().click();
  } else {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(REPO_NAME);
    await menu.getByRole('menuitem', { name: REPO_NAME }).first().click();
  }
  await expect(page.locator('.rv-tabs')).toBeVisible();
  await expect(page.locator('.graph-row').first()).toBeVisible({ timeout: 15_000 });
}

/**
 * Per-row lane geometry, read straight out of the rendered SVG: the node's
 * centre plus whether its OWN column carries ink above / below it. Everything is
 * derived from the DOM (never from the component's internals) so this stays a
 * true rendering assertion.
 */
async function laneGeometry(page: import('@playwright/test').Page) {
  return page.$$eval('.graph-row:not(.wip-row)', (rows) =>
    rows.map((row) => {
      const svg = row.querySelector('svg.gutter')!;
      const node = svg.querySelector('circle:last-of-type') as SVGCircleElement | null;
      const subject = row.querySelector('.ci-subject')?.textContent?.trim() ?? '';
      if (!node) return { subject, cx: -1, above: false, below: false };
      const cx = Number(node.getAttribute('cx'));
      const cy = Number(node.getAttribute('cy'));
      const r = Number(node.getAttribute('r'));
      // Only straight <line> elements in the node's own column count; the curved
      // merge/converge <path>s belong to other lanes.
      const at = [...svg.querySelectorAll('line')].filter(
        (l) =>
          Number(l.getAttribute('x1')) === cx && Number(l.getAttribute('x2')) === cx,
      );
      const above = at.some(
        (l) => Number(l.getAttribute('y1')) === 0 && Number(l.getAttribute('y2')) >= cy - r,
      );
      const below = at.some((l) => Number(l.getAttribute('y1')) <= cy + r && Number(l.getAttribute('y2')) > cy);
      return { subject, cx, above, below };
    }),
  );
}

test('a lane is continuous through its commits — no dashed spine', async ({ page }) => {
  await openGraph(page);
  const geo = await laneGeometry(page);
  expect(geo.length, 'seeded commits should render').toBeGreaterThanOrEqual(6);

  // The oldest commit is the ROOT: nothing below it (the old renderer trailed a
  // stub off it into empty space).
  const root = geo[geo.length - 1];
  expect(root.subject, 'last row is the root commit').toBe('init');
  expect(root.below, 'root commit must not trail a line below it').toBe(false);
  expect(root.above, 'root commit is fed from the row above').toBe(true);

  // Every commit except a branch tip must be fed from above. With the whole
  // history loaded the only tips are the newest commit on each lane, so at most
  // a couple of rows may lack it — the old renderer had ZERO rows with ink
  // above, which is what this guards.
  const withAbove = geo.filter((g) => g.above).length;
  expect(
    withAbove,
    `rows fed from above (${withAbove}/${geo.length}) — a dashed spine shows up as 0`,
  ).toBeGreaterThanOrEqual(geo.length - 2);

  // And the mainline commits in the middle of history are continuous on BOTH
  // sides, which is the actual "solid line" property.
  const midline = geo.find((g) => g.subject === 'main: third')!;
  expect(midline.above && midline.below, '"main: third" continuous through its row').toBe(true);
});

test('ref chips are tied to their lane, and the columns are labelled', async ({ page }) => {
  await openGraph(page);

  // Column header names the three zones.
  const head = page.locator('.graph-head');
  await expect(head).toBeVisible();
  await expect(head).toContainText('BRANCH / TAG');
  await expect(head).toContainText('GRAPH');
  await expect(head).toContainText('COMMIT MESSAGE');

  // A chip carries its lane's color on the edge facing the graph, and the gutter
  // draws a leader line from the chip across to the node — together that is what
  // makes "which branch is this?" answerable at a glance.
  const chipRow = page.locator('.graph-row', { has: page.locator('.ref-chip') }).first();
  await expect(chipRow).toBeVisible();
  const laneColor = await chipRow
    .locator('.ref-chip')
    .first()
    .evaluate((el) => getComputedStyle(el).borderInlineEndColor);
  expect(laneColor, 'chip lane bar should be colored, not transparent').not.toMatch(
    /transparent|rgba\(0, 0, 0, 0\)/,
  );

  const leader = await chipRow.locator('svg.gutter line').evaluateAll((lines) =>
    lines.some((l) => Number(l.getAttribute('y1')) === Number(l.getAttribute('y2'))),
  );
  expect(leader, 'a horizontal leader line should join the chip to its node').toBe(true);
});
