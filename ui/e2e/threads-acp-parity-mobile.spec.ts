import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// ACP threads on otto-fake-agent (global-setup shims gemini / agy / grok):
// what the agent sends reaches the thread and the person's answers reach the
// agent (docs/features/threads-structured-runtime.md › ACP per provider).
//   • the agent switches its own mode → the access chip follows
//   • url-mode elicitation: the card names the URL, choosing it opens it
//   • "Always reject" (ACP reject_always) is offered and sent
//   • a notice and a compaction are quiet timeline lines; an image answer row
//   • tools that need no approval fold; an edit → the "N changed files" card
// Needs OTTO_E2E_BIN on a build with otto-fake-agent beside it.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-acp-parity-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# ACP parity\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'ACP WS', root_path: fixtureDir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_prefs_v1');
    // window.open is how a plain-web build opens a link: record it.
    (window as unknown as { __opened: string[] }).__opened = [];
    window.open = ((url?: string | URL) => {
      (window as unknown as { __opened: string[] }).__opened.push(String(url));
      return null;
    }) as typeof window.open;
  }, wsId);
});


async function open(page: Page, provider: 'agy' | 'gemini', title: string): Promise<Locator> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: fixtureDir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  return thread;
}


for (const where of ['phone'] as const) {
  test(`${where}: tools that need no approval fold; an edit shows the changed-files card`, async ({ page }, info) => {
    test.skip(info.project.name !== 'iphone-portrait', 'iphone-portrait project only');
    test.setTimeout(90_000);
    const thread = await open(page, 'gemini', `ACP fold ${where}`);
    const composer = thread.getByTestId('runtime-composer');
    const sendBy = async (text: string) => {
      await composer.fill(text);
      const send = thread.getByTestId('runtime-send');
      await send.tap();
    };
    await sendBy('tool');
    await expect(thread.getByText('done', { exact: true }).last()).toBeVisible({ timeout: 20_000 });
    const fold = thread.locator('[data-testid^="runtime-fold-"]');
    await expect(fold).toHaveCount(1);
    await expect(fold).not.toHaveAttribute('open', '');
    await sendBy('edit src/a.rs');
    await expect(thread.getByText('edited').last()).toBeVisible({ timeout: 20_000 });
    const changes = thread.getByTestId('runtime-turn-changes');
    await expect(changes).toContainText('1 changed file');
    {
      const box = (await changes.getByRole('button').first().boundingBox())!;
      expect(box.height).toBeGreaterThanOrEqual(44);
    }
  });
}
