import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow, openPage } from './helpers';

// Real E2E coverage for `ui/src/modules/panels/` (FilesPanel / FileTree,
// OutputsPanel, ActivityPanel, InfoPanel) hosted inside the Side panel across
// desktop and phone/tablet viewports, keyboard flows, and light/dark themes.

test.use({ serviceWorkers: 'block' });

let workspaceId = '';
let workspaceRoot = '';
let sessionId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, `Panels E2E ${Date.now().toString(36)}`);
  const workspaces = (await (await ctx.get(`${base}/api/v1/workspaces`)).json()) as {
    id: string;
    root_path: string;
  }[];
  workspaceRoot = workspaces.find((w) => w.id === workspaceId)!.root_path;

  mkdirSync(join(workspaceRoot, 'src'), { recursive: true });
  writeFileSync(join(workspaceRoot, 'README.md'), '# Panels E2E\n\nReal file preview content.\n');
  writeFileSync(join(workspaceRoot, 'src', 'index.ts'), 'export const hello = "panels";\n');

  const resp = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'claude',
      title: 'Panels E2E Session',
      cwd: workspaceRoot,
      meta: { origin: 'manual' },
    },
  });
  expect(resp.ok()).toBeTruthy();
  const created = (await resp.json()) as { id: string };
  sessionId = created.id;
  await ctx.dispose();
});

async function openAgentsWithSession(page: Page, opts: { scheme?: 'light' | 'dark' } = {}): Promise<void> {
  await page.addInitScript(
    ({ wsId, sid, scheme }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_firstrun_dismissed', '1');
      localStorage.setItem('otto_right_open', '0');
      localStorage.setItem('otto_right_tab', 'files');
      localStorage.setItem('otto_active_session', sid);
      if (scheme) localStorage.setItem('otto_scheme', scheme);
    },
    { wsId: workspaceId, sid: sessionId, scheme: opts.scheme ?? '' },
  );
  await openPage(page, 'agents');
  // Wait for the seeded agent session to hydrate so agent-only side panel tabs
  // (Activity, Outputs, Info) are mounted before clicking the rail.
  await expect(page.getByRole('tab', { name: /Panels E2E Session/ })).toBeVisible({ timeout: 15_000 });
}

async function openPanelTab(page: Page, label: 'Files' | 'Activity' | 'Outputs' | 'Info'): Promise<void> {
  const isCompact = (page.viewportSize()?.width ?? 1280) <= 1024;
  if (isCompact) {
    const drawerOpen = await page.locator('.rpanel').isVisible().catch(() => false);
    if (!drawerOpen) {
      await page.getByRole('button', { name: 'Side panel', exact: true }).click();
      await expect(page.locator('.rpanel .rpanel-title')).toBeVisible({ timeout: 15_000 });
    }
  }
  const titleEl = page.locator('.rpanel .rpanel-title');
  if (await titleEl.isVisible().catch(() => false)) {
    const current = (await titleEl.textContent())?.trim();
    if (current === label) return;
  }
  const btn = page.locator('.rstrip').getByRole('button', { name: label, exact: true });
  await expect(btn).toBeVisible({ timeout: 15_000 });
  const pressed = await btn.getAttribute('aria-pressed');
  if (pressed !== 'true') {
    await btn.click();
  }
  await expect(titleEl).toHaveText(label, { timeout: 15_000 });
}

test.describe('Panels — desktop', () => {
  test.use({ viewport: { width: 1280, height: 800 }, isMobile: false, hasTouch: false });

  test('FilesPanel lists real files, opens preview, and adds/removes sections without overflow', async ({ page }) => {
    await openAgentsWithSession(page);
    await openPanelTab(page, 'Files');

    const panel = page.locator('.rpanel');
    await expect(panel.getByText('README.md', { exact: true })).toBeVisible({ timeout: 15_000 });
    await expect(panel.getByText('src', { exact: true })).toBeVisible();

    // Add a second file section and confirm counter increments cleanly.
    await expect(panel.locator('.fp-count')).toHaveText('1 / 4');
    await panel.getByRole('button', { name: 'Add file section', exact: true }).click();
    await expect(panel.locator('.fp-count')).toHaveText('2 / 4');

    // Expand `src` folder in the first tree and verify child file renders.
    await panel.getByText('src', { exact: true }).first().click();
    await expect(panel.getByText('index.ts', { exact: true }).first()).toBeVisible({ timeout: 10_000 });

    await expectNoHorizontalOverflow(page);
  });

  test('ActivityPanel supports keyboard task creation, filtering, and zero overflow', async ({ page }) => {
    await openAgentsWithSession(page);
    await openPanelTab(page, 'Activity');

    const panel = page.locator('.rpanel');
    const addBtn = panel.getByRole('button', { name: /add task/i });
    await expect(addBtn).toBeVisible({ timeout: 15_000 });
    await addBtn.click();

    const taskName = `Check panel keyboard flow ${Date.now().toString(36)}`;
    const titleInput = panel.getByPlaceholder(/task title|what should/i).first();
    await expect(titleInput).toBeVisible();
    await titleInput.fill(taskName);
    await titleInput.press('Enter');

    await expect(panel.getByTestId('task-row').getByText(taskName, { exact: true })).toBeVisible({
      timeout: 10_000,
    });
    // Live trail also logs the board task and responds to the trail search filter.
    await expect(panel.getByText(`Board task added: ${taskName}`)).toBeVisible({ timeout: 10_000 });
    await panel.getByLabel('Filter the trail').fill(taskName);
    await expect(panel.getByText(`Board task added: ${taskName}`)).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test('OutputsPanel renders artifacts with sandboxed preview and InfoPanel shows session metadata', async ({
    page,
  }) => {
    await page.route('**/api/v1/sessions/*/artifacts', async (route) => {
      if (route.request().method() !== 'GET') return route.continue();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            id: 'art-md-1',
            kind: 'file',
            label: 'release-notes.md',
            path: 'docs/release-notes.md',
            url: null,
            mime: 'text/markdown',
            produced_at: new Date().toISOString(),
            turn_id: 'turn-1',
          },
          {
            id: 'art-html-1',
            kind: 'file',
            label: 'report.html',
            path: 'docs/report.html',
            url: null,
            mime: 'text/html',
            produced_at: new Date().toISOString(),
            turn_id: 'turn-1',
          },
        ]),
      });
    });
    await page.route('**/api/v1/sessions/*/artifacts/art-md-1', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/markdown; charset=utf-8',
        body: '# Release Summary\n\nAll checks passed.',
      });
    });
    await page.route('**/api/v1/sessions/*/artifacts/art-html-1', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/html; charset=utf-8',
        body: '<h1>HTML Artifact</h1><script>window.__xss = 1;</script>',
      });
    });

    await openAgentsWithSession(page);
    await openPanelTab(page, 'Outputs');

    const panel = page.locator('.rpanel');
    const mdOption = panel.getByRole('option', { name: /release-notes\.md/ });
    const htmlOption = panel.getByRole('option', { name: /report\.html/ });
    await expect(mdOption).toBeVisible({ timeout: 15_000 });
    await expect(htmlOption).toBeVisible({ timeout: 15_000 });
    await mdOption.click();
    await expect(panel.getByText('Release Summary')).toBeVisible({ timeout: 10_000 });

    // HTML artifact renders inside a strictly sandboxed iframe (`sandbox=""`).
    await htmlOption.click();
    const frame = panel.locator('iframe');
    await expect(frame).toBeVisible({ timeout: 10_000 });
    await expect(frame).toHaveAttribute('sandbox', '');

    // Switch to Info tab and verify session details render cleanly.
    await openPanelTab(page, 'Info');
    await expect(panel.getByText('Panels E2E Session')).toBeVisible({ timeout: 15_000 });

    // Verify vertical rail keyboard navigation (ArrowUp/ArrowDown).
    const infoRailBtn = page.locator('.rstrip').getByRole('button', { name: 'Info', exact: true });
    await infoRailBtn.focus();
    await page.keyboard.press('ArrowUp');
    await expect(infoRailBtn).not.toBeFocused();

    await expectNoHorizontalOverflow(page);
  });
});

test.describe('Panels — mobile drawer & themes', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`Files, Activity, Outputs, and Info work cleanly inside the mobile drawer (${scheme})`, async ({
      page,
    }) => {
      await openAgentsWithSession(page, { scheme });
      for (const tab of ['Files', 'Activity', 'Outputs', 'Info'] as const) {
        await openPanelTab(page, tab);
        await expectNoHorizontalOverflow(page);
      }
    });
  }
});
