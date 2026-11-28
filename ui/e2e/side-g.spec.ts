import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow, openPage } from './helpers';
import { apiCtx, seedWorkspace } from './seed';
import { seedInsights } from './insightsSeed';
import { agentStep, cancelStarted, chain, createWorkflow, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';

// The rc.6 side surfaces of Help, Settings, Insights, Skills Lab, Workflows and
// Personal agents' channels on the three sizes, driven with real taps on a
// phone: list → item → ‹ Back → another item; ⋯ › Hide / Show (⌘B) on desktop;
// the Detail panes open and close; ☰ and the Side panel still one tap away.
// With OTTO_SHOTS_DIR set it also saves light and dark screenshots.

test.use({ serviceWorkers: 'block' });

const SHOTS = process.env.OTTO_SHOTS_DIR;
const PHONE = { width: 390, height: 844 };
const TABLET = { width: 820, height: 1180 };
const DESKTOP = { width: 1440, height: 900 };

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}

let wsId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

async function prime(page: Page, s: 'light' | 'dark'): Promise<void> {
  await page.addInitScript(([v, id]) => {
    localStorage.setItem('otto_scheme', v as string);
    localStorage.setItem('otto_workspace', id as string);
  }, [s, wsId]);
}

/** Desktop: ⋯ › Hide <name>, "Show <name>" in the header, then ⌘B twice. */
async function hideShow(page: Page, paneTestid: string, name: string): Promise<void> {
  const pane = page.getByTestId(paneTestid);
  await page.getByTestId(`${paneTestid}-more`).click();
  await page.getByRole('menuitem', { name: new RegExp(`Hide ${name}`) }).click();
  await expect(pane).toBeHidden();
  await page.getByTestId('show-list-pane').click();
  await expect(pane).toBeVisible();
  await page.keyboard.press('Meta+b');
  await expect(pane).toBeHidden();
  await page.keyboard.press('Meta+b');
  await expect(pane).toBeVisible();
}

/** Phone: ☰ opens the Navigator, the Side panel button is in the header. */
async function shellDoors(page: Page): Promise<void> {
  await page.getByTestId('open-navigator').tap();
  await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('open-side-panel')).toBeVisible();
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`desktop (${s})`, () => {
    test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });

    test('Help: guides pane, hide / show, a guide open', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'walkthroughs/git');
      await expect(page.getByTestId('guides-list').getByRole('heading', { name: 'Guides' })).toBeVisible();
      await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'git');
      await shot(page, `help-desktop-${s}`);
      await hideShow(page, 'guides-list', 'Guides');
    });

    test('Settings: the 240 px section nav beside the section', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'settings/appearance');
      const nav = page.getByTestId('settings-nav');
      await expect(nav.locator('.settings-nav-item[aria-current="page"] .settings-nav-label')).toHaveText('Appearance');
      expect(Math.round((await nav.boundingBox())!.width)).toBe(240);
      await shot(page, `settings-desktop-${s}`);
      await hideShow(page, 'settings-nav', 'Settings');
      await nav.locator('.settings-nav-item', { hasText: 'Notifications' }).click();
      await expect(page.locator('.settings-body h1.ph-title')).toHaveText('Notifications');
    });

    test('Insights: reports pane opens on a report', async ({ page }) => {
      seedInsights();
      await prime(page, s);
      await openPage(page, 'insights');
      const list = page.getByTestId('report-list');
      await expect(list.locator('button.rep-row').first()).toHaveAttribute('aria-current', 'true', { timeout: 30_000 });
      await expect(page.getByTestId('insight-report')).toBeVisible();
      await shot(page, `insights-desktop-${s}`);
      await hideShow(page, 'report-list', 'Reports');
    });

    test('Skills Lab: skills pane, filters, + and ⋯', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'skills-eval');
      const pane = page.getByTestId('skills-list');
      await expect(pane.getByTestId('skill-row').first()).toBeVisible({ timeout: 30_000 });
      await expect(pane.getByTestId('new-skill')).toBeVisible();
      await expect(pane.locator('[data-testid="skill-row"][aria-current="true"]')).toHaveCount(1);
      await shot(page, `skills-desktop-${s}`);
      await hideShow(page, 'skills-list', 'Skills');
      await page.getByTestId('tab-review').click();
      await expect(page.getByTestId('skill-review')).toBeVisible({ timeout: 20_000 });
      await shot(page, `skills-review-desktop-${s}`);
      await page.getByTestId('tab-evaluator').click();
      await page.getByTestId('tab-matrix').click();
      await expect(page.getByTestId('matrix-view').getByRole('heading').first()).toBeVisible({ timeout: 20_000 });
      await shot(page, `skills-matrix-desktop-${s}`);
    });

    test('Agent channels: create two, list pane beside the channel', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'personal-agents/rooms');
      for (const name of [`side-g ${s} one`, `side-g ${s} two`]) {
        const add = page.getByTestId('pa-new-channel');
        if (await add.isVisible()) await add.click();
        else await page.getByRole('button', { name: 'Create a channel' }).click();
        const dlg = page.getByRole('dialog', { name: 'New channel' });
        await dlg.getByRole('textbox').fill(name);
        await dlg.getByRole('button', { name: 'Create' }).click();
        await expect(page.locator('.detail-title')).toHaveText(name);
      }
      await expect(page.getByTestId('pa-channels').getByTestId('pa-channel-row').first()).toBeVisible();
      await shot(page, `channels-desktop-${s}`);
      await hideShow(page, 'pa-channels', 'Channels');
    });
  });

  test.describe(`tablet (${s})`, () => {
    test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
    for (const [name, route, pane] of [
      ['help', 'walkthroughs', 'guides-list'],
      ['settings', 'settings/appearance', 'settings-nav'],
      ['skills', 'skills-eval', 'skills-list'],
    ] as const) {
      test(`${name}: list pane beside the content, one header row`, async ({ page }) => {
        await prime(page, s);
        await openPage(page, route);
        await expect(page.getByTestId(pane)).toBeVisible({ timeout: 30_000 });
        await expectNoHorizontalOverflow(page);
        await shot(page, `${name}-tablet-${s}`);
      });
    }
  });

  test.describe(`phone (${s})`, () => {
    test.use({ viewport: PHONE, isMobile: true, hasTouch: true });

    test('Help: push guides, Back keeps the mark', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'walkthroughs');
      const rows = page.getByTestId('guide-row');
      await expect(rows.nth(2)).toBeVisible();
      await expect(page.getByTestId('guide-article')).toBeHidden();
      await expectNoHorizontalOverflow(page);
      await shot(page, `help-phone-list-${s}`);
      const second = await rows.nth(2).getAttribute('data-guide');
      await rows.nth(2).tap();
      await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', second!);
      const back = page.getByRole('button', { name: 'Back to Guides', exact: true });
      await expect(back).toBeVisible();
      await shot(page, `help-phone-content-${s}`);
      await back.tap();
      await expect(rows.nth(2)).toHaveAttribute('aria-current', 'page');
      await rows.nth(3).tap();
      await expect(page.getByTestId('guide-article')).toBeVisible();
      await back.tap();
      await shellDoors(page);
    });

    test('Settings: two levels with ‹ Settings in the header', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'settings');
      const nav = page.getByTestId('settings-nav');
      await expect(nav.locator('.settings-nav-item').first()).toBeVisible();
      await expectNoHorizontalOverflow(page);
      await shot(page, `settings-phone-list-${s}`);
      await nav.locator('.settings-nav-item', { hasText: 'Notifications' }).tap();
      const back = page.getByRole('button', { name: 'Back to Settings', exact: true });
      await expect(back).toBeVisible();
      await expect(nav).toBeHidden();
      await shot(page, `settings-phone-content-${s}`);
      await back.tap();
      await expect(nav.locator('.settings-nav-item[aria-current="page"] .settings-nav-label')).toHaveText('Notifications');
      await nav.locator('.settings-nav-item', { hasText: 'Session names' }).tap();
      await expect(page.locator('.settings-body h1.ph-title').last()).toBeVisible();
      await back.tap();
      await shellDoors(page);
    });

    test('Insights: push a report, Back, another', async ({ page }) => {
      seedInsights();
      await prime(page, s);
      await openPage(page, 'insights');
      const rows = page.getByTestId('report-list').locator('button.rep-row');
      await expect(rows.first()).toBeVisible({ timeout: 30_000 });
      await expectNoHorizontalOverflow(page);
      await shot(page, `insights-phone-list-${s}`);
      await rows.nth(1).tap();
      await expect(page.getByTestId('insight-report')).toBeVisible();
      const back = page.getByRole('button', { name: 'Back to Reports', exact: true });
      await shot(page, `insights-phone-content-${s}`);
      await back.tap();
      await expect(rows.nth(1)).toHaveAttribute('aria-current', 'true');
      await rows.nth(2).tap();
      await expect(page.getByTestId('insight-report')).toBeVisible();
      await back.tap();
      await shellDoors(page);
    });

    test('Skills Lab: push a skill, Back, another', async ({ page }) => {
      await prime(page, s);
      await openPage(page, 'skills-eval');
      const rows = page.getByTestId('skill-row');
      await expect(rows.nth(1)).toBeVisible({ timeout: 30_000 });
      await expectNoHorizontalOverflow(page);
      await shot(page, `skills-phone-list-${s}`);
      await rows.nth(1).tap();
      const back = page.getByRole('button', { name: 'Back to Skills', exact: true });
      await expect(back).toBeVisible();
      await expectNoHorizontalOverflow(page);
      await shot(page, `skills-phone-content-${s}`);
      await back.tap();
      await expect(rows.nth(1)).toHaveAttribute('aria-current', 'true');
      await rows.nth(2).tap();
      await expect(back).toBeVisible();
      await back.tap();
      await shellDoors(page);
    });
  });
}

// Workflows need a live run: one workflow, a run whose second step keeps
// running, shared by the size tests below.
test.describe('Workflows run and editor', () => {
  let api: WfApi;
  let wf = '';
  let run = '';
  test.beforeAll(async () => {
    api = await wfApi('E2E side-g');
    wf = await createWorkflow(api, 'E2E side-g run', chain(agentStep('a', 'Gather'), agentStep('b', 'Work for a while', ['sleep 120000'])));
    run = await startRun(api, wf);
    await waitRun(api, run, (r) => r.nodes.some((n) => n.node_id === 'b' && n.status === 'running'));
  });
  test.afterAll(async () => {
    await cancelStarted(api);
    await api?.ctx.dispose();
  });
  const primeWf = (page: Page, s: string) =>
    page.addInitScript(([v, id]) => {
      localStorage.setItem('otto_scheme', v as string);
      localStorage.setItem('otto_workspace', id as string);
    }, [s, api.ws]);

  for (const s of ['light', 'dark'] as const) {
    test.describe(`desktop (${s})`, () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('run column is the list pane; the inspector is the Detail pane', async ({ page }) => {
        await primeWf(page, s);
        await page.goto(`/#/workflows/${wf}/runs/${run}`);
        await expect(page.getByTestId('wf-run-column')).toBeVisible({ timeout: 30_000 });
        await expect(page.getByTestId('wf-step-pane')).toBeVisible();
        await shot(page, `workflows-run-desktop-${s}`);
        await hideShow(page, 'wf-run-column', 'Run');
        await page.getByRole('button', { name: 'Maximize conversation' }).click();
        await expect(page.getByTestId('wf-run-column')).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await page.goto(`/#/workflows/${wf}`);
        const insp = page.getByTestId('wf-inspector');
        await expect(insp).toBeVisible({ timeout: 30_000 });
        await shot(page, `workflows-editor-desktop-${s}`);
        await page.getByRole('button', { name: 'Hide inspector' }).click();
        await expect(insp).toBeHidden();
        await page.getByRole('button', { name: 'Show inspector' }).click();
        await expect(insp).toBeVisible();
        await insp.getByRole('tab', { name: 'Triggers' }).click();
        await shot(page, `workflows-editor-detail-desktop-${s}`);
        await page.getByTestId('wf-inspector-close').click();
        await expect(insp).toBeHidden();
      });
    });
    test.describe(`phone (${s})`, () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('run: push a step, Back; editor: the step sheet', async ({ page }) => {
        await primeWf(page, s);
        await page.goto(`/#/workflows/${wf}/runs/${run}`);
        const step = page.locator('[data-testid="wf-step"][data-node-id="b"]');
        await expect(step).toBeVisible({ timeout: 30_000 });
        await expectNoHorizontalOverflow(page);
        await shot(page, `workflows-run-phone-list-${s}`);
        await step.tap();
        const back = page.getByRole('button', { name: 'Back to Run', exact: true });
        await expect(back).toBeVisible();
        await expect(page.getByTestId('wf-step-pane')).toBeVisible();
        await shot(page, `workflows-run-phone-content-${s}`);
        await back.tap();
        await expect(step).toHaveAttribute('aria-current', 'true');
        await page.getByRole('button', { name: /steps done/ }).tap();
        await page.locator('[data-testid="wf-step"][data-node-id="a"]').tap();
        await expect(back).toBeVisible();
        await back.tap();
        await shellDoors(page);
        await page.goto(`/#/workflows/${wf}`);
        await page.getByTestId('wf-phone-outline').locator('[data-node-id="a"]').tap();
        const sheet = page.getByTestId('wf-step-sheet');
        await expect(sheet).toBeVisible();
        await shot(page, `workflows-editor-phone-sheet-${s}`);
        await page.getByTestId('wf-step-sheet-close').tap();
        await expect(sheet).toBeHidden();
      });
    });
  }
});
