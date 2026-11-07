import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

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

test('the home: agent list, ready composer, routines-and-runs panel, settings — light and dark', async ({ page }, info) => {
  await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/personal-agents');
  const rail = page.getByTestId('pa-rail');
  // The main agent leads the list, starred; the specialist shows it is working.
  await expect(rail.getByTestId(`pa-rail-${MAIN}`)).toBeVisible();
  const rows = rail.locator('[data-testid^="pa-rail-0"]');
  await expect(rows.first()).toHaveAttribute('data-testid', `pa-rail-${MAIN}`);
  await expect(rail.getByTestId(`pa-rail-${SCOUT}`)).toHaveAttribute('data-presence', 'working');
  await expect(rail.getByTestId(`pa-rail-${SCOUT}`)).toContainText('Working…');
  // Never chatted: the intro and a ready composer; a chip only prefills it
  // (no tokens are spent until you send).
  await expect(page.getByTestId('pa-intro')).toContainText('Task me with anything');
  await page.getByRole('button', { name: 'What are my agents up to?' }).click();
  await expect(page.getByTestId('pa-composer')).toHaveValue('What are my agents up to?');
  // The main agent's hand-offs are live chips above the chat.
  await expect(page.getByTestId('pa-handoffs')).toContainText('Scout');
  await expect(page.getByTestId('pa-handoffs')).toContainText('working…');
  await expectNoHorizontalOverflow(page);

  // The specialist: its routine in plain words with the next run, what it is
  // doing now, and its recent runs — one panel, no tabs.
  await rail.getByTestId(`pa-rail-${SCOUT}`).click();
  await expect(page).toHaveURL(new RegExp(`#/personal-agents/${SCOUT}$`));
  const panel = page.getByTestId('pa-side-panel');
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId('pa-routines')).toContainText('Morning brief');
  await expect(panel.getByTestId('pa-routines')).toContainText('next ');
  await expect(panel.getByTestId('pa-activity')).toContainText('Working…');
  await expect(panel.getByTestId('pa-recent')).toContainText('Two builds failed on main');
  await expect(panel.getByTestId('pa-recent')).toContainText('Scheduled · Morning brief');
  for (const scheme of ['light', 'dark'] as const) {
    await setScheme(page, scheme);
    await expect(panel.getByTestId('pa-routines')).toContainText('Morning brief');
    await page.screenshot({ path: info.outputPath(`pa-home-${scheme}.png`) });
  }
  await setScheme(page, 'light');

  // The panel toggles from the header; Settings opens on About.
  await page.getByTestId('pa-panel-toggle').click();
  await expect(panel).toHaveCount(0);
  await page.getByRole('button', { name: 'More for Scout' }).click();
  await page.getByRole('menuitem', { name: 'Settings' }).click();
  await expect(page.getByRole('tab', { name: 'About', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('pa-personality').getByLabel('Tone')).toHaveValue('nerdy');
  await expect(page.getByTestId('pa-personality')).toContainText('How Scout would answer');
  for (const scheme of ['light', 'dark'] as const) {
    await setScheme(page, scheme);
    await expect(page.getByTestId('pa-personality')).toBeVisible();
    await page.screenshot({ path: info.outputPath(`pa-settings-${scheme}.png`) });
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
  await expect(page.getByTestId('pa-side-panel').getByTestId('pa-routines')).toContainText('Won’t run — the agent is paused');
  await expect(page.getByTestId(`pa-rail-${SCOUT}`)).toContainText('Paused');
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

test('phone: the rail becomes a top strip and nothing scrolls sideways', async ({ page }, info) => {
  await setup(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`/#/personal-agents/${SCOUT}`);
  await expect(page.getByTestId('pa-rail')).toBeVisible();
  await expect(page.getByTestId('pa-conversation')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('pa-home-phone.png') });
});
