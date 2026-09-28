import { expect, test, type APIRequestContext, type Page } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedVaultDir, seedWorkspace } from './seed';
import { openPage } from './helpers';
import { domCount, requestLog } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Docs, visual editors and orchestration — perf regression gates (I6,
// GAPS_TO_9_5 §7/§8). Budgets are DOM / request COUNTS, not timings.
//
//   • GFM markdown renders with ONE DOM parse and the same HTML as the old
//     marked → sanitize → re-parse → tabindex → re-serialize path (V10).
//   • The vault graph makes ≤ 1 `graph` request per 20 s while idle (SD-03).
//   • A 5k-row CSV mounts ≤ 200 table rows; a 5k-line code file mounts a
//     windowed slice, not 5k lines (V6).
//   • Insights with 500 reports mounts ≤ 200 list rows (SE-17).
//   • A Design Hall metadata change patches ONE card: 0 `/design/search`
//     reloads, ≤ 1 artifact GET (SD-16).
// Unit-level halves live in ui/unit (renderQueue, mdSingleParse, siteStudio,
// runProgress, vaultGraphSeed) and the crates (MCP pool, vault revision index,
// Confluence converter, canvas format column, review wait).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
test.describe.configure({ mode: 'serial', timeout: 120_000 });

let workspaceId = '';
let vaultId = 0;
let vaultDir = '';
const V1 = '/api/v1';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
    const seeded = await seedVaultDir(ctx, base, workspaceId, { notes: 200 });
    vaultId = seeded.vaultId;
    vaultDir = seeded.dir;
    // V6 fixtures: a 5k-row CSV and a 5k-line source file.
    const csv = ['id,name,team,score,notes'];
    for (let i = 0; i < 5000; i++) csv.push(`${i},name-${i},team-${i % 17},${i % 100},"note, ${i}"`);
    writeFileSync(join(vaultDir, 'big.csv'), csv.join('\n'));
    writeFileSync(
      join(vaultDir, 'big.ts'),
      Array.from({ length: 5000 }, (_, i) => `export const v${i} = ${i}; // line ${i}`).join('\n'),
    );
    expect((await ctx.post(`${base}${V1}/workspaces/${workspaceId}/vault/vaults/${vaultId}/rescan`, { data: {} })).ok()).toBeTruthy();
  } finally {
    await ctx.dispose();
  }
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(({ ws, id }) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem(`otto_vault_last:${ws}`, String(id));
    localStorage.setItem('otto_rail_expanded', '0');
  }, { ws: workspaceId, id: vaultId });
});

test('GFM markdown: one DOM parse, output identical to the old triple-parse path', async ({ page }) => {
  await openPage(page, 'vault');
  const r = await page.evaluate(async () => {
    const md = await import('/src/lib/md.ts' as string);
    const { sanitizeHtml } = await import('/src/lib/sanitize.ts' as string);
    const { marked } = await import('/node_modules/marked/lib/marked.esm.js' as string);
    const fixtures = [
      '# Title\n\nPara with **bold**, _em_, `code` and a [link](https://example.com).\n',
      '| a | b |\n|---|---|\n| 1 | <b onclick="x()">2</b> |\n\n```js\nlet x = 1;\n```\n',
      '- [x] done\n- [ ] todo\n  - nested\n\n> quote\n\n<script>alert(1)</script><img src="javascript:x" onerror="y()">\n',
      Array.from({ length: 200 }, (_, i) => `## H${i}\n\n\`\`\`\ncode ${i}\n\`\`\`\n\n| c | ${i} |\n|---|---|\n| x | y |\n`).join('\n'),
    ];
    const old = (src: string): string => {
      const html = sanitizeHtml(marked.parse(src, { async: false, gfm: true, breaks: false }) as string);
      const doc = new DOMParser().parseFromString(html, 'text/html');
      for (const b of doc.querySelectorAll('pre, table')) b.setAttribute('tabindex', '0');
      return doc.body.innerHTML;
    };
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const proto = DOMParser.prototype as any;
    const orig = proto.parseFromString;
    let parses = 0;
    proto.parseFromString = function (this: DOMParser, ...a: unknown[]) {
      parses++;
      return orig.apply(this, a);
    };
    const out: { same: boolean; parses: number }[] = [];
    try {
      for (const f of fixtures) {
        parses = 0;
        const next = md.renderMarkdownGfm(f) as string;
        const p = parses;
        out.push({ same: next === old(f), parses: p });
      }
    } finally {
      proto.parseFromString = orig;
    }
    return out;
  });
  for (const x of r) {
    expect(x.same, 'single-parse output equals the old path').toBe(true);
    expect(x.parses).toBe(1);
  }
});

test('vault graph: at most one graph request per 20 s while idle', async ({ page }) => {
  await openPage(page, 'vault');
  await page.locator('button[title="Graph view"]').click();
  await expect(page.locator('.center canvas')).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(1500);
  const log = requestLog(page, /\/vault\/vaults\/\d+\/graph/);
  await page.waitForTimeout(20_000);
  log.stop();
  expect(log.stats().count, log.stats().paths.join('\n')).toBeLessThanOrEqual(1);
});

async function openVaultFile(page: Page, name: string): Promise<void> {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await expect(tree.getByText(name, { exact: true })).toBeVisible({ timeout: 15_000 });
  await tree.getByText(name, { exact: true }).click();
}

test('vault FileViewer: a 5k-row CSV mounts at most 200 rows and grows on demand', async ({ page }) => {
  await openVaultFile(page, 'big.csv');
  await expect(page.locator('.file-view table')).toBeVisible({ timeout: 15_000 });
  expect(await domCount(page, '.file-view tbody tr')).toBeLessThanOrEqual(200);
  await page.locator('.file-view .more-rows').click();
  expect(await domCount(page, '.file-view tbody tr')).toBeGreaterThan(200);
});

test('vault FileViewer: a 5k-line code file is windowed, not highlighted eagerly', async ({ page }) => {
  await openVaultFile(page, 'big.ts');
  await expect(page.locator('.code-ln').first()).toBeVisible({ timeout: 15_000 });
  expect(await domCount(page, '.code-ln')).toBeLessThan(300);
  expect(await domCount(page, '.file-view pre.code')).toBe(0);
});

test('insights: 500 reports mount at most 200 list rows', async ({ page }) => {
  const reports = Array.from({ length: 500 }, (_, i) => {
    const d = new Date(Date.UTC(2025, 0, 1) + i * 86_400_000).toISOString().slice(0, 10);
    return { kind: 'daily', period_start: d, period_end: d, summary: `# Day ${i}\n\nA summary.`, html_path: '', created_at: `${d}T08:00:00Z` };
  }).reverse();
  await page.route('**/api/v1/insights/reports', (r) => r.fulfill({ json: reports }));
  await openPage(page, 'insights');
  const list = page.getByTestId('report-list');
  await expect(list).toBeVisible({ timeout: 15_000 });
  expect(await domCount(page, '[data-testid="report-list"] .rep-row')).toBeLessThanOrEqual(200);
  await expect(list.locator('.more-reports')).toBeVisible();
});

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()}`).toBeTruthy();
  return r.json();
}

test('design hall: a metadata change patches one card, no full library reload', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  let id = '';
  try {
    for (let i = 0; i < 12; i++) {
      const res = await postJson(ctx, `${base}${V1}/design/artifacts`, {
        workspace_id: workspaceId,
        format: 'html',
        studio: 'frames',
        title: `Perf frame ${i}`,
        content: `<!doctype html><html><body><h1>Frame ${i}</h1></body></html>`,
      });
      id = res.artifact.id; // the newest: shown in "Recent"
    }
    await page.goto('/#/design');
    await expect(page.getByTestId('design-lobby')).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('Perf frame 11').first()).toBeVisible({ timeout: 15_000 });
    await page.waitForTimeout(1000);
    const search = requestLog(page, /\/api\/v1\/design\/search/);
    const one = requestLog(page, new RegExp(`/api/v1/design/artifacts/${id}(\\?|$)`));
    const r = await ctx.patch(`${base}${V1}/design/artifacts/${id}`, { data: { title: 'Perf frame renamed' } });
    expect(r.ok()).toBeTruthy();
    await expect(page.getByText('Perf frame renamed').first()).toBeVisible({ timeout: 10_000 });
    await page.waitForTimeout(1000);
    search.stop();
    one.stop();
    expect(search.stats().count, search.stats().paths.join('\n')).toBe(0);
    expect(one.stats().count).toBeLessThanOrEqual(1);
  } finally {
    await ctx.dispose();
  }
});
