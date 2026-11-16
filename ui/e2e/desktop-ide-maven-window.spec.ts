import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { daemonCall } from './ide-polyglot-fixtures';

// The IDE's Maven tool window on a multi-root, multi-module project (the shape
// of a real Java workspace: one flat pom, two 6-module reactors and a client
// lib whose only <name> tags live in <distributionManagement>). The window must
// load once, stay still while idle and while hovered, and take clicks.

const SHOT_DIR = process.env.OTTO_MAVEN_SHOTS;

interface Seed {
  dir: string;
  roots: string[];
  workspaceId: string;
}

const RELEASES = (id: string) =>
  `  <distributionManagement>\n    <repository><id>${id}</id><name>Corp Releases</name><url>\${NEXUSURL}/releases</url></repository>\n` +
  `    <snapshotRepository><id>snapshots</id><name>Corp Releases</name><url>\${NEXUSURL}/snapshots</url></snapshotRepository>\n` +
  `  </distributionManagement>\n  <repositories>\n    <repository><id>${id}</id><name>Corp Releases</name><url>\${NEXUSURL}/releases</url></repository>\n  </repositories>\n`;

function pom(opts: { group: string; artifact: string; name?: string; parent?: string; modules?: string[] }): string {
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<project xmlns="http://maven.apache.org/POM/4.0.0">',
    '  <modelVersion>4.0.0</modelVersion>',
    opts.parent ? `  <parent><groupId>${opts.group}</groupId><artifactId>${opts.parent}</artifactId><version>1.0.0</version></parent>` : '',
    `  <groupId>${opts.group}</groupId>`,
    `  <artifactId>${opts.artifact}</artifactId>`,
    '  <version>1.0.0</version>',
    opts.modules ? '  <packaging>pom</packaging>' : '',
    opts.name ? `  <name>${opts.name}</name>` : '',
    opts.modules ? `  <modules>\n${opts.modules.map((m) => `    <module>${m}</module>`).join('\n')}\n  </modules>` : '',
    '  <dependencies><dependency><groupId>org.projectlombok</groupId><artifactId>lombok</artifactId><version>1.18.34</version></dependency></dependencies>',
    '  <build><plugins><plugin><groupId>org.springframework.boot</groupId><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build>',
    RELEASES('releases'),
    '</project>',
    '',
  ].filter(Boolean).join('\n');
}

function writeReactor(root: string, group: string, artifact: string, name: string | undefined, modules: string[]): void {
  mkdirSync(root, { recursive: true });
  writeFileSync(join(root, 'pom.xml'), pom({ group, artifact, name, modules }));
  for (const m of modules) {
    mkdirSync(join(root, m, 'src', 'main', 'resources'), { recursive: true });
    writeFileSync(join(root, m, 'pom.xml'), pom({ group, artifact: m, parent: artifact }));
    writeFileSync(join(root, m, 'src', 'main', 'resources', 'application.properties'), 'server.port=8080\n');
  }
}

async function createSeed(): Promise<Seed> {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-maven-')));
  const tools = join(dir, 'billing_tools');
  mkdirSync(tools);
  writeFileSync(join(tools, 'pom.xml'), pom({ group: 'com.corp.tools', artifact: 'billing-tools', name: 'Billing Tools' }));
  const sixModules = (p: string) => ['services', 'apis', 'web', 'runner', 'integration', 'integration-runner'].map((m) => `${p}-${m}`);
  writeReactor(join(dir, 'id_generator'), 'com.corp.ids', 'ids-generator', 'Ids Generator', sixModules('ids'));
  writeReactor(join(dir, 'invoices'), 'com.corp.invoices', 'invoices', 'Invoices', sixModules('invoices'));
  // No top-level <name>: the window must not title it after a <distributionManagement> repo.
  writeReactor(join(dir, 'config_client'), 'com.corp.config', 'config-client', undefined, ['config-client-java', 'config-client-spring']);
  const roots = [tools, join(dir, 'id_generator'), join(dir, 'invoices'), join(dir, 'config_client')];

  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: 'Maven multi-root', root_path: tools });
    await daemonCall(c.ctx, c.base, c.token, 'PUT', '/ide/environments', {
      target: { kind: 'local', workspace_id: ws.id, root: tools, connection_id: null },
      additional_roots: roots.slice(1),
      env_vars: [],
    });
    return { dir, roots, workspaceId: ws.id };
  } finally {
    await c.ctx.dispose();
  }
}

async function bootMavenWindow(page: Page, seed: Seed): Promise<void> {
  await page.addInitScript(
    ({ wsId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', 'dark');
      localStorage.setItem('otto_direction', 'ltr');
    },
    { wsId: seed.workspaceId },
  );
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
  await page.getByTestId('ide-more-btn').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Show Maven tool window' }).click();
  const win = page.getByTestId('ide-build-tool-window');
  await expect(win).toBeVisible({ timeout: 15_000 });
  await expect(win).toContainText('Invoices', { timeout: 15_000 });
}

/** DOM mutations inside the window and the layout shift of its first row over `ms`. */
async function watchWindow(page: Page, ms: number): Promise<{ mutations: number; maxShift: number }> {
  return page.evaluate(async (waitMs) => {
    const win = document.querySelector('[data-testid="ide-build-tool-window"]')!;
    let mutations = 0;
    const mo = new MutationObserver((recs) => { mutations += recs.length; });
    mo.observe(win, { subtree: true, childList: true, attributes: true, characterData: true });
    const row = () => win.querySelector('.bt-body .bt-mod-row');
    const y0 = row()?.getBoundingClientRect().top ?? 0;
    let maxShift = 0;
    const t0 = performance.now();
    while (performance.now() - t0 < waitMs) {
      await new Promise((r) => requestAnimationFrame(r));
      maxShift = Math.max(maxShift, Math.abs((row()?.getBoundingClientRect().top ?? 0) - y0));
    }
    mo.disconnect();
    return { mutations, maxShift };
  }, ms);
}

let seed: Seed;

test.describe('Desktop IDE — Maven tool window on a multi-root project', () => {
  test.use({ viewport: { width: 1440, height: 900 }, isMobile: false, hasTouch: false });

  test.beforeAll(async () => {
    seed = await createSeed();
  });

  test.afterAll(() => {
    if (seed?.dir) rmSync(seed.dir, { recursive: true, force: true });
  });

  test('loads once, stays still idle and hovered, and takes clicks', async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop only');
    test.setTimeout(90_000);
    const loopErrors: string[] = [];
    page.on('pageerror', (e) => { if (/effect_update_depth_exceeded/i.test(e.message)) loopErrors.push(e.message); });
    let modelCalls = 0;
    let fsCalls = 0;
    let runPosts = 0;
    page.on('request', (r) => {
      if (r.method() === 'POST' && /\/ide\/runs(\?|$)/.test(r.url())) runPosts++;
      if (r.url().includes('/ide/build-tools/model')) modelCalls++;
      if (/\/ide\/fs\/(file|tree)/.test(r.url())) fsCalls++;
    });

    await bootMavenWindow(page, seed);
    const win = page.getByTestId('ide-build-tool-window');
    const body = win.locator('.bt-body');
    // Four roots, each named after its own pom, never after a deploy repository.
    await expect(body).toContainText('Billing Tools');
    await expect(body).toContainText('Ids Generator');
    await expect(body).toContainText('config-client');
    await expect(body).not.toContainText('Corp Releases');
    if (SHOT_DIR) await page.screenshot({ path: join(SHOT_DIR, 'maven-loaded.png') });

    // Idle with the mouse away from the window: no churn, no reload loop.
    await page.mouse.move(200, 450);
    const callsBefore = modelCalls + fsCalls;
    const idle = await watchWindow(page, 5_000);
    console.log(`[maven] idle: ${JSON.stringify(idle)} model=${modelCalls} fs=${fsCalls}`);
    expect(idle.mutations, 'DOM churn while idle').toBe(0);
    expect(modelCalls + fsCalls - callsBefore, 'requests while idle').toBe(0);

    // Hovering a module row must not move the tree under the pointer.
    const firstRow = body.locator('.bt-row-main', { hasText: 'Ids Generator' }).first();
    const box = (await firstRow.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    const hovered = await watchWindow(page, 2_000);
    console.log(`[maven] hovered: ${JSON.stringify(hovered)}`);
    expect(hovered.maxShift, 'rows shift while hovered').toBe(0);
    expect(hovered.mutations, 'DOM churn while hovered').toBeLessThan(10);

    // Clicks land where the pointer is (a raw mouse click, like a user's: no waiting for the row to settle).
    const invoices = body.locator('.bt-row-main', { hasText: 'Invoices' }).first();
    const clickRow = async () => {
      const b = (await invoices.boundingBox())!;
      await page.mouse.click(b.x + 40, b.y + b.height / 2);
    };
    const wasOpen = (await invoices.getAttribute('aria-expanded')) === 'true';
    await clickRow();
    await expect(invoices).toHaveAttribute('aria-expanded', wasOpen ? 'false' : 'true');
    if (wasOpen) await clickRow();
    await expect(invoices).toHaveAttribute('aria-expanded', 'true');
    const lifecycle = invoices.locator('xpath=ancestor::div[contains(@class,"bt-node")][1]').locator('.bt-folder-row', { hasText: 'Lifecycle' }).first();
    await lifecycle.click();
    await expect(lifecycle).toHaveAttribute('aria-expanded', 'true');
    await expect(win.locator('.bt-goal-main', { hasText: 'package' }).first()).toBeVisible();

    // A double-click on a goal starts it once, not once per click plus once for the dblclick.
    await win.locator('.bt-goal-main', { hasText: 'validate' }).first().dblclick();
    await expect.poll(() => runPosts).toBeGreaterThan(0);
    await page.waitForTimeout(800); // ui-guards: allow — absence: no second or third run starts
    expect(runPosts, 'POST /ide/runs per double-click').toBe(1);

    // Toolbar buttons respond.
    const skip = win.getByTestId('ide-bt-skip-tests');
    const pressed = await skip.getAttribute('aria-pressed');
    await skip.click();
    await expect(skip).not.toHaveAttribute('aria-pressed', pressed ?? 'false');
    await skip.click();
    await win.getByTestId('ide-bt-collapse-all').click();
    await expect(lifecycle).toHaveCount(0);
    const modelBeforeReload = modelCalls;
    await win.getByTestId('ide-bt-reload').click();
    await expect.poll(() => modelCalls).toBe(modelBeforeReload + 1);
    if (SHOT_DIR) await page.screenshot({ path: join(SHOT_DIR, 'maven-after-clicks.png') });

    // Bounded: one model load on open plus the explicit reload.
    console.log(`[maven] total model=${modelCalls} fs=${fsCalls}`);
    expect(modelCalls).toBeLessThanOrEqual(3);
    expect(loopErrors).toEqual([]);
  });
});
