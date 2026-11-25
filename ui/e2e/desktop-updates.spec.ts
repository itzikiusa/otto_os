// Desktop updates UI (lib/stores/updates.svelte.ts, shell/UpdateRow|UpdateDialog,
// Settings ▸ Updates) against a mocked desktop shell: `invoke` and the
// `otto://update` / `otto://menu` events are faked here, the daemon's
// restart-impact count is routed. Light and dark.
import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Review screenshots land in the test's output folder (update-<state>-<scheme>.png).
const shot = async (page: Page, name: string) => {
  await page.screenshot({ path: test.info().outputPath(`update-${name}.png`), animations: 'disabled' });
};

type Fx = {
  calls: { command: string; args: Record<string, unknown> }[];
  state: Record<string, unknown>;
  prefs: { auto_install: boolean; skipped: string | null };
  checkFails: string | null;
  emit: (event: string, payload: unknown) => void;
};

const NOTES = [
  '### What’s new',
  '- **IDE** tabs survive a restart',
  '- See [the changelog](https://github.com/itzikiusa/otto_os/releases)',
  '<script>window.__pwned = 1</script><img src=x onerror="window.__pwned = 2">',
].join('\n');

async function shell(page: Page, init: { state: Record<string, unknown>; prefs?: Fx['prefs']; rail?: boolean; scheme: string }) {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(({ wsId, init }) => {
    localStorage.setItem('otto_workspace', wsId);
    localStorage.setItem('otto_rail_expanded', init.rail ? '0' : '1');
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_scheme', init.scheme);
    const callbacks = new Map<number, (value: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let id = 0;
    const fx: Fx = {
      calls: [],
      state: init.state,
      prefs: init.prefs ?? { auto_install: false, skipped: null },
      checkFails: null,
      emit(event, payload) {
        for (const h of listeners.get(event) ?? []) callbacks.get(h)?.({ event, id: 0, payload });
      },
    };
    Object.assign(window, {
      __upd: fx,
      __OTTO_WIN__: 'main',
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
        transformCallback(cb: (value: unknown) => void) { callbacks.set(++id, cb); return id; },
        unregisterCallback(key: number) { callbacks.delete(key); },
        convertFileSrc: (p: string) => p,
        async invoke(command: string, args: Record<string, unknown> = {}) {
          fx.calls.push({ command, args });
          if (command === 'plugin:event|listen') {
            const ev = String(args.event);
            listeners.set(ev, [...(listeners.get(ev) ?? []), Number(args.handler)]);
            return ++id;
          }
          if (command === 'windows_registry') return ['main'];
          if (command === 'update_state') return fx.state;
          if (command === 'update_prefs_get') return fx.prefs;
          if (command === 'update_prefs_set') { fx.prefs = { ...fx.prefs, ...(args.prefs as object) }; return null; }
          if (command === 'update_check') {
            if (fx.checkFails) throw new Error(fx.checkFails);
            return fx.state;
          }
          return null;
        },
      },
    });
  }, { wsId: workspace, init });
}

const fx = (page: Page) => ({
  emit: (payload: unknown) => page.evaluate((p) => (window as unknown as { __upd: Fx }).__upd.emit('otto://update', p), payload),
  menu: (id: string) => page.evaluate((m) => (window as unknown as { __upd: Fx }).__upd.emit('otto://menu', m), id),
  calls: (command: string) => page.evaluate((c) => (window as unknown as { __upd: Fx }).__upd.calls.filter((x) => x.command === c), command),
  set: (patch: Partial<Pick<Fx, 'state' | 'checkFails'>>) => page.evaluate((p) => Object.assign((window as unknown as { __upd: Fx }).__upd, p), patch),
});

const AVAILABLE = { phase: 'available', current: '0.5.3', version: '0.5.4', channel: 'release', notes: NOTES, date: '2026-11-25T09:00:00Z', size: 131_000_000, last_check: '2026-11-25T10:00:00Z' };

for (const scheme of ['light', 'dark'] as const) {
  test(`update flow: row → notes → download → restart with what ends (${scheme})`, async ({ page }) => {
    await shell(page, { state: AVAILABLE, scheme });
    let impactCalls = 0;
    await page.route('**/api/v1/system/restart-impact', (r) => {
      impactCalls++;
      return r.fulfill({ json: { keeps: [{ label: 'agent sessions', count: 4 }], ends: [{ label: 'connection terminals', count: 2, detail: ['prod-db', 'k8s exec'] }] } });
    });
    await page.goto('/#/agents');
    const row = page.locator('.navigator [data-testid="update-row"]');
    await expect(row).toHaveText(/Update to 0\.5\.4/, { timeout: 30_000 });
    // Above Help, in the footer.
    const [rowBox, helpBox] = await Promise.all([row.boundingBox(), page.locator('.nav-foot button[title="Help"]').boundingBox()]);
    expect(rowBox!.y).toBeLessThan(helpBox!.y);
    await shot(page, `sidebar-available-${scheme}`);

    await row.click();
    const dialog = page.getByTestId('update-dialog');
    await expect(page.getByRole('dialog', { name: 'Otto 0.5.4' })).toBeVisible();
    await expect(dialog).toContainText('You have 0.5.3');
    await expect(dialog).toContainText('125 MB');
    const notes = page.getByTestId('update-notes');
    await expect(notes.locator('strong')).toHaveText('IDE');
    // Untrusted notes: no script, no event handlers; links go to the system browser.
    await expect(notes.locator('script, img')).toHaveCount(0);
    expect(await notes.innerHTML()).not.toContain('onerror');
    expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
    await shot(page, `dialog-available-${scheme}`);
    await notes.getByRole('link', { name: 'the changelog' }).click();
    await expect.poll(async () => (await fx(page).calls('plugin:opener|open_url')).map((c) => c.args.url)).toEqual(['https://github.com/itzikiusa/otto_os/releases']);

    await page.getByTestId('update-primary').click();
    await expect.poll(async () => (await fx(page).calls('update_download')).length).toBe(1);
    await fx(page).emit({ ...AVAILABLE, phase: 'downloading', pct: 42 });
    await expect(page.getByTestId('update-primary')).toHaveText('Downloading… 42 %');
    await expect(page.getByTestId('update-primary')).toBeDisabled();
    await expect(row).toHaveText(/Downloading… 42 %/);
    await shot(page, `dialog-downloading-${scheme}`);

    await fx(page).emit({ ...AVAILABLE, phase: 'staged' });
    await expect(page.getByTestId('restart-impact')).toContainText('Keeps running: 4 agent sessions.');
    await expect(page.getByTestId('restart-impact')).toContainText('Ends: 2 connection terminals (prod-db, k8s exec).');
    await expect(page.getByTestId('update-primary')).toHaveText('Restart and end 2');
    await expect(row).toHaveText(/Restart to update/);
    await shot(page, `dialog-staged-${scheme}`);
    expect(impactCalls).toBe(1);

    // Something ends: one more, attributed confirm. Cancel restarts nothing.
    await page.getByTestId('update-primary').click();
    const confirm = page.getByRole('dialog', { name: 'Restart to update to 0.5.4?' });
    await expect(confirm).toContainText('Restarting Otto ends: 2 connection terminals (prod-db, k8s exec)');
    await shot(page, `restart-confirm-${scheme}`);
    await confirm.getByRole('button', { name: 'Cancel' }).click();
    expect(await fx(page).calls('update_restart')).toHaveLength(0);
    await page.getByTestId('update-primary').click();
    await page.getByRole('dialog', { name: 'Restart to update to 0.5.4?' }).getByRole('button', { name: 'Restart and end 2' }).click();
    await expect.poll(async () => (await fx(page).calls('update_restart')).length).toBe(1);
    await expect(page.getByTestId('update-primary')).toHaveText('Restarting…');
  });

  test(`Check for Updates… menu: up to date, offline with Retry, source build (${scheme})`, async ({ page }) => {
    await shell(page, { state: { phase: 'idle', current: '0.5.3', channel: 'release', last_check: '2026-11-25T10:00:00Z' }, scheme });
    await page.goto('/#/agents');
    await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });
    await expect(page.getByTestId('update-row')).toHaveCount(0);
    await expect.poll(async () => (await fx(page).calls('plugin:event|listen')).some((c) => c.args.event === 'otto://menu')).toBe(true);
    await fx(page).menu('check-updates');
    await expect(page.getByRole('dialog', { name: 'You’re up to date' })).toContainText('Otto 0.5.3 is the newest version.');
    expect(await fx(page).calls('update_check')).toHaveLength(1);
    await shot(page, `uptodate-${scheme}`);
    await page.getByTestId('update-primary').click();
    await expect(page.getByTestId('update-dialog')).toHaveCount(0);

    await fx(page).set({ checkFails: 'You’re offline — Otto couldn’t reach github.com' });
    await fx(page).menu('check-updates');
    const dialog = page.getByTestId('update-dialog');
    await expect(dialog).toContainText('Couldn’t check for updates');
    await expect(dialog).toContainText('You’re offline');
    await shot(page, `offline-${scheme}`);
    await fx(page).set({ checkFails: null, state: AVAILABLE });
    await dialog.getByRole('button', { name: 'Retry' }).click();
    await expect(page.getByRole('dialog', { name: 'Otto 0.5.4' })).toBeVisible();
    await page.getByRole('button', { name: 'Later' }).click();

    await fx(page).set({ state: { phase: 'source', current: '0.5.3', channel: 'source' } });
    await fx(page).menu('check-updates');
    await expect(page.getByTestId('update-dialog')).toContainText('This is a source build — updates come from git.');
    await shot(page, `source-${scheme}`);
    await expect(page.getByTestId('update-row')).toHaveCount(0);
  });
}

test('⋯ skips the version (row hides), Settings ▸ Updates unskips it and toggles automatic download', async ({ page }) => {
  await shell(page, { state: AVAILABLE, scheme: 'light' });
  await page.goto('/#/agents');
  await page.locator('.navigator [data-testid="update-row"]').click({ timeout: 30_000 });
  await page.getByTestId('update-more').click();
  await expect(page.getByRole('menuitemcheckbox', { name: /Install updates automatically/ })).toHaveAttribute('aria-checked', 'false');
  await shot(page, 'more-menu-light');
  await page.getByRole('menuitem', { name: 'Skip this version' }).click();
  await expect(page.getByTestId('update-dialog')).toHaveCount(0);
  await expect(page.getByTestId('update-row')).toHaveCount(0);

  await page.goto('/#/settings/updates');
  const card = page.getByTestId('updates-settings');
  await expect(card).toContainText('Otto 0.5.3');
  await expect(card).toContainText('Release build · last checked:');
  await expect(card).toContainText('Otto 0.5.4 isn’t offered in the sidebar.');
  await shot(page, 'settings-light');
  await card.getByRole('checkbox', { name: 'Install updates automatically' }).check();
  await expect.poll(async () => (await fx(page).calls('update_prefs_set')).map((c) => JSON.stringify(c.args.prefs))).toEqual(['{"skipped":"0.5.4"}', '{"auto_install":true}']);
  await card.getByRole('button', { name: 'Unskip' }).click();
  await expect(card).not.toContainText('isn’t offered');
  await expect(page.locator('.navigator [data-testid="update-row"]')).toHaveText(/Update to 0\.5\.4/);
});

test('after relaunch: a rolled-back update shows inline in the collapsed rail; a good one toasts', async ({ page }) => {
  await shell(page, {
    rail: true,
    scheme: 'dark',
    state: { phase: 'idle', current: '0.5.3', channel: 'release', last_result: { ok: false, from: '0.5.3', to: '0.5.4', reason: 'Otto 0.5.4 did not answer within 120 s.' } },
  });
  await page.goto('/#/agents');
  const railRow = page.locator('[data-testid="update-row"][aria-label="Update failed and was undone"]');
  await expect(railRow).toBeVisible({ timeout: 30_000 });
  await expect(railRow).toHaveAttribute('title', 'Update failed and was undone');
  await shot(page, 'rail-failed-dark');
  await railRow.click();
  const dialog = page.getByRole('dialog', { name: 'Update failed and was undone' });
  await expect(dialog).toContainText('did not answer within 120 s');
  await dialog.getByRole('button', { name: 'Show the update log' }).click();
  await expect.poll(async () => (await fx(page).calls('update_open_log')).length).toBe(1);
  await shot(page, 'failed-dialog-dark');
  await page.getByTestId('update-primary').click();
  await expect(page.getByTestId('update-row')).toHaveCount(0);
  await fx(page).emit({ phase: 'idle', current: '0.5.4', channel: 'release', last_result: { ok: true, from: '0.5.3', to: '0.5.4' } });
  await expect(page.getByText('Updated to 0.5.4')).toBeVisible();
});

test('outside the desktop app there is no update UI at all', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
  await page.goto('/#/settings/updates');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('updates-settings')).toHaveCount(0);
  await expect(page.getByTestId('update-row')).toHaveCount(0);
});
