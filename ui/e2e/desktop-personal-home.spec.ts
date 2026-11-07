import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The Personal Agents home: the agent list (main agent first, with a star,
// a status line each), one continuous conversation per agent (an intro and a
// ready composer before the first message), the routines-and-runs side panel,
// the settings page, and — in Threads — personal-agent threads grouped under
// the agent's NAME with what started each one.
// Everything agent-side is route-mocked: no agent CLI runs.

test.use({ serviceWorkers: 'block' });

const MAIN = '01MAINAGENT0000000000000000';
const SCOUT = '01M1RX7QZDVJ236Y7MZKAD9XYZ';

/** Switch the app's color scheme the way Settings does (stored, reloaded). */
async function setScheme(page: Page, scheme: 'light' | 'dark'): Promise<void> {
  await page.evaluate((s) => localStorage.setItem('otto_scheme', s), scheme);
  await page.reload();
}

async function setup(page: Page): Promise<string> {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.removeItem('otto.pa.panel');
  }, ws);
  const agent = (id: string, name: string, extra: Record<string, unknown> = {}) => ({
    id, name, workspace_id: ws, avatar: '', soul_md: '', provider: 'claude', model: '', cwd: '', browser: false,
    delivery: { type: 'none' }, enabled: true, chat_session_id: null, created_by: null, created_by_agent: null,
    created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z', ...extra,
  });
  const agents = [
    agent(SCOUT, 'Scout', { avatar: 'char:nerd', soul_md: 'Watches the build and digs into failures.', created_by_agent: MAIN }),
    agent(MAIN, 'Otto', { avatar: 'char:chief', soul_md: '<!-- otto-template:main-agent -->\nI’m your main agent: your one point of contact.' }),
  ];
  const schedule = { id: 'sch-1', agent_id: SCOUT, schedule: { cadence: 'cron', expr: '0 9 * * 0,1,2,3,4' }, timezone: 'Asia/Jerusalem', directive: 'Morning brief\nSummarise failing builds', enabled: true, permission: 'directed', last_run_at: null, next_run_at: '2026-11-08T07:00:00Z', created_at: '', updated_at: '' };
  await page.route(`**/api/v1/workspaces/${ws}/personal-agents`, (r) => r.fulfill({ json: agents }));
  await page.route('**/api/v1/personal-agents/*/autonomy', (r) => {
    const main = r.request().url().includes(MAIN);
    return r.fulfill({ json: { primary: main, proactive: { enabled: false, runs_per_day: 4, max_minutes: 10 }, goals: [], rules: [], personality: { preset: main ? '' : 'nerdy', note: '' } } });
  });
  await page.route('**/api/v1/personal-agents/*/schedules', (r) => r.fulfill({ json: r.request().url().includes(SCOUT) ? [schedule] : [] }));
  await page.route('**/api/v1/personal-agents/*/routines', (r) => r.fulfill({ json: {
    agent: { id: SCOUT, name: 'Scout', enabled: true }, user_timezone: 'Asia/Jerusalem',
    routines: r.request().url().includes(SCOUT) ? [{ id: 'sch-1', agent_id: SCOUT, agent_name: 'Scout', name: 'Morning brief', instruction: schedule.directive, schedule: schedule.schedule, timezone: 'Asia/Jerusalem', enabled: true, paused: false, agent_paused: false, permission: 'directed', deliver: { type: 'chat' }, last_run_at: null, next_runs: ['2026-11-08T07:00:00Z', '2026-11-09T07:00:00Z', '2026-11-10T07:00:00Z'] }] : [],
  } }));
  await page.route('**/api/v1/personal-agents/*/runs', (r) => r.fulfill({ json: r.request().url().includes(SCOUT) ? [{
    id: 'run-1', agent_id: SCOUT, schedule_id: 'sch-1', workspace_id: ws, status: 'ok', trigger: 'schedule', mode: 'scheduled', started_at: '2026-11-05T07:00:00Z', finished_at: '2026-11-05T07:03:00Z',
    summary: 'Two builds failed on main', report_path: '/x', report_rel: `${SCOUT}/reports/r.md`, delivered: false, delivery_error: null, error: null, session_id: 'thr-run', report_hash: null, attempts: 1, skipped_delivery: true, goal_id: null, read_only: false, created_at: '2026-11-05T07:00:00Z',
  }] : [] }));
  await page.route('**/api/v1/personal-agents/*/delegations', (r) => r.fulfill({ json: [{
    id: 'thr-del', workspace_id: ws, lead_agent_id: MAIN, lead_thread_id: 'thr-chat', member_agent_id: SCOUT, member_name: 'Scout', task: 'Why did the nightly build fail?', status: 'running', summary: '', created_at: '2026-11-05T08:00:00Z', finished_at: null, delivered_at: null,
  }] }));
  await page.route('**/api/v1/personal-agents/*/instructions-proposal', (r) => r.fulfill({ json: { proposal: null, can_undo: false } }));
  await page.route('**/api/v1/personal-agents/*/memories', (r) => r.fulfill({ json: { version: 'v', items: [] } }));
  await page.route('**/api/v1/personal-agents/*/slack-as-me', (r) => r.fulfill({ json: { teams: [], available: [] } }));
  const dir = (id: string) => `/tmp/otto-e2e/personal/${id}`;
  const thread = (id: string, title: string, cwd: string, meta: Record<string, unknown>, status = 'idle', parent: string | null = null) => ({
    id, workspace_id: ws, provider: 'claude', title, cwd, status, access_mode: 'approval-required', created_by: 'root',
    created_at: '2026-11-05T07:00:00Z', last_active_at: '2026-11-05T08:00:00Z', last_sequence: 1, parent_id: parent, meta,
  });
  await page.route(`**/api/v1/workspaces/${ws}/runtime-sessions`, (r) => r.fulfill({ json: [
    thread('thr-chat', 'Chat: Otto', dir(MAIN), { source: 'personal_agent', personal_agent: MAIN, personal_agent_chat: true }),
    thread('thr-run', 'Agent: Scout', dir(SCOUT), { source: 'personal_agent', personal_agent: SCOUT, personal_agent_name: 'Scout', run_id: 'run-1', trigger: 'schedule', routine: 'Morning brief', agent_mode: 'scheduled' }),
    thread('thr-del', 'Agent: Scout', dir(SCOUT), { source: 'personal_agent', personal_agent: SCOUT, personal_agent_name: 'Scout', delegated_by_agent: MAIN, delegated_by_name: 'Otto' }, 'working', 'thr-chat'),
  ] }));
  return ws;
}

test('Focus keeps one conversation with agent switching and contextual sheets — light and dark', async ({ page }, info) => {
  await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/personal-agents');
  const picker = page.getByTestId('pa-agent-switcher');
  await expect(picker).toContainText('Otto');
  await expect(page.getByTestId('pa-rail')).toHaveCount(0);
  await expect(page.getByTestId('pa-side-panel')).toHaveCount(0);
  await expect(page.locator('h1')).toHaveCount(1);
  await page.getByRole('button', { name: 'What are my agents up to?' }).click();
  await expect(page.getByTestId('pa-composer')).toHaveValue('What are my agents up to?');
  await expect(page.getByTestId('pa-handoffs')).toContainText('Scout');
  await picker.click();
  await expect(page.getByRole('menuitem').first()).toContainText('Otto');
  await expectFullyInViewport(page, page.getByRole('menu'));
  await page.getByRole('menuitem', { name: 'Scout · Working' }).click();
  await expect(page).toHaveURL(new RegExp(`#/personal-agents/${SCOUT}$`));
  await expect(page.getByTestId('pa-composer')).toHaveAttribute('aria-label', 'Message Scout');
  await expect(page.getByTestId('pa-latest-result')).toContainText('Two builds failed on main');
  await picker.click();
  await page.getByRole('menuitem', { name: /Otto · Main agent/ }).click();
  await expect(page.getByTestId('pa-composer')).toHaveAttribute('aria-label', 'Message Otto');
  await expect(page.getByTestId('pa-composer')).toHaveValue('What are my agents up to?');
  await picker.click();
  await page.getByRole('menuitem', { name: 'Scout · Working' }).click();
  await expect(page.getByTestId('pa-composer')).toHaveAttribute('aria-label', 'Message Scout');
  await expect(page.getByTestId('pa-composer')).toHaveValue('');
  await page.getByTestId('pa-composer').fill('Keep this draft while I look around');
  await page.getByTestId('pa-composer').evaluate((el) => el.setAttribute('data-continuity-probe', 'original'));

  await page.getByRole('button', { name: 'Routines', exact: true }).click();
  const routines = page.getByRole('dialog', { name: 'Routines · Scout', exact: true });
  await expect(routines.getByTestId('pa-routines')).toContainText('Morning brief');
  await expect(routines.getByTestId('pa-routines')).toContainText('next ');
  await expect(routines.getByTestId('pa-activity')).toContainText('Working…');
  await expectFullyInViewport(page, routines);
  await page.keyboard.press('Escape');
  await expect(routines).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Routines', exact: true })).toBeFocused();
  await expect(page.getByTestId('pa-composer')).toHaveAttribute('data-continuity-probe', 'original');
  await expect(page.getByTestId('pa-composer')).toHaveValue('Keep this draft while I look around');

  await page.getByRole('button', { name: 'History', exact: true }).click();
  const history = page.getByRole('dialog', { name: 'History · Scout', exact: true });
  await expect(history).toContainText('Two builds failed on main');
  await expect(history.getByRole('tab', { name: 'Runs', exact: true })).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Settings for Scout' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings · Scout', exact: true });
  await expect(settings.getByRole('tab', { name: 'About', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(settings.getByTestId('pa-personality').getByLabel('Tone')).toHaveValue('nerdy');
  await settings.getByRole('tab', { name: 'Memory', exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`#/personal-agents/${SCOUT}/memory$`));
  await expect(page.getByTestId('pa-conversation')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('pa-composer')).toHaveAttribute('data-continuity-probe', 'original');
  await expect(page.getByTestId('pa-composer')).toHaveValue('Keep this draft while I look around');

  for (const scheme of ['light', 'dark'] as const) {
    await setScheme(page, scheme);
    await expect(page.getByTestId('pa-composer')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`pa-focus-${scheme}.png`) });
    await page.getByRole('button', { name: 'Settings for Scout' }).click();
    await expect(settings.getByTestId('pa-personality')).toBeVisible();
    await page.screenshot({ path: info.outputPath(`pa-focus-settings-${scheme}.png`) });
    await page.keyboard.press('Escape');
  }

  await picker.click();
  await page.getByRole('menuitem', { name: 'New agent…' }).click();
  const create = page.getByRole('dialog', { name: 'New agent', exact: true });
  await expect(create.getByLabel('Name', { exact: true })).toBeVisible();
  await expect(create.getByLabel('What it does')).toBeVisible();
  await expect(create.getByLabel('Personality')).toBeHidden();
  await create.locator('summary').click();
  await expect(create.getByLabel('Personality')).toBeVisible();
  await page.keyboard.press('Escape');

  for (const width of [1024, 390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    await expect(page.getByRole('button', { name: 'Routines', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'History', exact: true })).toBeVisible();
    await expectFullyInViewport(page, picker);
    await expectNoHorizontalOverflow(page);
    await page.getByRole('button', { name: 'Routines', exact: true }).click();
    await expectFullyInViewport(page, routines);
    await page.keyboard.press('Escape');
    await expect(routines).toHaveCount(0);
    await page.screenshot({ path: info.outputPath(`pa-focus-${width}.png`) });
  }
});

test('phone Focus keeps settings, memory, history and nested routine forms usable', async ({ page }, info) => {
  await setup(page);
  for (const [width, height] of [[390, 844], [844, 390]]) {
    await page.setViewportSize({ width, height });
    await page.goto(`/#/personal-agents/${SCOUT}`);
    await page.getByTestId('pa-composer').fill('Keep my phone draft');
    await page.getByRole('button', { name: 'More for Scout', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Settings', exact: true }).click();
    const settings = page.getByRole('dialog').filter({ has: page.getByRole('tablist', { name: 'Agent sections', exact: true }) });
    for (const name of ['Memory', 'Runs', 'About']) {
      await settings.getByRole('tab', { name, exact: true }).click();
      await expect(settings.getByRole('tab', { name, exact: true })).toHaveAttribute('aria-selected', 'true');
      await expectFullyInViewport(page, settings);
      await expectNoHorizontalOverflow(page);
    }
    await page.screenshot({ path: info.outputPath(`pa-phone-settings-${width}.png`) });
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Routines', exact: true }).click();
    await page.getByRole('button', { name: 'Add routine', exact: true }).click();
    const form = page.getByRole('dialog', { name: 'New routine for Scout', exact: true });
    await expectFullyInViewport(page, form);
    await form.getByLabel('What should it do?').fill('Send me a daily summary');
    await expect(form.getByLabel('Timezone', { exact: true })).toBeAttached();
    await form.getByRole('button', { name: 'Add routine', exact: true }).scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, form.getByRole('button', { name: 'Add routine', exact: true }));
    await page.screenshot({ path: info.outputPath(`pa-phone-routine-${width}.png`) });
    await form.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Routines · Scout', exact: true })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('pa-composer')).toHaveValue('Keep my phone draft');
  }
});

test('a paused agent says its routines won’t run, with Resume right there', async ({ page }) => {
  await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/#/personal-agents/${SCOUT}`);
  await expect(page.getByTestId('pa-conversation')).toBeVisible();
  await page.evaluate(async (id) => {
    const path = '/src/lib/stores/personalAgents.svelte.ts';
    const { personalAgents } = await import(/* @vite-ignore */ path);
    personalAgents.agents = personalAgents.agents.map((a: { id: string }) => (a.id === id ? { ...a, enabled: false } : a));
  }, SCOUT);
  await expect(page.getByTestId('pa-paused-banner')).toContainText('its routines won’t run');
  await expect(page.getByTestId('pa-paused-banner').getByRole('button', { name: 'Resume' })).toBeVisible();
  await page.getByRole('button', { name: 'Routines', exact: true }).click();
  await expect(page.getByTestId('pa-side-panel').getByTestId('pa-routines')).toContainText('Won’t run — the agent is paused');
});

test('no chat yet: a running routine replaces the intro, and its first result opens the chat live', async ({ page }) => {
  await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/#/personal-agents/${SCOUT}`);
  await expect(page.getByTestId('pa-intro')).toBeVisible();
  const store = async (fn: string) =>
    page.evaluate(async ([id, body]) => {
      const path = '/src/lib/stores/personalAgents.svelte.ts';
      const { personalAgents } = await import(/* @vite-ignore */ path);
      new Function('pa', 'id', body)(personalAgents, id);
    }, [SCOUT, fn] as const);
  // The run is on: a calm "Running …" in the body, no starter chips.
  await store('pa.runningByAgent = { ...pa.runningByAgent, [id]: true };');
  await expect(page.getByTestId('pa-run-wait')).toContainText('Running');
  await expect(page.getByRole('group', { name: 'Try asking' })).toHaveCount(0);
  // The run pinned the agent's chat (daemon: personal_agent_changed → refetch): no reload needed.
  await store('pa.runningByAgent = { ...pa.runningByAgent, [id]: false }; pa.agents = pa.agents.map((a) => (a.id === id ? { ...a, chat_session_id: "thr-run" } : a));');
  await expect(page.getByTestId('pa-intro')).toHaveCount(0);
});

test('Threads groups agent threads by NAME and badges what started each', async ({ page }, info) => {
  await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/threads');
  const sidebar = page.getByTestId('threads-sidebar');
  // The group header is the agent's name — never its folder's ULID.
  await expect(sidebar.getByRole('region', { name: 'Project Scout' }).or(sidebar.locator('section[aria-label="Project Scout"]'))).toBeVisible();
  await expect(sidebar).not.toContainText(SCOUT);
  await expect(sidebar.locator('section[aria-label="Project Otto"]')).toBeVisible();
  await expect(page.getByTestId('thread-badge-thr-chat')).toHaveText('Chat');
  await expect(page.getByTestId('thread-badge-thr-run')).toHaveText('Scheduled · Morning brief');
  await expect(page.getByTestId('thread-badge-thr-del')).toHaveText('Delegated by Otto');
  // The breadcrumb names the agent too.
  await expect(page.locator('body')).not.toContainText('01MAINAGENT');
  for (const scheme of ['light', 'dark'] as const) {
    await setScheme(page, scheme);
    await expect(page.getByTestId('thread-badge-thr-chat')).toHaveText('Chat');
    await page.screenshot({ path: info.outputPath(`threads-agent-groups-${scheme}.png`) });
  }
});

test('phone: agent switcher and conversation fit without a permanent rail', async ({ page }, info) => {
  await setup(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`/#/personal-agents/${SCOUT}`);
  await expect(page.getByTestId('pa-agent-switcher')).toBeVisible();
  await expect(page.getByTestId('pa-rail')).toHaveCount(0);
  await expect(page.getByTestId('pa-conversation')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('pa-home-phone.png') });
});

test('settings keyboard tabs retain selected focus when unsaved navigation is declined', async ({ page }) => {
  await setup(page);
  await page.route('**/api/v1/personal-agents/*/context', (route) => route.fulfill({ json: {
    content: 'Saved context', version: 'v1', path: '/tmp/context.md', exists: true,
  } }));
  await page.goto(`/#/personal-agents/${SCOUT}/memory`);
  const sheet = page.getByRole('dialog', { name: 'Settings · Scout', exact: true });
  await sheet.getByTestId('agent-document').getByRole('button', { name: 'Edit context', exact: true }).click();
  await sheet.getByRole('textbox', { name: 'Context Markdown' }).fill('Unsaved context');
  const memory = sheet.getByRole('tab', { name: 'Memory', exact: true });
  await memory.focus();
  await memory.press('ArrowLeft');
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/${SCOUT}/memory$`));
  await expect(memory).toBeFocused();
  await expect(memory).toHaveAttribute('aria-selected', 'true');
  await expect(sheet.getByRole('textbox', { name: 'Context Markdown' })).toHaveValue('Unsaved context');
  await memory.press('ArrowLeft');
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/${SCOUT}/activity$`));
  await expect(page.getByRole('tab', { name: 'Runs', exact: true })).toBeFocused();
});
