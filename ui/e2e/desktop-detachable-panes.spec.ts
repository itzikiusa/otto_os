import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// This suite exercises the real Svelte host against a controlled native IPC
// boundary. Actual WKWebView movement is covered by the standalone desktop
// probe; a Chromium test cannot prove native reparenting or macOS close events.
type NativeFixture = {
  calls: { command: string; args: Record<string, unknown> }[];
  failNextOpen: boolean;
  emit: (event: string, payload: unknown) => void;
};
type FixtureWindow = Window & { __paneFixture: NativeFixture };

async function installNativeFixture(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (value: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let nextId = 1;
    let state: Record<string, unknown> | null = null;
    const fixture: NativeFixture = {
      calls: [],
      failNextOpen: false,
      emit(event, payload) {
        for (const [id, listener] of listeners) {
          if (listener.event === event) callbacks.get(listener.handler)?.({ id, event, payload });
        }
      },
    };
    Object.assign(window, {
      __paneFixture: fixture,
      __TAURI_EVENT_PLUGIN_INTERNALS__: {
        unregisterListener: (_event: string, id: number) => listeners.delete(id),
      },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
        transformCallback(callback: (value: unknown) => void) {
          const id = nextId++;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        async invoke(command: string, args: Record<string, unknown> = {}) {
          fixture.calls.push({ command, args });
          if (command === 'plugin:event|listen') {
            const id = nextId++;
            listeners.set(id, { event: String(args.event), handler: Number(args.handler) });
            return id;
          }
          if (command === 'plugin:event|unlisten') return;
          if (command === 'windows_registry') return ['main'];
          if (command === 'pane_state') return state;
          if (command === 'pane_open') {
            if (fixture.failNextOpen) {
              fixture.failNextOpen = false;
              throw new Error('Native pane creation failed');
            }
            state = {
              host: 'main', child: 'pane-main', mode: 'attached',
              alwaysOnTop: false, fullscreen: false, visible: true, focused: false,
            };
            fixture.emit('otto://pane-state', state);
            setTimeout(() => fixture.emit('otto://pane-to-host', {
              ns: 'otto-side', type: 'ready', route: args.route,
            }), 0);
            return state;
          }
          if (command === 'pane_detach' || command === 'pane_return') {
            state = { ...state, mode: command === 'pane_return' ? 'attached' : args.pane };
            fixture.emit('otto://pane-state', state);
            return state;
          }
          if (command === 'pane_close') {
            state = null;
            fixture.emit('otto://pane-state', null);
          }
          if (command === 'pane_monitors') return [{ id: 0, name: 'Built-in display', current: true }];
          if (command === 'pane_window_action') return state;
          if (command === 'plugin:window|is_focused') return true;
          return null;
        },
      },
    });
  });
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop shell fixture');
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.removeItem('otto_side_pane');
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
  await installNativeFixture(page);
});

async function openPair(page: Page): Promise<void> {
  await page.goto('/#/agents');
  const row = page.locator('.navigator [data-nav-id="connections"]').first();
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.click({ modifiers: ['Alt'] });
  await expect(page.getByTestId('side-pane-cover')).toHaveCount(0);
  await expect.poll(() => calls(page, 'pane_open')).toBe(1);
}

async function calls(page: Page, command: string): Promise<number> {
  return page.evaluate((cmd) => (window as unknown as FixtureWindow).__paneFixture.calls.filter((c) => c.command === cmd).length, command);
}

async function paneAction(page: Page, label: string): Promise<void> {
  await page.getByTestId('pane-window-menu').first().click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: label, exact: true }).click();
}

test('native side surface opens once and receives bounded geometry', async ({ page }) => {
  await openPair(page);
  await expect(page.getByTestId('side-pane-frame')).toHaveCount(0);
  await expect(page.getByTestId('split-divider')).toBeVisible();
  await expect.poll(() => calls(page, 'pane_layout')).toBeGreaterThan(0);
  const bounds = await page.evaluate(() => {
    const entries = (window as unknown as FixtureWindow).__paneFixture.calls;
    return entries.filter((c) => c.command === 'pane_layout').at(-1)?.args.bounds as {
      x: number; y: number; width: number; height: number;
    };
  });
  expect(bounds.width).toBeGreaterThan(0);
  expect(bounds.height).toBeGreaterThan(0);
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.y).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(1281);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(801);
  const eventTargets = await page.evaluate(() => (window as unknown as FixtureWindow).__paneFixture.calls
    .filter((c) => c.command === 'plugin:event|listen' && ['otto://pane-state', 'otto://pane-to-host', 'otto://menu'].includes(String(c.args.event)))
    .map((c) => c.args.target));
  expect(eventTargets).toHaveLength(3);
  for (const target of eventTargets) expect(target).toEqual({ kind: 'Webview', label: 'main' });
});

for (const which of ['main', 'side'] as const) {
  test(`detach ${which} and return retains the primary document and one native child`, async ({ page }) => {
    await openPair(page);
    const primary = page.locator('.primary-pane .center');
    const before = (await primary.boundingBox())!.width;
    await primary.evaluate((node) => {
      (window as Window & { __originalCenter?: Element }).__originalCenter = node;
      node.setAttribute('data-live-draft', 'unsaved value');
    });
    await paneAction(page, `Detach ${which} pane`);
    await expect.poll(() => calls(page, 'pane_detach')).toBe(1);
    await expect(page.getByTestId('split-divider')).toHaveCount(0);
    await expect.poll(async () => (await primary.boundingBox())!.width).toBeGreaterThan(before);

    // Narrowing the physical host must not destroy the side document, even
    // though the responsive shell stops showing an attached split here.
    await page.setViewportSize({ width: 780, height: 640 });
    await expect(page.getByTestId('side-pane')).toHaveCount(1);
    expect(await calls(page, 'pane_close')).toBe(0);
    await page.setViewportSize({ width: 1280, height: 800 });
    await paneAction(page, 'Return to split');
    await expect(page.getByTestId('split-divider')).toBeVisible();
    expect(await calls(page, 'pane_open')).toBe(1);
    expect(await calls(page, 'pane_close')).toBe(0);
    await expect(primary).toHaveAttribute('data-live-draft', 'unsaved value');
    expect(await primary.evaluate((node) => node === (window as Window & { __originalCenter?: Element }).__originalCenter)).toBe(true);
    await expect.poll(async () => (await primary.boundingBox())!.width).toBeCloseTo(before, 0);
  });
}

test('native close-to-return event restores the split without recreating the pane', async ({ page }) => {
  await openPair(page);
  await paneAction(page, 'Detach side pane');
  await expect(page.getByTestId('split-divider')).toHaveCount(0);
  await page.evaluate(() => (window as unknown as FixtureWindow).__paneFixture.emit('otto://pane-state', {
    host: 'main', child: 'pane-main', mode: 'attached',
    alwaysOnTop: false, fullscreen: false, visible: true,
  }));
  await expect(page.getByTestId('split-divider')).toBeVisible();
  expect(await calls(page, 'pane_open')).toBe(1);
  expect(await calls(page, 'pane_close')).toBe(0);
});

test('a failed creation offers Retry and keeps the original pane available', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });
  await page.evaluate(() => { (window as unknown as FixtureWindow).__paneFixture.failNextOpen = true; });
  await page.locator('.navigator [data-nav-id="connections"]').first().click({ modifiers: ['Alt'] });
  await expect(page.getByTestId('side-pane-cover').getByRole('button', { name: 'Retry' })).toBeVisible();
  await expect(page.locator('.primary-pane .center')).toBeVisible();
  await page.getByTestId('side-pane-cover').getByRole('button', { name: 'Retry' }).click();
  await expect.poll(() => calls(page, 'pane_open')).toBe(2);
  await expect(page.getByTestId('side-pane-cover')).toHaveCount(0);
  await expect(page.getByTestId('split-divider')).toBeVisible();
});

test('host menu occludes an attached native surface and restores it on dismissal', async ({ page }) => {
  await openPair(page);
  await page.getByTestId('pane-window-menu').first().click();
  await expect(page.locator('.ctx-menu')).toBeVisible();
  const lastVisible = () => page.evaluate(() => (window as unknown as FixtureWindow).__paneFixture.calls.filter((c) => c.command === 'pane_layout').at(-1)?.args.visible);
  await expect.poll(lastVisible).toBe(false);
  await page.keyboard.press('Escape');
  await expect(page.locator('.ctx-menu')).toHaveCount(0);
  await expect.poll(lastVisible).toBe(true);
  expect(await calls(page, 'pane_open')).toBe(1);
  expect(await calls(page, 'pane_close')).toBe(0);
});

test('agent shell commands still find and focus a detached pane from a narrow host', async ({ page }) => {
  await openPair(page);
  await paneAction(page, 'Detach side pane');
  await page.setViewportSize({ width: 780, height: 640 });
  const results: { ok: boolean; result?: { pane?: string }; error?: unknown }[] = [];
  await page.route('**/ui/commands/detachable-*/*', async (route) => {
    if (route.request().url().endsWith('/result')) results.push(route.request().postDataJSON());
    await route.fulfill({ status: 200, body: '{}' });
  });
  // Deliver through the real UI command dispatcher. Permission/grant decisions
  // belong to the daemon and are exercised in desktop-agent-ui-control.spec.ts.
  for (const [command, args] of [['open', { module: 'connections' }], ['focus', { pane: 'side' }]] as const) {
    await page.evaluate(async ({ command, args }) => {
      const modulePath = '/src/lib/uiCommands.ts';
      const bridge = await import(modulePath);
      bridge.handleUiFrame({ type: 'hello_ack', conn_id: 'detachable-fixture' });
      bridge.handleUiFrame({
        type: 'ui_command', id: `detachable-${command}`, session_id: 'fixture-agent',
        command, args, deadline_ms: 5000,
        agent: { session_id: 'fixture-agent', title: 'Pane test', provider: 'shell' },
      });
    }, { command, args });
  }
  await expect.poll(() => results.length).toBe(2);
  for (const result of results) {
    expect(result.error).toBeUndefined();
    expect(result.ok).toBe(true);
    expect(result.result?.pane).toBe('side');
  }
  const focus = await page.evaluate(() => (window as unknown as FixtureWindow).__paneFixture.calls.filter((c) => c.command === 'pane_focus').at(-1)?.args);
  expect(focus).toEqual({ pane: 'side' });
  expect(await calls(page, 'pane_open')).toBe(1);
});

test('native Close Tab honors actual primary focus over an old side-focus report', async ({ page }) => {
  await openPair(page);
  await page.bringToFront();
  await page.getByTestId('pane-window-menu').first().focus();
  expect(await page.evaluate(() => document.hasFocus())).toBe(true);
  const before = await calls(page, 'pane_focus');
  await page.evaluate(() => {
    const fixture = (window as unknown as FixtureWindow).__paneFixture;
    fixture.emit('otto://pane-to-host', { ns: 'otto-side', type: 'focus' });
    fixture.emit('otto://menu', 'close-tab');
  });
  await expect.poll(() => calls(page, 'pane_focus')).toBeGreaterThan(before);
  await expect(page.getByTestId('side-pane')).toHaveCount(1);
  expect(await calls(page, 'pane_close')).toBe(0);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`${scheme} detached window menu stays within the viewport`, async ({ page }, info) => {
    await page.addInitScript((value) => {
      localStorage.setItem('otto_theme', 'native');
      localStorage.setItem('otto_scheme', value);
    }, scheme);
    await page.emulateMedia({ colorScheme: scheme });
    await openPair(page);
    await paneAction(page, 'Detach main pane');
    await page.setViewportSize({ width: 780, height: 640 });
    await page.getByTestId('pane-window-menu').first().click();
    await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Return to split', exact: true })).toBeVisible();
    await expectFullyInViewport(page, page.locator('.ctx-menu'), 'detached pane window menu');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`detachable-${scheme}.png`) });
    await info.attach(`Detached pane menu (${scheme}, native IPC fixture)`, {
      path: info.outputPath(`detachable-${scheme}.png`), contentType: 'image/png',
    });
  });
}
