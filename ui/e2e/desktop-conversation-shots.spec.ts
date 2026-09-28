import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { appendRecords, toolUse, writeChatTranscript } from './chat-fixture';

// Design screenshots of the chat (docs/design/conversation-view.md §5.2):
// wide + narrow (6-pane tiled grid), light + dark, working and waiting. Opt-in —
// runs only with OTTO_CHAT_SHOTS=<dir> (files are named `<prefix>-<shot>.png`,
// prefix from OTTO_CHAT_SHOTS_PREFIX, default "chat"). Nothing is asserted
// beyond the chat being on screen; the pictures are the output.

const OUT = process.env.OTTO_CHAT_SHOTS ?? '';
const PREFIX = process.env.OTTO_CHAT_SHOTS_PREFIX ?? 'chat';

test.setTimeout(180_000);

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const dirs: string[] = [];

async function seedChat(title: string): Promise<{ id: string; path: string }> {
  const { dir, path } = writeChatTranscript();
  dirs.push(dir);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: path } },
  });
  if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  return { id: (await r.json()).id as string, path };
}

async function open(page: Page, scheme: 'light' | 'dark', ids: string[], first: string): Promise<void> {
  await page.emulateMedia({ colorScheme: scheme });
  await page.addInitScript(
    ({ ws, ids, scheme }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_firstrun_dismissed', '1');
      localStorage.setItem('otto_nav_all_ws', '0');
      localStorage.setItem('otto_theme', 'native');
      localStorage.setItem('otto_scheme', scheme);
      for (const id of ids) localStorage.setItem(`otto_session_view:${id}`, 'chat');
    },
    { ws: wsId, ids, scheme },
  );
  await page.goto(`/#/agents/${first}`);
  await expect(page.locator('.conv[data-loaded="true"]').first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.conv .turn[data-role="assistant"]').first()).toBeVisible({ timeout: 30_000 });
}

async function shot(page: Page, name: string): Promise<void> {
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(OUT, `${PREFIX}-${name}.png`) });
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  test.skip(!OUT, 'set OTTO_CHAT_SHOTS=<dir> to take the design screenshots');
  mkdirSync(OUT, { recursive: true });
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
});

test.afterEach(async () => {
  await ctx?.dispose();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

for (const scheme of ['light', 'dark'] as const) {
  test(`wide chat, ${scheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const s = await seedChat('Flaky retry test');
    await open(page, scheme, [s.id], s.id);
    await shot(page, `wide-${scheme}`);
    // Scroll to the top of the first response, open its steps and a failing command.
    const conv = page.locator('.conv').first();
    const groups = conv.locator('.steps:not(.single) .steps-head, [data-steps-toggle]');
    for (let i = 0; i < (await groups.count()); i++) {
      const g = groups.nth(i);
      if ((await g.getAttribute('aria-expanded')) !== 'true') await g.click();
    }
    const failed = conv.locator('.step[data-status="err"] .step-row').first();
    if (await failed.count()) await failed.click();
    const edit = conv.locator('.step[data-tool="edit"] .step-row').first();
    if (await edit.count()) await edit.click();
    await conv.locator('.step[data-status="err"]').first().scrollIntoViewIfNeeded().catch(() => {});
    await shot(page, `wide-${scheme}-steps`);
    await conv.locator('.step[data-tool="edit"]').first().scrollIntoViewIfNeeded().catch(() => {});
    await shot(page, `wide-${scheme}-diff`);
  });

  test(`narrow tiled panes, ${scheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    const seeded = [];
    for (const t of ['Retry fix', 'Docs pass', 'Perf probe', 'Review', 'Migrate', 'Triage']) seeded.push(await seedChat(t));
    await open(page, scheme, seeded.map((s) => s.id), seeded[0].id);
    const tiled = page.locator('button[aria-label="Tiled view"]');
    if (await tiled.count()) {
      await tiled.first().click();
      await expect(page.locator('[data-tile-id]').first()).toBeVisible({ timeout: 20_000 });
      await expect(page.locator('.conv[data-loaded="true"]')).toHaveCount(6, { timeout: 30_000 });
    }
    await shot(page, `narrow-${scheme}`);
  });

  test(`working state, ${scheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    const s = await seedChat('Working agent');
    await open(page, scheme, [s.id], s.id);
    // Keep the PTY busy → the session reads as working; a pending tool call is
    // what the agent is doing right now.
    await ctx.post(`${base}/api/v1/sessions/${s.id}/input`, { data: { text: 'while true; do echo tick; sleep 1; done', submit: true } });
    appendRecords(s.path, toolUse('9', 'toolu_live', 'Bash', { command: 'cargo test -p otto-net --release', description: 'Run the release tests' }));
    await page.waitForTimeout(4000);
    await shot(page, `working-${scheme}`);
  });

  test(`waiting for you, ${scheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    const s = await seedChat('Waiting agent');
    await open(page, scheme, [s.id], s.id);
    await expect
      .poll(async () => ((await (await ctx.get(`${base}/api/v1/sessions/${s.id}`)).json()) as { status: string }).status, { timeout: 30_000 })
      .not.toBe('working');
    appendRecords(s.path, toolUse('7', 'toolu_perm', 'Bash', { command: 'rm -rf target/', description: 'Clean the build' }));
    await expect(page.locator('[data-live-status="waiting"]')).toBeVisible({ timeout: 15_000 });
    await shot(page, `waiting-${scheme}`);
  });
}
