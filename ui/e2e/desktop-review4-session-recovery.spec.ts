import {test, expect, type APIRequestContext, type Page} from '@playwright/test';
import type {Transcript, Turn} from '../src/lib/api/types';
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
  // The batch-result region (the dialog also keeps a session-count status).
  await expect(dialog.getByRole('status').filter({ hasText: /started;/ })).toContainText('1 started; 1 still need to start');
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
