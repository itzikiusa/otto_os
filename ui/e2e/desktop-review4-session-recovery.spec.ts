import {test, expect, type APIRequestContext, type Page} from '@playwright/test';
import type {HistoryEntry, HistoryStatus, Transcript, Turn} from '../src/lib/api/types';
import {apiCtx, seedWorkspace} from './seed';
import {withUsage} from './chat-fixture';
import {heapAfterGC, watchLongTasks, longTasks, dist} from './perf';
import {execFileSync} from 'node:child_process';

test.use({serviceWorkers: 'block'});
let ctx: APIRequestContext, base: string, wsId: string, sessionId: string;

test.beforeEach(async ({page}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser acceptance');
  ({ctx, base} = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {kind: 'agent', provider: 'shell', title: 'Recovery fixture', cwd: '/tmp', meta: {nested_provider: 'claude', origin: 'e2e'}},
  });
  expect(response.ok()).toBe(true);
  sessionId = (await response.json()).id;
  await withUsage(page);
  await page.addInitScript(({wsId, sessionId}) => {
    localStorage.setItem('otto_workspace', wsId);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem(`otto_session_view:${sessionId}`, 'chat');
  }, {wsId, sessionId});
});
test.afterEach(async () => { await ctx?.dispose(); });

function turn(index: number): Turn {
  return {id: `child-${index}`, role: index % 2 ? 'assistant' : 'user', ts: null,
    blocks: [{kind: 'text', md: `Child checkpoint ${index}.`}], duration_ms: null, model: null, system: [], reasoning_steps: 0};
}
function transcriptPage(turns: Turn[], cursor = '0', has_earlier = false): Transcript {
  return {session_id: sessionId, provider: 'claude', title: 'Recovery fixture', cwd: '/tmp', model: null,
    cursor, has_earlier, turns, subagents: [], unavailable_reason: null,
    stats: {turns: 180, tool_calls: 0, cost_usd: null, input_tokens: null, output_tokens: null,
      duration_ms: null, reasoning_steps: 0, thinking_steps: 0, unknown_records: 0}};
}
async function openChat(page: Page) {
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
}

test('child history pages both directions and refetches after collapse without accumulating turns', async ({page}) => {
  const requested: (string | null)[] = [];
  await page.route(`**/sessions/${sessionId}/transcript?*`, route => {
    const params = new URL(route.request().url()).searchParams;
    if (!params.has('sub')) {
      const parent = turn(1);
      parent.blocks = [{kind: 'subagent', agent_id: 'child', description: 'Paged child', agent_type: 'reviewer', status: 'done'}];
      return route.fulfill({json: transcriptPage([parent])});
    }
    expect(params.get('sub')).toBe('child');
    const before = params.get('before'); requested.push(before);
    const end = before === null ? 180 : Number(before), start = Math.max(0, end - 60);
    return route.fulfill({json: transcriptPage(Array.from({length: end - start}, (_, i) => turn(start + i)), String(start), start > 0)});
  });
  await openChat(page);
  const steps = page.locator('.conv .steps-head');
  await expect(steps).toBeVisible();
  if (await steps.getAttribute('aria-expanded') === 'false') await steps.click();
  const card = page.locator('.sub[data-agent="child"]');
  await card.locator('.sub-head').click();
  await expect(card).toContainText('Child checkpoint 179.');
  await expect(card.locator('.sub-body .turn')).toHaveCount(60);
  await card.getByRole('button', {name: 'Load earlier', exact: true}).click();
  await expect(card).toContainText('Child checkpoint 60.');
  await expect(card).not.toContainText('Child checkpoint 179.');
  await expect(card.locator('.sub-body .turn')).toHaveCount(60);
  await card.getByRole('button', {name: 'Load earlier', exact: true}).click();
  await expect(card).toContainText('Child checkpoint 0.');
  await expect(card.getByRole('button', {name: 'Load earlier', exact: true})).toHaveCount(0);
  await card.getByRole('button', {name: 'Load newer', exact: true}).click();
  await expect(card).toContainText('Child checkpoint 60.');
  await card.locator('.sub-head').click();
  await expect(card.locator('.sub-body')).toHaveCount(0);
  await card.locator('.sub-head').click();
  await expect(card).toContainText('Child checkpoint 60.');
  await card.getByRole('button', {name: 'Load newer', exact: true}).click();
  await expect(card).toContainText('Child checkpoint 179.');
  await expect(card.locator('.sub-body .turn')).toHaveCount(60);
  await expect(card.getByRole('button', {name: 'Load newer', exact: true})).toHaveCount(0);
  expect(requested).toEqual([null, '120', '60', '120', '120', null]);
});

test('mixed session batch retains submitted settings and retries only its failed member', async ({page}) => {
  const submitted: Record<string, unknown>[] = [];
  await page.route(`**/workspaces/${wsId}/sessions`, async route => {
    if (route.request().method() !== 'POST') return route.continue();
    submitted.push(route.request().postDataJSON());
    if (submitted.length === 2) return route.fulfill({status: 503, json: {code: 'unavailable', message: 'Synthetic second-member failure'}});
    return route.continue();
  });
  await page.goto(`/#/agents/${sessionId}`);
  await page.getByRole('button', {name: 'New session', exact: true}).click();
  const dialog = page.getByRole('dialog', {name: 'New session', exact: true});
  await dialog.locator('.provider-card', {hasText: 'shell'}).locator('.card-main').click();
  await dialog.getByLabel('One more shell session').click();
  await dialog.locator('#ns-title').fill('Recovery batch');
  await dialog.locator('#ns-cwd').fill('/tmp');
  await dialog.getByRole('button', {name: 'Start 2 sessions', exact: true}).click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('status')).toContainText('1 started; 1 still need to start');
  expect(submitted).toHaveLength(2);
  await dialog.locator('#ns-title').fill('Changed after failure');
  await dialog.locator('#ns-cwd').fill('/');
  await dialog.getByRole('button', {name: 'Retry 1 failed', exact: true}).click();
  await expect(dialog).toHaveCount(0);
  expect(submitted).toHaveLength(3);
  expect(submitted[2]).toEqual(submitted[1]);
  const rows = await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/sessions`)).json() as {title: string}[];
  expect(rows.filter(row => row.title === 'Recovery batch 1')).toHaveLength(1);
  expect(rows.filter(row => row.title === 'Recovery batch 2')).toHaveLength(1);
  expect(rows).toHaveLength(3);
});

async function historyFixture(page: Page, row: () => HistoryEntry) {
  await page.route('**/api/v1/workspaces/*/history/page?*', route => route.fulfill({json: {entries: [row()], next_cursor: null}}));
  await page.route(`**/sessions/${sessionId}/transcript?*`, route => route.fulfill({json: transcriptPage([turn(1)])}));
  await page.route('**/api/v1/workspaces/*/history/transcript?*', route => route.fulfill({json: transcriptPage([turn(1)])}));
}
function historyRow(status: HistoryStatus): HistoryEntry {
  const stamp = new Date().toISOString();
  return {session_id: status === 'on_disk' ? null : sessionId, provider: 'claude', title: 'History recovery fixture',
    first_prompt: 'Review', cwd: '/tmp', repo_name: 'Fixture', started_at: stamp, last_active_at: stamp,
    turns: 1, status, transcript_path: '/tmp/synthetic-recovery.jsonl', resumable: true};
}

test('History live and stale inactive rows open through safe resume and override a warmed Terminal preference', async ({page}) => {
  test.setTimeout(90_000);
  let status: HistoryStatus = 'working';
  await historyFixture(page, () => historyRow(status));
  const calls: string[] = [];
  await page.route(`**/sessions/${sessionId}/resume`, route => {calls.push('resume'); return route.continue();});
  await page.route(`**/sessions/${sessionId}/restart`, route => {calls.push('restart'); return route.fulfill({status: 500});});
  for (const state of ['working', 'running', 'idle', 'exited', 'reconnectable'] as const) {
    await test.step(state, async () => {
      status = state;
      await openChat(page);
      await page.locator('.view-seg button', {hasText: 'Terminal'}).click();
      await expect(page.locator('.conv')).toHaveCount(0);
      // Keep the same document/store: a direct reload would hide the warmed
      // preference bug by recreating the reactive preference cache.
      await page.evaluate(() => {location.hash = '#/history';});
      await expect(page.getByTestId('history-row')).toHaveAttribute('data-status', state);
      const action = page.getByTestId('history-resume');
      await expect(action).toHaveText(['working', 'running', 'idle'].includes(state) ? 'Open in Otto' : 'Resume in Otto');
      await action.click();
      await expect(page).toHaveURL(new RegExp(`#/agents/${sessionId}$`));
      await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
      const actual = await (await ctx.get(`${base}/api/v1/sessions/${sessionId}`)).json();
      expect(actual.live).toBe(true);
    });
  }
  expect(calls).toEqual(Array(5).fill('resume'));
});

test('History stale live row resumes an isolated shell that exited after listing', async ({page}) => {
  await historyFixture(page, () => historyRow('working'));
  await page.goto('/#/history');
  await expect(page.getByTestId('history-row')).toHaveAttribute('data-status', 'working');
  // This is the throwaway daemon's fixture session, never the desktop daemon.
  expect((await ctx.post(`${base}/api/v1/sessions/${sessionId}/kill`)).ok()).toBe(true);
  await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${sessionId}`)).json()).live).toBe(false);
  await page.getByTestId('history-resume').click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${sessionId}$`));
  await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
  expect((await (await ctx.get(`${base}/api/v1/sessions/${sessionId}`)).json()).live).toBe(true);
});

test('History resume completion preserves navigation made while the response was pending', async ({page}) => {
  await historyFixture(page, () => historyRow('working'));
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let requested = false;
  await page.route(`**/sessions/${sessionId}/resume`, async route => {
    const response = await route.fetch();
    requested = true;
    await held;
    await route.fulfill({response});
  });
  try {
    await page.goto('/#/history');
    await page.getByTestId('history-resume').click();
    await expect.poll(() => requested).toBe(true);
    // Stay in the same document: a full navigation would abort the request
    // and conceal a destroyed component's late follow-up navigation.
    await page.evaluate(() => { location.hash = '#/settings/insights'; });
    await expect(page).toHaveURL(/#\/settings\/insights$/);
    await expect(page.getByTestId('history-resume')).toHaveCount(0);
    const completed = page.waitForResponse(response => response.url().endsWith(`/sessions/${sessionId}/resume`));
    release();
    await (await completed).finished();
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(page).toHaveURL(/#\/settings\/insights$/);
    expect((await (await ctx.get(`${base}/api/v1/sessions/${sessionId}`)).json()).live).toBe(true);
  } finally { release(); }
});

test('History resume preserves an accepted destination while its page readiness is pending', async ({page}) => {
  await historyFixture(page, () => historyRow('working'));
  let release!: () => void;
  const held = new Promise<void>(resolve => {release = resolve;});
  let requested = false;
  await page.route(`**/sessions/${sessionId}/resume`, async route => {
    const response = await route.fetch(); requested = true;
    await held; await route.fulfill({response});
  });
  try {
    await page.goto('/#/history');
    await page.getByTestId('history-resume').click();
    await expect.poll(() => requested).toBe(true);
    // Control only the router's public page-readiness hook. The real router
    // accepts the hash but keeps History mounted until this promise settles.
    await page.evaluate(async () => {
      const path = '/src/lib/router.svelte.ts';
      const {router} = await import(path);
      const host = window as unknown as {releaseReviewRoute?: () => void};
      const ready = new Promise<void>(resolve => {host.releaseReviewRoute = resolve;});
      router.setPrepare((parts: readonly string[]) => parts[0] === 'settings' ? ready : null);
      router.go('settings/insights');
    });
    await expect.poll(() => page.evaluate(async () => {
      const path = '/src/lib/router.svelte.ts';
      return (await import(path)).router.pendingTarget?.[0];
    })).toBe('settings');
    await expect(page.getByTestId('history-resume')).toBeVisible();
    const completed = page.waitForResponse(response => response.url().endsWith(`/sessions/${sessionId}/resume`));
    release(); await (await completed).finished();
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(page).toHaveURL(/#\/settings\/insights$/);
  } finally {
    release();
    await page.evaluate(() => (window as unknown as {releaseReviewRoute?: () => void}).releaseReviewRoute?.()).catch(() => {});
  }
});

test('History import keeps a failed resume on screen and retries without importing twice', async ({page}) => {
  await historyFixture(page, () => historyRow('on_disk'));
  const calls: string[] = [];
  const imported = await (await ctx.get(`${base}/api/v1/sessions/${sessionId}`)).json();
  await page.route('**/api/v1/workspaces/*/history/import', route => {
    calls.push('import');
    expect(route.request().postDataJSON()).toEqual({provider: 'claude', transcript_path: '/tmp/synthetic-recovery.jsonl'});
    // Imported identity points at our shell fixture; no provider CLI is used.
    return route.fulfill({json: imported});
  });
  let failResume = true;
  await page.route(`**/sessions/${sessionId}/resume`, route => {
    calls.push('resume');
    return failResume ? route.fulfill({status: 409, json: {code: 'conflict', message: 'Synthetic resume unavailable'}}) : route.continue();
  });
  await page.goto('/#/history');
  await page.getByTestId('history-resume').click();
  await expect(page.getByText('Couldn’t resume', {exact: true})).toBeVisible();
  await expect(page).toHaveURL(/#\/history/);
  await expect(page.getByTestId('history-resume')).toBeEnabled();
  expect(calls).toEqual(['import', 'resume']);
  failResume = false;
  await page.getByTestId('history-resume').click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${sessionId}$`));
  await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
  expect(calls).toEqual(['import', 'resume', 'resume']);
});


test('child history allocation recovers after 100 mounted inspections and 1200-turn paging', async ({page, browser}, info) => {
  test.setTimeout(180_000);
  let childReads = 0;
  await page.route(`**/sessions/${sessionId}/transcript?*`, route => {
    const params = new URL(route.request().url()).searchParams;
    if (!params.has('sub')) {
      const parent = turn(1);
      parent.blocks = [{kind: 'subagent', agent_id: 'child', description: 'Measured child', agent_type: 'reviewer', status: 'done'}];
      return route.fulfill({json: transcriptPage([parent])});
    }
    childReads++;
    const end = Number(params.get('before') ?? 1200), start = Math.max(0, end - 60);
    const turns = Array.from({length: end - start}, (_, i) => {
      const row = turn(start + i);
      row.blocks = [{kind: 'text', md: `Child checkpoint ${start + i}. ` + 'Bounded child content. '.repeat(100)}];
      return row;
    });
    const response = transcriptPage(turns, String(start), start > 0);
    response.stats.turns = 1200;
    return route.fulfill({json: response});
  });
  await openChat(page);
  const steps = page.locator('.conv .steps-head');
  if (await steps.getAttribute('aria-expanded') === 'false') await steps.click();
  const card = page.locator('.sub[data-agent="child"]'), toggle = card.locator('.sub-head');
  const inspect = async () => {
    await toggle.click(); await expect(card.locator('.sub-body .turn')).toHaveCount(60);
    await toggle.click(); await expect(card.locator('.sub-body')).toHaveCount(0);
  };
  // Warm the actual render/module caches before comparing retained allocations.
  for (let i = 0; i < 10; i++) await inspect();
  const cdp = await browser.newBrowserCDPSession();
  const samples: unknown[] = [];
  const sample = async (phase: string) => {
    const heapBytes = await heapAfterGC(page);
    const {processInfo} = await cdp.send('SystemInfo.getProcessInfo');
    const ids = processInfo.map(p => Number(p.id)).filter(Number.isSafeInteger);
    const rss = execFileSync('ps', ['-p', ids.join(','), '-o', 'pid=,rss='], {encoding: 'utf8'});
    const rssByPid = Object.fromEntries(rss.trim().split('\n').map(row => row.trim().split(/\s+/).map(Number)));
    const retained = await page.evaluate(async id => {
      const path = '/src/lib/stores/transcript.svelte.ts';
      const {transcript} = await import(path);
      return {bodies: Object.keys(transcript.peek(id)?.subagents ?? {}).length, domNodes: document.querySelectorAll('*').length};
    }, sessionId);
    samples.push({phase, wallMs: Date.now(), heapBytes, ...retained,
      processes: processInfo.map(p => ({type: p.type, pid: p.id, cpuSeconds: p.cpuTime, rssKiB: rssByPid[p.id] ?? null}))});
    expect(retained.bodies).toBe(0);
  };
  try {
    await watchLongTasks(page);
    await sample('warm');
    const timings: number[] = [];
    for (let i = 0; i < 100; i++) {
      const start = performance.now(); await inspect(); timings.push(performance.now() - start);
      if ((i + 1) % 20 === 0) await sample(`inspection-${i + 1}`);
    }
    await toggle.click();
    await expect(card).toContainText('Child checkpoint 1199.');
    for (let pageIndex = 0; pageIndex < 19; pageIndex++) {
      await card.getByRole('button', {name: 'Load earlier', exact: true}).click();
      await expect(card).toContainText(`Child checkpoint ${1140 - (pageIndex + 1) * 60}.`);
      await expect(card.locator('.sub-body .turn')).toHaveCount(60);
    }
    await expect(card.getByRole('button', {name: 'Load earlier', exact: true})).toHaveCount(0);
    for (let pageIndex = 0; pageIndex < 19; pageIndex++) {
      await card.getByRole('button', {name: 'Load newer', exact: true}).click();
      await expect(card).toContainText(`Child checkpoint ${(pageIndex + 1) * 60}.`);
      await expect(card.locator('.sub-body .turn')).toHaveCount(60);
    }
    await toggle.click(); await expect(card.locator('.sub-body')).toHaveCount(0);
    await sample('paging-recovered');
    for (let i = 0; i < 100; i++) await inspect();
    await sample('repeat-100-recovered');
    const report = {scope: 'One mounted parent; 1200 synthetic child turns; browser processes only. Initial100 inspection timings; an additional100 inspections check recovery after paging. ForcedGC between checkpoints. Measured timings/RSS are observations, not a native-app guarantee.', childReads, inspectionMs: dist(timings), longTaskMs: dist(await longTasks(page)), samples};
    await info.attach('child-allocation-measurements', {body: JSON.stringify(report, null, 2), contentType: 'application/json'});
  } finally { await cdp.detach(); }
});
