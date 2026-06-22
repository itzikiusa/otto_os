import { test, expect, type Page, type TestInfo } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── Full mobile sweep of the Product feature ───────────────────────────────────
// Seeds ONE complete story (every sub-surface populated) into the isolated test
// daemon, then walks each Product sub-flow and writes a viewport screenshot per
// device project. Runs across iPhone/iPad portrait+landscape + iPhone SE, so a
// single `npm run test:e2e -- product-sweep` produces the horizontal (landscape)
// and vertical (portrait) views the reviewer asked for.
//
//   OTTO_SHOT_PHASE=before|after   → e2e/.shots/<phase>/<project>/NN-name.png
//   PHASE 'after' also ASSERTS no horizontal overflow on every flow.

const PHASE = process.env.OTTO_SHOT_PHASE ?? 'before';
const PHONE = new Set(['iphone-portrait', 'iphone-se']);

// Playwright's default actionTimeout is 0 (infinite) — bound every action so a
// non-actionable element fails fast and our per-flow try/catches keep the sweep
// moving instead of burning the whole test budget on one stuck click.
test.use({ actionTimeout: 10_000 });

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  // Sanity: confirm the seeded rows surface through every endpoint the tabs hit,
  // and TIME each one so a slow/hanging endpoint is obvious in the log.
  const probe = async (label: string, path: string) => {
    const t0 = Date.now();
    const r = await ctx.get(`${base}/api/v1${path}`);
    const ms = Date.now() - t0;
    let n = 0;
    try {
      const j = await r.json();
      n = Array.isArray(j) ? j.length : Array.isArray(j?.sections) ? j.sections.length : 1;
    } catch {
      n = -1;
    }
    // eslint-disable-next-line no-console
    console.log(`[seed probe] ${label.padEnd(12)} ${r.status()} n=${n} ${ms}ms`);
  };
  await probe('analyses', `/product/stories/${storyId}/analyses`);
  await probe('versions', `/product/stories/${storyId}/versions`);
  await probe('questions', `/product/stories/${storyId}/questions`);
  await probe('notes', `/product/stories/${storyId}/notes`);
  await probe('events', `/product/stories/${storyId}/events`);
  await probe('testcases', `/product/stories/${storyId}/testcases`);
  await probe('learnings', `/workspaces/${workspaceId}/product/learnings`);
  await probe('inject', `/product/stories/${storyId}/inject`);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function isPhone(info: TestInfo): boolean {
  return PHONE.has(info.project.name);
}

async function shotsDir(info: TestInfo): Promise<string> {
  const dir = join(process.cwd(), 'e2e', '.shots', PHASE, info.project.name);
  mkdirSync(dir, { recursive: true });
  return dir;
}

async function openProduct(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  // The app holds a persistent WebSocket, so 'networkidle' never settles; wait
  // for the story list to have loaded (the seeded row appears) instead.
  await page.locator('.story-row').first().waitFor({ timeout: 15_000 }).catch(() => {});
}

async function pageHasClass(page: Page, cls: string): Promise<boolean> {
  const c = (await page.locator('.product-page').getAttribute('class')) ?? '';
  return c.includes(cls);
}

async function ensureListOpen(page: Page, info: TestInfo): Promise<void> {
  if (!isPhone(info)) return;
  if (!(await pageHasClass(page, 'm-list-open'))) {
    await page.locator('.m-acc-head').nth(0).click();
  }
}

async function ensureContentOpen(page: Page, info: TestInfo): Promise<void> {
  if (!isPhone(info)) return;
  if (!(await pageHasClass(page, 'm-content-open'))) {
    await page.locator('.m-acc-head').nth(1).click();
  }
}

/** Click the visible Stories|Learnings view toggle. */
async function setView(page: Page, name: 'Stories' | 'Learnings'): Promise<void> {
  const tab = page.getByRole('tab', { name }).locator('visible=true').first();
  await tab.click();
}

async function selectStory(page: Page, info: TestInfo): Promise<void> {
  await ensureListOpen(page, info);
  await setView(page, 'Stories');
  const row = page.locator('.story-row').first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();
  // Detail loads; on phone selectStory flips to the content panel.
  await ensureContentOpen(page, info);
}

async function openTab(page: Page, info: TestInfo, label: string): Promise<void> {
  await ensureContentOpen(page, info);
  const tab = page.locator('.tab-strip .st', { hasText: label }).first();
  await tab.scrollIntoViewIfNeeded();
  await tab.click();
}

/** Wait for a flow's real content (data-dependent) to render before snapping,
 *  so screenshots show populated state rather than "Loading…" placeholders. */
async function settleFor(page: Page, sel: string | null, ms = 600): Promise<void> {
  // First, let any in-content "Loading …" placeholder clear (data arrived).
  await page
    .getByText(/^Loading .*…$/)
    .first()
    .waitFor({ state: 'detached', timeout: 12_000 })
    .catch(() => {});
  if (sel) {
    await page.locator(sel).first().waitFor({ timeout: 9_000 }).catch(() => {});
  }
  await page.waitForTimeout(ms);
}

async function overflowPx(page: Page): Promise<number> {
  return page.evaluate(() => {
    const el = document.documentElement;
    return el.scrollWidth - el.clientWidth;
  });
}

const violations: { project: string; flow: string; px: number }[] = [];

async function shot(page: Page, info: TestInfo, name: string): Promise<void> {
  const dir = await shotsDir(info);
  // Let layout settle before measuring/snapping.
  await page.waitForTimeout(350);
  const px = await overflowPx(page);
  if (px > 2) violations.push({ project: info.project.name, flow: name, px });
  await page.screenshot({ path: join(dir, `${name}.png`) });
}

test('product mobile sweep', async ({ page }, info) => {
  test.setTimeout(240_000);

  // DEBUG: trace product API requests so a hung/slow fetch is visible.
  if (process.env.OTTO_SWEEP_DEBUG) {
    page.on('requestfinished', (req) => {
      const u = req.url();
      if (u.includes('/api/v1/product') || u.includes('/api/v1/workspaces')) {
        // eslint-disable-next-line no-console
        console.log(`  [req fin] ${req.method()} ${u.split('/api/v1')[1]}`);
      }
    });
    page.on('requestfailed', (req) => {
      const u = req.url();
      if (u.includes('/api/v1/product') || u.includes('/api/v1/workspaces')) {
        // eslint-disable-next-line no-console
        console.log(`  [req FAIL] ${u.split('/api/v1')[1]} → ${req.failure()?.errorText}`);
      }
    });
    page.on('request', (req) => {
      const u = req.url();
      if (u.includes('/testcases')) {
        // eslint-disable-next-line no-console
        console.log(`  [req START] ${req.method()} ${u.split('/api/v1')[1]}`);
      }
    });
    page.on('console', (msg) => {
      if (msg.type() === 'error' || /mutation|Error/i.test(msg.text())) {
        // eslint-disable-next-line no-console
        console.log(`  [console.${msg.type()}] ${msg.text().slice(0, 200)}`);
      }
    });
    page.on('pageerror', (err) => {
      // eslint-disable-next-line no-console
      console.log(`  [pageerror] ${err.message.slice(0, 200)}`);
    });
  }

  // 1. Story list / sidebar (import + new-draft affordances).
  await openProduct(page);
  await ensureListOpen(page, info);
  await shot(page, info, '01-stories-list');

  // 2. Import dialog (shared Modal, width 480 — narrow-screen stress).
  try {
    await page.getByRole('button', { name: 'Import', exact: false }).first().click();
    await expect(page.getByText('Import story').first()).toBeVisible({ timeout: 8_000 });
    await shot(page, info, '02-import-dialog');
    await page.keyboard.press('Escape');
    await page.waitForTimeout(200);
  } catch {
    /* import affordance shape varies; skip if not reachable */
  }

  // 3. Select the seeded story → Overview (draft editor = "create a new one").
  await selectStory(page, info);
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
  await shot(page, info, '03-overview');

  // 4. Analysis — configure panel.
  await openTab(page, info, 'Analysis');
  await expect(page.locator('.analysis-tab')).toBeVisible({ timeout: 15_000 });
  await shot(page, info, '04-analyze-config');

  // 5. Analysis — viewing analyzed results (pick the seeded run from History).
  try {
    const sel = page.locator('.hist-select');
    // Wait for the history GET to populate the dropdown. Native <option> elements
    // aren't "visible" until the select opens, so poll the COUNT instead.
    await expect(sel.locator('option')).toHaveCount(2, { timeout: 9_000 });
    await sel.selectOption({ index: 1 });
    // Wait for the synthesized summary + findings to render, then scroll the
    // results into view so the screenshot shows them (not just the config panel).
    await page.locator('.synthesis-card').first().waitFor({ timeout: 9_000 }).catch(() => {});
    await page.locator('.findings-card').first().waitFor({ timeout: 9_000 }).catch(() => {});
    await page.locator('.synthesis-card').first().scrollIntoViewIfNeeded({ timeout: 3_000 }).catch(() => {});
    await page.waitForTimeout(500);
  } catch {
    /* keep going; capture whatever rendered */
  }
  await shot(page, info, '05-analysis-results');
  await page.locator('.product-body').first().evaluate((el) => (el.scrollTop = 0)).catch(() => {});

  // 6. Questions.
  await openTab(page, info, 'Questions');
  await expect(page.locator('.qtab')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.q-card');
  await shot(page, info, '06-questions');

  // 7. Notes.
  await openTab(page, info, 'Notes');
  await expect(page.locator('.ntab')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.n-card');
  await shot(page, info, '07-notes');

  // 8. Rewrite (before/after diff).
  await openTab(page, info, 'Rewrite');
  await expect(page.locator('.rewrite-tab')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.vm-row', 900); // version bodies fetch + DiffView render
  await shot(page, info, '08-rewrite');

  // 9. Test cases (grouped by category).
  await openTab(page, info, 'Test Cases');
  await expect(page.locator('.tc-tab')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.case-card');
  await shot(page, info, '09-testcases');

  // 10. Plan (task tree + multi-provider planning + swarm link).
  await openTab(page, info, 'Plan');
  await expect(page.locator('.plan-tab')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.tasks', 700);
  await shot(page, info, '10-plan');

  // 11. History (event timeline).
  await openTab(page, info, 'History');
  await expect(page.locator('.history-tab')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.event-row');
  await shot(page, info, '11-history');

  // 12. Inject (build the bundle, then snapshot the rendered sections).
  await openTab(page, info, 'Inject');
  await expect(page.locator('.inject-tab')).toBeVisible({ timeout: 15_000 });
  try {
    await page.locator('.action-btn.primary').first().click();
    // The button flips to "Rebuild" once the bundle is assembled.
    await page.getByRole('button', { name: 'Rebuild' }).first().waitFor({ timeout: 9_000 });
    await page.waitForTimeout(500);
  } catch {
    /* bundle build is best-effort */
  }
  await shot(page, info, '12-inject');

  // 13. Learnings (global library view). On phone the view toggle lives in the
  // list panel, so open that first, switch, then reveal the content panel.
  await ensureListOpen(page, info);
  await setView(page, 'Learnings');
  await ensureContentOpen(page, info);
  await expect(page.locator('.learnings-view')).toBeVisible({ timeout: 15_000 });
  await settleFor(page, '.card-title');
  await shot(page, info, '13-learnings');
});

test.afterAll(() => {
  if (violations.length) {
    // eslint-disable-next-line no-console
    console.log('\n[product-sweep] horizontal-overflow violations:');
    for (const v of violations) {
      // eslint-disable-next-line no-console
      console.log(`  ${v.project.padEnd(18)} ${v.flow.padEnd(22)} +${v.px}px`);
    }
  }
  if (PHASE === 'after') {
    expect(violations, `overflow on: ${violations.map((v) => `${v.project}/${v.flow}`).join(', ')}`).toEqual([]);
  }
});
