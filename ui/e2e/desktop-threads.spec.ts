import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Threads experimental workspace (#/threads & #/threads/<sessionId>):
//   • Custom-sidebar auto-discovery of newly shipped `threads` module;
//   • Strict session isolation between `#/threads` (`meta.surface = 'threads'`)
//     and `#/agents` (classic agent sessions);
//   • T3Code / PandaCode parity controls: interactive Model, Effort, and Mode
//     pickers (plus ⇧⇥ Plan toggle), interactive in-thread `/` slash commands,
//     single unified pane header, and Plan Mode approval bar;
//   • macOS WKWebView PUA function-key guard (`\uF700–\uF8FF` arrow keys);
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
let sessionPlan = '';
let agentOnlySession = '';

function userLine(uuid: string, sessionId: string, text: string): string {
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

function assistantLine(uuid: string, sessionId: string, text: string): string {
  return (
    JSON.stringify({
      type: 'assistant',
      uuid,
      requestId: `req_${uuid}`,
      sessionId,
      timestamp: '2026-10-31T12:00:04Z',
      message: {
        id: `msg_${uuid}`,
        role: 'assistant',
        model: 'claude-sonnet-4-6',
        content: [{ type: 'text', text }],
        stop_reason: 'end_turn',
        usage: {
          input_tokens: 400,
          output_tokens: 120,
          cache_read_input_tokens: 0,
          cache_creation_input_tokens: 0,
        },
      },
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
  const trPlan = join(fixtureDir, '00000000-0000-4000-8000-000000000c03.jsonl');
  const trAgent = join(fixtureDir, '00000000-0000-4000-8000-000000000d04.jsonl');

  writeFileSync(
    trA,
    userLine('turn-a1', '00000000-0000-4000-8000-000000000a01', 'Alpha architect ready') +
      assistantLine('turn-a2', '00000000-0000-4000-8000-000000000a01', 'Standing by on Alpha.'),
  );
  writeFileSync(trB, userLine('turn-b1', '00000000-0000-4000-8000-000000000b02', 'Beta worker standing by'));
  writeFileSync(
    trPlan,
    userLine('turn-p1', '00000000-0000-4000-8000-000000000c03', 'Draft an auth migration plan') +
      assistantLine(
        'turn-p2',
        '00000000-0000-4000-8000-000000000c03',
        '## Plan\n1. Add token validator\n2. Wire route middleware',
      ),
  );
  writeFileSync(trAgent, userLine('turn-g1', '00000000-0000-4000-8000-000000000d04', 'Classic agent prompt'));

  const rA = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'Alpha Architect',
      cwd: fixtureDir,
      meta: {
        origin: 'e2e',
        surface: 'threads',
        nested_provider: 'claude',
        e2e_transcript_path: trA,
      },
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
        surface: 'threads',
        nested_provider: 'codex',
        delegated_by: sessionA,
        e2e_transcript_path: trB,
      },
    },
  });
  expect(rB.ok(), await rB.text()).toBeTruthy();
  sessionB = (await rB.json()).id as string;

  const rPlan = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'Planner Thread',
      cwd: fixtureDir,
      meta: {
        origin: 'e2e',
        surface: 'threads',
        interaction_mode: 'plan',
        nested_provider: 'claude',
        e2e_transcript_path: trPlan,
      },
    },
  });
  expect(rPlan.ok(), await rPlan.text()).toBeTruthy();
  sessionPlan = (await rPlan.json()).id as string;

  const rAgent = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'Classic Agent Session',
      cwd: fixtureDir,
      meta: {
        origin: 'e2e',
        nested_provider: 'claude',
        e2e_transcript_path: trAgent,
      },
    },
  });
  expect(rAgent.ok(), await rAgent.text()).toBeTruthy();
  agentOnlySession = (await rAgent.json()).id as string;

  for (const id of [sessionA, sessionB, sessionPlan, agentOnlySession]) {
    await expect
      .poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${id}`)).json()).live)
      .toBe(true);
  }
});

test.afterAll(async () => {
  for (const id of [agentOnlySession, sessionPlan, sessionB, sessionA]) {
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
    localStorage.removeItem('otto_sidebar_groups_collapsed');
  }, wsId);
});

test('custom sidebar auto-discovers Threads, stage pickers + slash menu work, and PUA arrow keys are guarded', async ({
  page,
}) => {
  // Simulate an existing user with a customized sidebar saved before `threads` shipped.
  await page.addInitScript(() => {
    if (sessionStorage.getItem('threads-custom-sidebar-seeded')) return;
    sessionStorage.setItem('threads-custom-sidebar-seeded', '1');
    localStorage.setItem('otto_sidebar_order', JSON.stringify(['home', 'agents', 'git']));
    localStorage.setItem('otto_sidebar_hidden', JSON.stringify(['browser']));
    localStorage.removeItem('otto_sidebar_known_modules');
  });

  await page.goto('/#/home');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });

  const threadsNav = page
    .getByTestId('sidebar-group-work')
    .getByRole('button', { name: 'Threads', exact: true });
  await expect(threadsNav).toBeVisible();
  await threadsNav.click();

  await expect(page.getByTestId('threads-page')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('threads-new-btn').click();
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible();
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('New thread');
  await expect(stage.locator('h1')).toContainText(/What should we (build|work on)/);

  // One agent chip holds harness · model · effort; access mode sits in the meta row.
  // Claude runs on the structured runtime (always asks first), so the access
  // mode is shown for a PTY harness (Antigravity).
  const agentBtn = stage.getByTestId('picker-agent');
  const modeBtn = stage.getByTestId('picker-mode');
  await expect(agentBtn).toBeVisible();
  await expect(modeBtn).toHaveCount(0);

  // One panel: provider rail · models · Reasoning · access (T3).
  await agentBtn.click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await expect(picker.getByTestId('model-picker-provider-claude')).toBeVisible();
  await expect(picker.getByTestId('model-picker-search')).toBeFocused();
  await picker.getByTestId('model-picker-provider-agy').click();
  await expect(modeBtn).toContainText('Full access');
  await picker.getByTestId('picker-effort').getByRole('button', { name: 'High', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(picker).toHaveCount(0);
  await expect(agentBtn).toContainText('High');

  // ⇧⇥ inside the composer toggles Plan mode.
  const input = stage.getByTestId('new-thread-input');
  await input.focus();
  await page.keyboard.press('Shift+Tab');
  await expect(modeBtn).toContainText('Plan');

  // Typing `/` opens the interactive slash-command popup.
  await input.fill('/');
  await expect(stage.locator('[data-slash-pop]')).toBeVisible();

  // macOS WKWebView U+F703 (NSRightArrowFunctionKey) beforeinput must NOT insert □.
  await input.fill('abc');
  const guardResult = await input.evaluate((el: HTMLTextAreaElement) => {
    el.focus();
    el.setSelectionRange(1, 1);
    const ev = new InputEvent('beforeinput', {
      bubbles: true,
      cancelable: true,
      inputType: 'insertText',
      data: '\uF703',
    });
    el.dispatchEvent(ev);
    return {
      value: el.value,
      selStart: el.selectionStart,
      hasPua: /[\uF700-\uF8FF]/.test(el.value),
    };
  });
  expect(guardResult).toEqual({ value: 'abc', selStart: 2, hasPua: false });

  await expectNoHorizontalOverflow(page);
});

test('strict Threads vs Agents isolation, single pane header, interactive slash commands, and Plan bar', async ({
  page,
}) => {
  // 1. Classic Agents surface shows `Classic Agent Session` and hides `Alpha Architect`.
  await page.goto('/#/agents');
  const nav = page.locator('.navigator');
  await expect(nav).toBeVisible({ timeout: 30_000 });
  await expect(nav.getByText('Classic Agent Session').first()).toBeVisible();
  await expect(nav.getByText('Alpha Architect')).toHaveCount(0);

  // 2. Threads surface shows `Alpha Architect` and hides `Classic Agent Session`.
  await page.goto(`/#/threads/${sessionA}`);
  const paneA = page.getByTestId(`thread-pane-${sessionA}`);
  await expect(paneA).toBeVisible({ timeout: 30_000 });
  const tSidebar = page.getByTestId('threads-sidebar');
  await expect(tSidebar.getByText('Alpha Architect').first()).toBeVisible();
  await expect(tSidebar.getByText('Classic Agent Session')).toHaveCount(0);

  // 3. Single view: the page top bar is the only header (breadcrumb names the
  //    thread); no pane header and no duplicate .conv-head inside ConversationView.
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('Alpha Architect');
  await expect(paneA.locator('.tp-head')).toHaveCount(0);
  await expect(paneA.locator('.conv-head')).toHaveCount(0);
  // The right panel starts closed (T3); its toggle opens the surface launcher.
  await expect(page.getByTestId('threads-inspector')).toHaveCount(0);
  await page.getByTestId('threads-toggle-inspector').click();
  await expect(page.getByTestId('threads-surface-launcher')).toBeVisible();
  await expect(page.getByTestId('threads-surface-files')).toBeEnabled();
  // Browser opens as an inspector tab — never the shell's Side panel beside
  // the inspector (that was two right columns).
  await page.getByTestId('threads-surface-browser').click();
  const insp = page.getByTestId('threads-inspector');
  await expect(insp.getByRole('tab', { name: 'Browser' })).toHaveAttribute('aria-selected', 'true');
  await expect(insp.locator('.browser')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.browser')).toHaveCount(1);

  // 4. Interactive Model / Effort / Mode pickers & `/` slash commands inside live ThreadPane.
  await expect(paneA.getByTestId('picker-model')).toBeVisible();
  await expect(paneA.getByTestId('picker-mode')).toBeVisible();
  await paneA.getByTestId('picker-model').click();
  await expect(paneA.getByTestId('picker-effort')).toBeVisible();
  await page.keyboard.press('Escape');

  const ta = paneA.locator('.composer textarea');
  await ta.fill('/');
  await expect(paneA.locator('[data-slash-pop]')).toBeVisible();

  await ta.fill('/status');
  await ta.press('Enter');
  await expect(paneA.getByTestId('thread-status-card')).toBeVisible();

  await ta.fill('/diff');
  await ta.press('Enter');
  await expect(
    page.getByTestId('threads-inspector').getByRole('tab', { name: 'Diff' }),
  ).toHaveAttribute('aria-selected', 'true');

  // 5. Plan-mode thread displays the Plan approval bar with both implementation actions.
  await page.goto(`/#/threads/${sessionPlan}`);
  const panePlan = page.getByTestId(`thread-pane-${sessionPlan}`);
  await expect(panePlan).toBeVisible({ timeout: 15_000 });
  await expect(panePlan.getByTestId('plan-action-bar')).toBeVisible();
  await expect(panePlan.getByTestId('implement-plan-btn')).toBeVisible();
  await expect(panePlan.getByTestId('implement-plan-beside-btn')).toBeVisible();
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
  if ((await page.getByTestId('threads-inspector').count()) === 0) {
    await page.getByTestId('threads-toggle-inspector').click();
  }
  await page.getByTestId('threads-surface-add').click();
  await page.getByRole('menuitem', { name: /^Files/ }).click();
  const fileTabs = page.getByTestId('inspector-files-tab');
  await expect(fileTabs).toBeVisible();
  await fileTabs.getByRole('button', { name: 'README.md' }).first().click();
  await expect(fileTabs.getByRole('heading', { name: 'Threads Workspace' })).toBeVisible();

  // Connections inspector tab exposes one-click side-by-side DB Explorer launch.
  const inspector = page.getByTestId('threads-inspector');
  await inspector.getByTestId('threads-surface-add').click();
  await page.getByRole('menuitem', { name: /Connections/ }).click();
  await expect(page.getByTestId('connections-open-beside')).toBeVisible();

  // Peers inspector tab relays a direct session-to-session message from Alpha → Beta.
  await canvas.getByTestId(`thread-pane-${sessionA}`).click();
  await inspector.getByTestId('threads-surface-add').click();
  await page.getByRole('menuitem', { name: /Peers/ }).click();
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

test('phone handoff opens the same live agent in Threads and returns to Terminal', async ({ page }, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const before = await (await ctx.get(`${base}/api/v1/sessions/${agentOnlySession}`)).json();
  // A phone legitimately resizes the terminal; ownership metadata must survive.
  const stableMeta = Object.fromEntries(Object.entries(before.meta).filter(([key]) => !['pty_cols', 'pty_rows'].includes(key)));
  const spawned: string[] = [];
  page.on('request', req => {
    if (req.method() === 'POST' && /\/(?:runtime-)?sessions$/.test(new URL(req.url()).pathname)) spawned.push(req.url());
  });
  await page.goto(`/#/agents/${agentOnlySession}`);
  const handoff = page.locator('.pane-head').getByRole('button', { name: 'Threads', exact: true });
  await expect(handoff).toBeVisible();
  expect((await handoff.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  expect((await page.locator('.pane-head').boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await page.screenshot({ path: info.outputPath('mobile-handoff-agents.png') });
  await handoff.click();
  await expect(page).toHaveURL(new RegExp(`#/threads/${agentOnlySession}$`));
  await expect(page.getByText('Classic agent prompt', { exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  const during = await (await ctx.get(`${base}/api/v1/sessions/${agentOnlySession}`)).json();
  expect(during.id).toBe(before.id);
  expect(during.provider_session_id).toBe(before.provider_session_id);
  expect(during.live).toBe(true);
  expect(during.meta).toMatchObject({ ...stableMeta, surface: 'threads' });
  const actions = page.getByTestId('threads-thread-menu');
  if (await actions.isVisible()) await actions.click();
  else {
    await page.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Thread actions', exact: true }).click();
  }
  await page.getByRole('menuitem', { name: 'Open in Terminal', exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${agentOnlySession}$`));
  await expect(page.locator('.term-host')).toBeVisible();
  const after = await (await ctx.get(`${base}/api/v1/sessions/${agentOnlySession}`)).json();
  expect(after.id).toBe(before.id);
  expect(after.live).toBe(true);
  expect(after.meta).toMatchObject({ ...stableMeta, surface: 'agents' });
  expect(spawned).toEqual([]);
});
