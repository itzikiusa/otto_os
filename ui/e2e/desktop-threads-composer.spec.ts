import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads composer and timeline on REAL fake-agent threads: Codex runs
// otto-fake-codex (`codex app-server`), Claude otto-fake-claude (stream-json);
// both answer `slow <s>` with "Working…" and take a steer meanwhile
// ("steered: <text>"). Needs OTTO_E2E_BIN on a build with the fakes beside it.
//   • ONE primary button that says what it will do: Send idle, Stop while the
//     agent works with nothing typed, Queue (default) / Steer with a draft;
//     ⌘↵ the other one; the choice in the toolbar ⋯ › View (composerSend.ts).
//   • The / popup lists the agent's own commands (hostCommands.ts).
//   • ↑ / ↓ recall sent prompts (promptHistory.ts).
//   • Files dropped on the pane attach like a paste (composerDrop.ts).
//   • A terminal selection → "Add to chat" context chip (TerminalAddToChat).
//   • ⋯ › View › Tool calls: show all / hide successful / hide all (workLog.ts).
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
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-composer-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Threads composer\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Composer WS', root_path: fixtureDir } });
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

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_prefs_v1');
  }, wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function open(page: Page, provider: 'codex' | 'claude' | 'gemini', title: string): Promise<{ id: string; thread: Locator }> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: fixtureDir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  return { id, thread };
}

async function send(thread: Locator, text: string, reply = `echo: ${text}`): Promise<void> {
  await thread.getByTestId('runtime-composer').fill(text);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText(reply, { exact: false }).last()).toBeVisible({ timeout: 20_000 });
}

/** Start a turn that runs ~`secs` and wait until the agent is working. */
async function startSlow(thread: Locator, secs: number): Promise<void> {
  await thread.getByTestId('runtime-composer').fill(`slow ${secs}`);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText('Working…').last()).toBeVisible({ timeout: 20_000 });
}

async function pickView(page: Page, name: string): Promise<void> {
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).click();
  await page.locator('.ctx-submenu').getByRole('menuitemcheckbox', { name }).click();
}

for (const provider of ['codex', 'claude'] as const) {
  test(`${provider}: one primary button — Stop, then Queue or Steer with a draft; ⌘↵ the other; the View choice`, async ({ page }) => {
    test.setTimeout(150_000);
    const { thread } = await open(page, provider, `Primary ${provider}`);
    const composer = thread.getByTestId('runtime-composer');
    const primary = thread.getByTestId('runtime-send');
    const stop = thread.getByTestId('runtime-stop');
    await expect(primary).toHaveAttribute('data-delivery', 'send');
    await expect(stop).toHaveCount(0);

    // Working with nothing typed: Stop is the one button.
    await startSlow(thread, 12);
    await expect(stop).toBeVisible();
    await expect(primary).toHaveCount(0);
    // A draft: the primary names the default delivery, Stop steps back.
    await composer.fill('queued one');
    await expect(primary).toHaveAttribute('data-delivery', 'queue');
    await expect(primary).toHaveAttribute('aria-label', 'Queue — sends when this turn ends');
    await expect(primary).toHaveAttribute('title', /⌘↵ steers/);
    await expect(stop).toBeVisible();
    const [p, s] = [await primary.boundingBox(), await stop.boundingBox()];
    expect(s!.width).toBeLessThan(p!.width);
    await composer.press('Enter');
    await expect(thread.getByTestId('runtime-queued')).toHaveCount(1);
    // ⌘↵ steers into the running turn (the queue bar's path).
    await composer.fill('also check b');
    await composer.press('Meta+Enter');
    await expect(thread.getByText('steered: also check b').last()).toBeVisible({ timeout: 20_000 });
    await expect(thread.getByTestId('runtime-queued')).toHaveCount(1);

    // ⋯ › View: Enter steers from now on (per device), ⌘↵ queues.
    await pickView(page, 'While working, messages steer');
    await composer.fill('now steer');
    await expect(primary).toHaveAttribute('data-delivery', 'steer');
    await expect(primary).toHaveAttribute('title', /⌘↵ queues/);
    await expect(composer).toHaveAttribute('placeholder', /./);
    await composer.press('Enter');
    await expect(thread.getByText('steered: now steer').last()).toBeVisible({ timeout: 20_000 });
    await composer.fill('then queue');
    await composer.press('Meta+Enter');
    await expect(thread.getByTestId('runtime-queued')).toHaveCount(2);
    expect(await page.evaluate(() => localStorage.getItem('otto_threads_prefs_v1'))).toContain('steer');
    // Stop from the small control ends the turn; the queue then drains.
    await stop.click();
    await expect(thread.getByText('echo: queued one').last()).toBeVisible({ timeout: 30_000 });
    await expect(thread.getByText('echo: then queue').last()).toBeVisible({ timeout: 30_000 });
    await expect(primary).toHaveAttribute('data-delivery', 'send');
    await expectNoHorizontalOverflow(page);
  });
}

test('claude: the / popup lists the agent’s own commands under its heading; choosing one types it for the agent', async ({ page }) => {
  test.setTimeout(90_000);
  const { thread } = await open(page, 'claude', 'Slash claude');
  await send(thread, 'hello'); // the agent is up and has published its commands
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('/');
  const pop = thread.locator('[data-slash-pop]');
  await expect(pop.getByTestId('slash-group')).toHaveText(['Otto', 'Claude commands']);
  await expect(pop.locator('[data-source="agent"]')).toHaveText([/\/compact/]); // /review is Otto's here, so the agent's is left out
  await expect(pop.getByRole('option', { name: /\/review/ })).toHaveCount(1);
  await expectFullyInViewport(page, pop, 'slash popup');
  await composer.pressSequentially('comp');
  await expect(pop.getByTestId('slash-group')).toHaveText(['Claude commands']);
  await composer.press('Enter');
  await expect(composer).toHaveValue('/compact ');
  await composer.press('Enter');
  await expect(thread.getByText('echo: /compact').last()).toBeVisible({ timeout: 20_000 });
});

test('codex: its skills are its commands; one Otto takes itself is not offered twice', async ({ page }) => {
  test.setTimeout(90_000);
  const { thread } = await open(page, 'codex', 'Slash codex');
  await send(thread, 'hello');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('/re');
  const pop = thread.locator('[data-slash-pop]');
  await expect(pop.getByRole('option', { name: /\/review/ })).toHaveCount(1);
  await expect(pop.getByTestId('slash-group')).toHaveCount(0);
});

test('↑ in an empty composer recalls sent prompts, ↓ goes back to the empty draft; an edit leaves history', async ({ page }) => {
  test.setTimeout(90_000);
  for (const provider of ['codex', 'claude'] as const) {
    const { thread } = await open(page, provider, `History ${provider}`);
    await send(thread, 'first prompt');
    await send(thread, 'second prompt');
    const composer = thread.getByTestId('runtime-composer');
    await composer.focus();
    await composer.press('ArrowUp');
    await expect(composer).toHaveValue('second prompt');
    await composer.press('ArrowUp');
    await expect(composer).toHaveValue('first prompt');
    await composer.press('ArrowDown');
    await expect(composer).toHaveValue('second prompt');
    await composer.press('ArrowDown');
    await expect(composer).toHaveValue('');
    // A draft of one's own keeps ↑ for the caret.
    await composer.fill('my draft');
    await composer.press('ArrowUp');
    await expect(composer).toHaveValue('my draft');
    // Recalled, then edited: ↑ is the caret's again.
    await composer.fill('');
    await composer.press('ArrowUp');
    await composer.press('End');
    await composer.pressSequentially('!');
    await composer.press('ArrowUp');
    await expect(composer).toHaveValue('second prompt!');
    // The / popup keeps its own ↑.
    await composer.fill('/');
    await composer.press('ArrowUp');
    await expect(composer).toHaveValue('/');
  }
});

/** A drag of `files` (name, type, bytes) entering and dropping on `target`. */
async function dropFiles(target: Locator, files: { name: string; type: string; text: string }[], stopAtOver = false): Promise<void> {
  await target.evaluate((el, { files, stopAtOver }) => {
    const dt = new DataTransfer();
    for (const f of files) dt.items.add(new File([f.text], f.name, { type: f.type }));
    const fire = (type: string) => el.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt }));
    fire('dragenter');
    fire('dragover');
    if (!stopAtOver) fire('drop');
  }, { files, stopAtOver });
}

test('files dropped on the thread attach like a paste; an unsupported type is refused by name', async ({ page }) => {
  test.setTimeout(90_000);
  for (const provider of ['codex', 'claude'] as const) {
    const { thread } = await open(page, provider, `Drop ${provider}`);
    const box = thread.locator('.rt-composer');
    // Over the pane: only the composer lights up.
    await dropFiles(thread.locator('.rt-scroll'), [{ name: 'shot.png', type: 'image/png', text: 'png' }], true);
    await expect(box).toHaveAttribute('data-drop', '');
    await thread.locator('.rt-scroll').evaluate((el) => el.dispatchEvent(new DragEvent('dragleave', { bubbles: true, dataTransfer: (() => { const d = new DataTransfer(); d.items.add(new File(['x'], 'x.png', { type: 'image/png' })); return d; })() })));
    await expect(box).not.toHaveAttribute('data-drop', '');
    await dropFiles(thread.locator('.rt-scroll'), [{ name: 'shot.png', type: 'image/png', text: 'png' }]);
    await expect(thread.getByTestId('runtime-images')).toBeVisible();
    await expect(thread.getByTestId('runtime-images').getByRole('button', { name: 'Open shot.png' })).toHaveCount(1);
    await dropFiles(box, [{ name: 'notes.zip', type: 'application/zip', text: 'zip' }]);
    await expect(thread.getByTestId('runtime-attach-error')).toContainText('notes.zip not added: application/zip can’t be attached here');
    await expect(box).not.toHaveAttribute('data-drop', '');
  }
});

test('a terminal selection becomes an “Add to chat” context chip; nothing is sent, the draft stays', async ({ page }) => {
  test.setTimeout(120_000);
  const { thread } = await open(page, 'codex', 'Terminal chip');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('/terminal');
  await composer.press('Enter');
  const drawer = page.locator('[data-testid^="thread-terminal-drawer-"]');
  await expect(drawer).toBeVisible({ timeout: 30_000 });
  const screen = drawer.locator('.xterm-screen');
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await drawer.locator('.xterm').click();
  await page.keyboard.type('echo otto-selection-check\n');
  await composer.fill('keep this draft');
  const add = drawer.getByTestId('terminal-add-to-chat');
  await expect(add).toHaveCount(0);
  const box = (await screen.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + 4);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + 60, { steps: 8 });
  await page.mouse.up();
  await expect(add).toBeVisible();
  await expectFullyInViewport(page, add, 'Add to chat');
  await add.click();
  const chip = thread.getByTestId('runtime-context-chips');
  await expect(chip).toContainText(/Terminal · \d+ lines?/);
  await expect(composer).toHaveValue('keep this draft');
  await expect(thread.getByTestId('runtime-queued')).toHaveCount(0);
  await expect(thread.locator('.rt-user')).toHaveCount(0); // nothing was sent
});

test('codex: View › Tool calls hides successful (or all) calls; failures and replies always show', async ({ page }) => {
  test.setTimeout(120_000);
  const { thread } = await open(page, 'codex', 'Tool visibility');
  await send(thread, 'hello');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('tool image');
  await composer.press('Enter');
  const okTool = thread.locator('[data-item-type="dynamic_tool"]');
  await expect(okTool).toHaveCount(1, { timeout: 30_000 });
  await expect(thread.getByTestId('runtime-send')).toHaveAttribute('data-delivery', 'send', { timeout: 30_000 });
  await composer.fill('fail');
  await composer.press('Enter');
  const failed = thread.locator('.rw.failed[data-item-type="command_execution"]');
  await expect(failed).toHaveCount(1, { timeout: 30_000 });
  await expect(thread.getByTestId('runtime-send')).toHaveAttribute('data-delivery', 'send', { timeout: 30_000 });

  await pickView(page, 'Tool calls: hide successful');
  await expect(okTool).toHaveCount(0);
  await expect(failed).toHaveCount(1);
  await expect(thread.getByText('echo: hello')).toBeVisible();
  await pickView(page, 'Tool calls: hide all');
  await expect(okTool).toHaveCount(0);
  await expect(failed).toHaveCount(1);
  expect(await page.evaluate(() => localStorage.getItem('otto_threads_prefs_v1'))).toContain('"tools":"none"');
  await pickView(page, 'Tool calls: show all');
  await expect(okTool).toHaveCount(1);
});

test('gemini (ACP, otto-fake-agent): Queue only — the button and ⌘↵ say it cannot steer; its commands; a reply', async ({ page }) => {
  test.setTimeout(120_000);
  const { thread } = await open(page, 'gemini', 'ACP composer');
  await send(thread, 'hello');
  const composer = thread.getByTestId('runtime-composer');
  // Its own slash commands under its heading (/review is Otto's here).
  await composer.fill('/');
  const pop = thread.locator('[data-slash-pop]');
  await expect(pop.getByTestId('slash-group')).toHaveText(['Otto', /commands$/]);
  await expect(pop.locator('[data-source="agent"]')).toHaveText([/\/compact/]);
  await composer.fill('');
  // Working: Queue is the only mid-turn delivery, and the button says why.
  await composer.fill('hang');
  await composer.press('Enter');
  const primary = thread.getByTestId('runtime-send');
  const stop = thread.getByTestId('runtime-stop');
  await expect(stop).toBeVisible({ timeout: 20_000 });
  await composer.fill('after the hang');
  await expect(primary).toHaveAttribute('data-delivery', 'queue');
  await expect(primary).toHaveAttribute('title', /can’t take a message mid-turn/);
  await composer.press('Meta+Enter');
  await expect(page.getByText(/can’t take a message mid-turn/).first()).toBeVisible();
  await expect(thread.getByTestId('runtime-queued')).toHaveCount(1);
  // The View choice "steer" still queues here.
  await pickView(page, 'While working, messages steer');
  await composer.fill('second');
  await expect(primary).toHaveAttribute('data-delivery', 'queue');
  await stop.click();
  await expect(thread.getByText('echo: after the hang').last()).toBeVisible({ timeout: 30_000 });
});

test('gemini (ACP): gated tool calls never fold or hide; ↑ recalls the prompt', async ({ page }) => {
  test.setTimeout(90_000);
  const { thread } = await open(page, 'gemini', 'ACP gated tools');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('ask2');
  await composer.press('Enter');
  const dock = thread.getByTestId('runtime-attention');
  for (let i = 0; i < 2; i++) {
    await expect(dock).toBeVisible({ timeout: 20_000 });
    await dock.getByRole('button', { name: 'Allow', exact: true }).click();
  }
  await expect(dock).toHaveCount(0, { timeout: 20_000 });
  await expect(thread.getByTestId('runtime-send')).toHaveAttribute('data-delivery', 'send', { timeout: 20_000 });
  // Answered approvals keep their calls in sight (one group line that opens),
  // never in a fold, and "hide all" keeps them.
  const group = thread.getByText('Used 2 tools');
  await expect(group).toBeVisible();
  await expect(thread.locator('[data-testid^="runtime-fold-"]')).toHaveCount(0);
  await pickView(page, 'Tool calls: hide all');
  await expect(group).toBeVisible();
  await composer.focus();
  await composer.press('ArrowUp');
  await expect(composer).toHaveValue('ask2');
});

test('the Threads terminal dock: a selection → “Add to chat” goes to the focused thread’s draft', async ({ page }) => {
  test.setTimeout(120_000);
  // The page's bottom terminal dock, open from the start (threadsChrome, per device).
  await page.addInitScript(() => localStorage.setItem('otto_threads_chrome_v1', JSON.stringify({ terminalOpen: true })));
  const { thread } = await open(page, 'codex', 'Dock chip');
  const dock = page.getByTestId('threads-terminal-dock');
  await expect(dock).toBeVisible({ timeout: 30_000 });
  const screen = dock.locator('.xterm-screen').first();
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await dock.locator('.xterm').first().click();
  await page.keyboard.type('echo otto-dock-check\n');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('dock draft stays');
  const add = dock.getByTestId('terminal-add-to-chat');
  await expect(add).toHaveCount(0);
  const box = (await screen.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + 4);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + 60, { steps: 8 });
  await page.mouse.up();
  await expect(add).toBeVisible();
  await expectFullyInViewport(page, add, 'Add to chat (dock)');
  await add.click();
  await expect(thread.getByTestId('runtime-context-chips')).toContainText(/Terminal · \d+ lines?/);
  await expect(composer).toHaveValue('dock draft stays');
  await expect(thread.locator('.rt-user')).toHaveCount(0);
});
