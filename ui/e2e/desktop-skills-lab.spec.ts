import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Skills Lab — focused E2E for the new capabilities, against the real isolated
// daemon (no coding-agent CLIs; the harness runs CLAUDE_BIN=/nonexistent):
//   • library skill CRUD (create / list / multi-file edit / delete)
//   • a STATIC skill review completing with a deterministic report (no agents)
//   • UI smoke: the 3-tab Skills Lab shell, the skills browser, and — the
//     "don't void the evaluator" regression — the Evaluator tab still renders.
//
// Desktop-only (data view, not a mobile-layout sweep) → runs once so the shared
// daemon library never sees a duplicate skill name across projects.
// ─────────────────────────────────────────────────────────────────────────────

test.describe.configure({ mode: 'serial' });

test.beforeEach(({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'Skills Lab spec runs on desktop only');
});

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const SKILL = 'e2e-lab-skill';

test.beforeAll(async () => {
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  wsId = await seedWorkspace(ctx, base);
});

test('API: library skill CRUD — create, multi-file edit, delete', async () => {
  const body = [
    '---',
    'description: An E2E Skills Lab fixture skill used to exercise review.',
    'category: review',
    'version: 1',
    '---',
    '',
    '## Workflow',
    '',
    '1. Read the input.',
    '2. Produce the output.',
    '',
    '## Output',
    '',
    'A short result. Use when asked to test the lab.',
    '',
    '## Example',
    '',
    'Example: given X, return Y.',
  ].join('\n');

  // Create.
  let r = await ctx.post(`${base}/api/v1/library/skills`, {
    data: { name: SKILL, category: 'review', description: 'E2E lab skill', body },
  });
  expect(r.ok()).toBeTruthy();

  // Creating the same name again conflicts (409).
  r = await ctx.post(`${base}/api/v1/library/skills`, { data: { name: SKILL, category: 'review', description: 'dup' } });
  expect(r.status()).toBe(409);

  // List contains it.
  r = await ctx.get(`${base}/api/v1/library/skills`);
  const list = (await r.json()) as { name: string }[];
  expect(list.some((s) => s.name === SKILL)).toBeTruthy();

  // File tree has SKILL.md.
  r = await ctx.get(`${base}/api/v1/library/skills/${SKILL}/files`);
  const files = (await r.json()) as { path: string }[];
  expect(files.some((f) => f.path === 'SKILL.md')).toBeTruthy();

  // Write a nested reference file, read it back.
  r = await ctx.put(`${base}/api/v1/library/skills/${SKILL}/file`, {
    data: { path: 'references/notes.md', content: '# notes\nsome guidance' },
  });
  expect(r.ok()).toBeTruthy();
  r = await ctx.get(`${base}/api/v1/library/skills/${SKILL}/file?path=references%2Fnotes.md`);
  expect(((await r.json()) as { content: string }).content).toContain('some guidance');

  // Path traversal is rejected.
  r = await ctx.get(`${base}/api/v1/library/skills/${SKILL}/file?path=..%2F..%2Fsecret`);
  expect(r.ok()).toBeFalsy();

  // SKILL.md cannot be deleted; a reference file can.
  r = await ctx.delete(`${base}/api/v1/library/skills/${SKILL}/file?path=SKILL.md`);
  expect(r.ok()).toBeFalsy();
  r = await ctx.delete(`${base}/api/v1/library/skills/${SKILL}/file?path=references%2Fnotes.md`);
  expect(r.ok()).toBeTruthy();
});

test('API: static skill review completes with a deterministic report', async () => {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/skill-reviews`, {
    data: { skill_name: SKILL, skill_source: 'library', providers: [], agent_mode: 'static' },
  });
  expect(r.ok()).toBeTruthy();
  const rev = (await r.json()) as { id: string; status: string };

  // Poll to a terminal state (static-only ⇒ near-instant, no agents).
  let review = rev as {
    id: string;
    status: string;
    static_report: { verdict: string; scorecard: unknown[] } | null;
  };
  for (let i = 0; i < 40 && review.status === 'running'; i++) {
    await new Promise((res) => setTimeout(res, 250));
    const g = await ctx.get(`${base}/api/v1/skill-reviews/${rev.id}`);
    review = await g.json();
  }
  expect(review.status).toBe('done');
  expect(review.static_report).toBeTruthy();
  expect(['Ready', 'Ready with fixes', 'Do not publish']).toContain(review.static_report!.verdict);
  expect(review.static_report!.scorecard.length).toBeGreaterThan(5);

  // It shows up in the workspace review list.
  const listR = await ctx.get(`${base}/api/v1/workspaces/${wsId}/skill-reviews`);
  const reviews = (await listR.json()) as { id: string }[];
  expect(reviews.some((x) => x.id === rev.id)).toBeTruthy();
});

test('UI: Skills Lab tabs, skills browser, and preserved evaluator', async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);

  await page.goto('/#/skills-eval');

  // The umbrella shell + its three sections.
  await expect(page.locator('[data-testid="lab-tabs"]')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('[data-testid="tab-skills"]')).toBeVisible();
  await expect(page.locator('[data-testid="tab-review"]')).toBeVisible();
  await expect(page.locator('[data-testid="tab-evaluator"]')).toBeVisible();

  // Skills tab (default) — the browser lists our created library skill.
  await expect(page.locator('[data-testid="skills-browser"]')).toBeVisible();
  await expect(page.getByRole('button', { name: SKILL }).first()).toBeVisible({ timeout: 15_000 });

  // Review tab — the review panel + new-review entry render.
  await page.locator('[data-testid="tab-review"]').click();
  await expect(page.locator('[data-testid="skill-review"]')).toBeVisible();
  await expect(page.locator('[data-testid="new-skill-review"]')).toBeVisible();

  // Evaluator tab — the existing evaluator is preserved (regression guard).
  await page.locator('[data-testid="tab-evaluator"]').click();
  await expect(page.locator('[data-testid="eval-tabs"]')).toBeVisible({ timeout: 20_000 });
});
