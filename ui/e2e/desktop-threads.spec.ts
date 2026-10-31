import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Threads experimental workspace (#/threads & #/threads/<sessionId>):
//   • Sidebar navigation under Work → Threads + Experimental badge + New Thread
//     welcome stage with harness cards and zero horizontal overflow;
//   • Multi-session 2-up Split canvas, Pinned threads, PandaCode-style open file
//     tabs + Markdown preview, Connections side-by-side action, and real
//     session-to-session message relay via POST /api/v1/sessions/:id/message.
//
// Seeded sessions use inert shell PTYs + synthetic JSONL transcripts so zero
// external coding CLI binary is needed in CI.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';
let sessionA = '';
let sessionB = '';

function transcriptLine(uuid: string, sessionId: string, text: string): string {
  return (
    JSON.stringify({
      type: 'user',
      uuid,
      sessionId,
      timestamp: '2026-10-31T12:00:00Z',
      message: { role: 'user', content: text },
    }) + '\n'
  );
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;

  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-'));
  writeFileSync(
    join(fixtureDir, 'README.md'),
    '# Threads Workspace\n\nLive multi-session stage and file viewer smoke fixture.\n',
  );

  const wsResp = await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'Threads Smoke WS', root_path: fixtureDir },
  });
  expect(wsResp.ok(), await wsResp.text()).toBeTruthy();
  wsId = (await wsResp.json()).id as string;

  const trA = join(fixtureDir, '00000000-0000-4000-8000-000000000a01.jsonl');
  const trB = join(fixtureDir, '00000000-0000-4000-8000-000000000b02.jsonl');
  writeFileSync(trA, transcriptLine('turn-a1', '00000000-0000-4000-8000-000000000a01', 'Alpha architect ready'));
  writeFileSync(trB, transcriptLine('turn-b1', '00000000-0000-4000-8000-000000000b02', 'Beta worker standing by'));

  const rA = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'Alpha Architect',
      cwd: fixtureDir,
      meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: trA },
    },
  });
  expect(rA.ok(), await rA.text()).toBeTruthy();
  sessionA = (await rA.json()).id as string;

  const rB = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'Beta Worker',
      cwd: fixtureDir,
      meta: {
        origin: 'e2e',
        nested_provider: 'codex',
        delegated_by: sessionA,
        e2e_transcript_path: trB,
      },
    },
  });
  expect(rB.ok(), await rB.text()).toBeTruthy();
  sessionB = (await rB.json()).id as string;

  await expect
    .poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${sessionA}`)).json()).live)
    .toBe(true);
  await expect
    .poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${sessionB}`)).json()).live)
    .toBe(true);
});

test.afterAll(async () => {
  for (const id of [sessionB, sessionA]) {
    if (id && ctx) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
  }
  await ctx?.dispose();
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('threads-e2e-init')) return;
    sessionStorage.setItem('threads-e2e-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.removeItem('otto_threads_v1');
    localStorage.removeItem('otto_sidebar_hidden');
    localStorage.removeItem('otto_sidebar_groups_collapsed');
  }, wsId);
});

test('sidebar navigates to #/threads and renders New Thread stage without overflow', async ({ page }) => {
  await page.goto('/#/home');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });

  const threadsNav = page
    .getByTestId('sidebar-group-work')
    .getByRole('button', { name: 'Threads', exact: true });
  await expect(threadsNav).toBeVisible();
  await threadsNav.click();

  await expect(page.getByTestId('threads-page')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('threads-experimental-badge')).toContainText('Experimental');

  await page.getByTestId('threads-new-btn').click();
  await expect(page.getByTestId('new-thread-stage')).toBeVisible();
  await expect(page.getByTestId('harness-claude')).toBeVisible();
  await expect(page.getByTestId('new-thread-input')).toBeVisible();

  await expectNoHorizontalOverflow(page);
});

test('multi-session split canvas, pins, file viewer, and peer-to-peer message relay', async ({ page }) => {
  await page.goto(`/#/threads/${sessionA}`);
  await expect(page.getByTestId(`thread-pane-${sessionA}`)).toBeVisible({ timeout: 30_000 });

  // Pin Alpha Architect and verify it surfaces in Pinned section.
  await page.getByTestId(`thread-pin-${sessionA}`).click();
  await expect(page.getByTestId(`thread-pinned-${sessionA}`)).toBeVisible();

  // Open Beta Worker beside Alpha Architect in 2-up Split canvas.
  await page.getByTestId(`thread-split-${sessionB}`).click();
  const canvas = page.getByTestId('threads-canvas');
  await expect(canvas.getByTestId(`thread-pane-${sessionA}`)).toBeVisible();
  await expect(canvas.getByTestId(`thread-pane-${sessionB}`)).toBeVisible();

  // Open README.md in PandaCode-style file tabs and inspect rendered Markdown.
  const fileTabs = page.getByTestId('inspector-files-tab');
  await expect(fileTabs).toBeVisible();
  await fileTabs.getByRole('button', { name: 'README.md' }).first().click();
  await expect(fileTabs.getByRole('heading', { name: 'Threads Workspace' })).toBeVisible();

  // Connections inspector tab exposes one-click side-by-side DB Explorer launch.
  const inspector = page.getByTestId('threads-inspector');
  await inspector.getByRole('tab', { name: 'Connections' }).click();
  await expect(page.getByTestId('connections-open-beside')).toBeVisible();

  // Peers inspector tab relays a direct session-to-session message from Alpha → Beta.
  await canvas.getByTestId(`thread-pane-${sessionA}`).click();
  await inspector.getByRole('tab', { name: 'Peers' }).click();
  await page.getByTestId('peers-target-select').selectOption(sessionB);
  const relayMsg = 'Sync schema check from Alpha';
  await page.getByTestId('peers-message-input').fill(relayMsg);

  const sentPromise = page.waitForResponse(
    (r) =>
      r.url().includes(`/api/v1/sessions/${sessionB}/message`) &&
      r.request().method() === 'POST',
  );
  await page.getByTestId('peers-send-btn').click();
  const sentResp = await sentPromise;
  expect(sentResp.ok()).toBeTruthy();

  await expect(page.getByTestId('peers-timeline')).toContainText(relayMsg);

  // Narrow viewport drawer layout stays free of horizontal overflow.
  await page.setViewportSize({ width: 720, height: 800 });
  await expectNoHorizontalOverflow(page);
});
