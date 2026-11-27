import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, realpathSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx, daemonInfo } from './seed';

// Real Jupyter kernels in the notebook view (rc.6 design A3, J1/J2/J3/J5/J6/J12/J14/J15).
// Needs a throwaway project whose `.venv` has ipykernel + matplotlib, and the
// daemon started with HOME pointing at a throwaway home holding the bash_kernel
// spec (`Library/Jupyter/kernels/bash`):
//   OTTO_E2E_JUPYTER_PROJECT=<proj with .venv> HOME=<throwaway home> npx playwright test desktop-ide-notebook-jupyter
// Without OTTO_E2E_JUPYTER_PROJECT every test skips (CI has no kernels).

const PROJECT = process.env.OTTO_E2E_JUPYTER_PROJECT ?? '';

function notebook(spec: { name: string; language: string; display_name: string }, sources: string[]): string {
  return JSON.stringify({
    nbformat: 4,
    nbformat_minor: 5,
    metadata: { kernelspec: spec },
    cells: sources.map((s, i) => ({ id: `c${i}`, cell_type: 'code', metadata: {}, execution_count: null, outputs: [], source: [s] })),
  });
}

let wsId = '';
async function seed(): Promise<string> {
  if (wsId) return wsId;
  const root = realpathSync(PROJECT);
  writeFileSync(
    join(root, 'py.ipynb'),
    notebook({ name: 'python3', language: 'python', display_name: 'Python 3' }, [
      '%matplotlib inline\nimport matplotlib.pyplot as plt\nplt.plot([1, 2])',
      'import time\nfor i in range(4):\n    print(i)\n    time.sleep(1)',
      "x = input('name? ')",
      'x',
      '1 / 0',
    ]),
  );
  writeFileSync(join(root, 'runall.ipynb'), notebook({ name: 'python3', language: 'python', display_name: 'Python 3' }, ["print('one')", "print('two')", "print('three')"]));
  writeFileSync(join(root, 'bash.ipynb'), notebook({ name: 'bash', language: 'bash', display_name: 'Bash' }, ['echo hi-from-bash']));
  writeFileSync(join(root, 'proj.ipynb'), notebook({ name: 'proj-k', language: 'python', display_name: 'Project K' }, ['import os\nos.environ["OTTO_PROJ_K"]']));
  // A kernelspec stored inside the project: asks once before it starts.
  const pk = join(root, '.venv/share/jupyter/kernels/proj-k');
  mkdirSync(pk, { recursive: true });
  writeFileSync(
    join(pk, 'kernel.json'),
    JSON.stringify({
      argv: [join(root, '.venv/bin/python'), '-m', 'ipykernel_launcher', '--IPKernelApp.matplotlib=inline', '-f', '{connection_file}'],
      display_name: 'Project K',
      language: 'python',
      env: { OTTO_PROJ_K: 'yes' },
    }),
  );
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `Jupyter ${Date.now()}`, root_path: root });
    wsId = ws.id;
  } finally {
    await c.ctx.dispose();
  }
  return wsId;
}

async function boot(page: Page, scheme: 'dark' | 'light'): Promise<void> {
  const id = await seed();
  const root = realpathSync(PROJECT);
  await page.addInitScript(
    ({ wsId, root, uiScheme }) => {
      if (sessionStorage.getItem('__ide_nb_init')) return;
      sessionStorage.setItem('__ide_nb_init', '1');
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', uiScheme);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.preferences.v1', JSON.stringify({ autoSave: false }));
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-nb', name: 'NB', folders: [{ path: root, name: 'proj' }], updatedAt: Date.now() }]));
    },
    { wsId: id, root, uiScheme: scheme },
  );
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
}

async function openNotebook(page: Page, name: string): Promise<void> {
  // A fresh IDE auto-opens the project's first file (bash.ipynb): let that land
  // first, or it takes the editor over in the middle of the test.
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab').first()).toBeVisible({ timeout: 20_000 });
  const f = page.getByTestId('ide-project-tree').locator(`button.tree-item[title="${name}"], button.tree-item[title$="/${name}"]`).first();
  await expect(f).toBeVisible({ timeout: 15_000 });
  await f.dblclick();
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText(name, { timeout: 15_000 });
  await expect(page.getByTestId('ide-notebook-view')).toBeVisible({ timeout: 15_000 });
}

const select = (page: Page) => page.getByTestId('ide-nb-kernel-select');
const outputs = (page: Page, i: number) => page.getByTestId(`ide-nb-outputs-${i}`);

function daemonPid(): string {
  const port = new URL(daemonInfo().base).port;
  return execFileSync('lsof', ['-nP', `-iTCP:${port}`, '-sTCP:LISTEN', '-t']).toString().trim().split('\n')[0];
}

function children(pid: string): string[] {
  try {
    return execFileSync('pgrep', ['-P', pid]).toString().trim().split('\n').filter(Boolean);
  } catch {
    return [];
  }
}

test.describe('Jupyter kernels in the notebook view', () => {
  test.skip(!PROJECT, 'set OTTO_E2E_JUPYTER_PROJECT to a throwaway project with an ipykernel .venv');

  test('J1: listing kernels reads kernel.json files only — no process spawned', async () => {
    await seed();
    const c = await apiCtx();
    try {
      const body = { target: { kind: 'local', root: realpathSync(PROJECT) }, notebook_path: join(realpathSync(PROJECT), 'py.ipynb') };
      // The first call probes GPU / MPS once per daemon (sysctl, cached): warm it.
      await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/notebooks/kernels', body);
      const pid = daemonPid();
      const before = children(pid);
      const resp = await daemonCall<{ kernels: { id: string; kind: string; display_name: string; origin?: string; needs_trust?: boolean }[] }>(
        c.ctx, c.base, c.token, 'POST', '/ide/notebooks/kernels',
        { target: { kind: 'local', root: realpathSync(PROJECT) }, notebook_path: join(realpathSync(PROJECT), 'py.ipynb') },
      );
      const after = children(pid);
      console.log('J1 kernels:', JSON.stringify(resp.kernels.map((k) => [k.kind, k.display_name, k.origin, k.needs_trust])));
      console.log(`J1 daemon ${pid} children before ${before.length}, after ${after.length}`);
      expect(after.length).toBe(before.length);
      expect(resp.kernels.some((k) => k.kind === 'jupyter' && k.display_name.endsWith('(.venv) · ipykernel'))).toBe(true);
      expect(resp.kernels.some((k) => k.kind === 'jupyter' && k.display_name === 'Bash' && k.origin === 'user')).toBe(true);
      expect(resp.kernels.some((k) => k.display_name === 'Project K' && k.origin === 'workspace' && k.needs_trust)).toBe(true);
    } finally {
      await c.ctx.dispose();
    }
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`J2/J3/J5/J6/J15: kernel select, chip, streamed outputs, input() (${scheme})`, async ({ page }) => {
      test.setTimeout(180_000);
      await page.setViewportSize({ width: 1440, height: 900 });
      await boot(page, scheme);
      await openNotebook(page, 'py.ipynb');
      // J2: metadata.kernelspec.name "python3" opens on the env's ipykernel; grouped select.
      await expect(select(page)).toBeEnabled({ timeout: 20_000 });
      await expect(select(page).locator('optgroup[label="Jupyter kernels"] option', { hasText: '(.venv) · ipykernel' })).toHaveCount(1);
      const chosen = await select(page).evaluate((el) => (el as HTMLSelectElement).selectedOptions[0]?.textContent ?? '');
      expect(chosen).toMatch(/\(\.venv\) · ipykernel$/);
      await expect(select(page)).toHaveAttribute('title', /ipykernel_launcher -f/);
      // Debug Cell on a Jupyter kernel is its own (cut) gate: disabled, says why.
      await expect(page.getByTestId('ide-nb-debug-0')).toBeDisabled();
      await expect(page.getByTestId('ide-nb-debug-0')).toHaveAttribute('title', 'Debugging Jupyter kernels isn’t available yet');

      // J3: %matplotlib inline plot → an inline PNG.
      await page.getByTestId('ide-nb-run-0').click();
      await expect(outputs(page, 0).locator('img.nb-out-img')).toBeVisible({ timeout: 60_000 });
      await expect(page.getByTestId('ide-nb-kernel-status')).toHaveText('idle', { timeout: 15_000 });

      // J5: the first line streams in while the cell still runs.
      await page.getByTestId('ide-nb-run-1').click();
      await expect(outputs(page, 1)).toContainText('0', { timeout: 3_000 });
      await expect(page.getByTestId('ide-nb-kernel-status')).toHaveText('busy');
      await expect(page.getByTestId('ide-nb-run-1')).toBeDisabled();
      await expect(outputs(page, 1)).toContainText('3', { timeout: 10_000 });
      await expect(page.getByTestId('ide-nb-run-1')).toBeEnabled({ timeout: 10_000 });

      // J6: input() asks under the cell; Enter answers it.
      await page.getByTestId('ide-nb-run-2').click();
      const field = page.getByTestId('ide-nb-input').locator('input');
      await expect(field).toBeVisible({ timeout: 15_000 });
      await expect(page.getByTestId('ide-nb-input')).toContainText('name?');
      await expect(field).toBeFocused();
      await field.fill('ada');
      await field.press('Enter');
      await expect(page.getByTestId('ide-nb-input')).toHaveCount(0);
      await page.getByTestId('ide-nb-run-3').click();
      await expect(outputs(page, 3)).toContainText("'ada'", { timeout: 15_000 });
      // ANSI traceback keeps its colours on token classes.
      await page.getByTestId('ide-nb-run-4').click();
      await expect(outputs(page, 4).locator('.ansi-danger').first()).toBeVisible({ timeout: 15_000 });
      await page.screenshot({ path: test.info().outputPath(`notebook-ipykernel-${scheme}.png`) });

      // Phone width: the toolbar wraps, nothing scrolls sideways.
      await page.setViewportSize({ width: 390, height: 844 });
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: test.info().outputPath(`notebook-ipykernel-phone-${scheme}.png`) });
    });
  }

  test('Run All runs every cell (a committed output never resets the run)', async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await boot(page, 'dark');
    await openNotebook(page, 'runall.ipynb');
    await expect(select(page)).toBeEnabled({ timeout: 20_000 });
    await page.getByTestId('ide-nb-run-all').click();
    await expect(outputs(page, 2)).toContainText('three', { timeout: 60_000 });
    await expect(outputs(page, 0)).toContainText('one');
    await expect(outputs(page, 1)).toContainText('two');
  });

  test('J10/J15: Bash kernel from the user dir; Debug Cell says it can’t debug', async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await boot(page, 'dark');
    await openNotebook(page, 'bash.ipynb');
    await expect(select(page)).toBeEnabled({ timeout: 20_000 });
    const chosen = await select(page).evaluate((el) => (el as HTMLSelectElement).selectedOptions[0]?.textContent ?? '');
    expect(chosen).toBe('Bash');
    await expect(page.getByTestId('ide-nb-debug-0')).toHaveAttribute('title', 'This kernel doesn’t support debugging');
    await page.getByTestId('ide-nb-run-0').click();
    await expect(outputs(page, 0)).toContainText('hi-from-bash', { timeout: 60_000 });
    await page.screenshot({ path: test.info().outputPath('notebook-bash-dark.png') });
  });

  test('J12: a project kernelspec asks once; declining starts nothing', async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await boot(page, 'dark');
    await openNotebook(page, 'proj.ipynb');
    await expect(select(page)).toBeEnabled({ timeout: 20_000 });
    const pid = daemonPid();
    const before = children(pid).length;
    await page.getByTestId('ide-nb-run-0').click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText("Start kernel 'Project K' from this project? It runs:");
    await expect(dialog).toContainText('--IPKernelApp.matplotlib=inline');
    await page.screenshot({ path: test.info().outputPath('notebook-trust-prompt.png') });
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);
    await page.waitForTimeout(1500); // ui-guards: allow — proves nothing starts after Cancel
    expect(children(pid).length).toBe(before);
    await page.getByTestId('ide-nb-run-0').click();
    await page.getByRole('dialog').getByRole('button', { name: 'Start kernel' }).click();
    await expect(outputs(page, 0)).toContainText("'yes'", { timeout: 60_000 });
    // A second start (Restart) doesn't ask again.
    await page.getByRole('button', { name: 'Restart kernel' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByTestId('ide-nb-kernel-status')).toHaveText('idle', { timeout: 30_000 });
  });

  test('J14: 12 notebooks run, the 13th is refused; closing one frees its kernel group', async () => {
    test.setTimeout(240_000);
    await seed();
    const root = realpathSync(PROJECT);
    const c = await apiCtx();
    const target = { kind: 'local', root };
    const path = (i: number) => join(root, `cap-${i}.ipynb`);
    try {
      // Start from no kernels: the UI tests above leave theirs running (no tab closed).
      for (const nb of ['py.ipynb', 'bash.ipynb', 'proj.ipynb', 'runall.ipynb']) {
        await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/notebooks/shutdown', { target, notebook_path: join(root, nb) });
      }
      for (let i = 0; i < 12; i++) {
        const r = await daemonCall<{ status: string }>(c.ctx, c.base, c.token, 'POST', '/ide/notebooks/execute', { target, notebook_path: path(i), code: '1' });
        expect(r.status).toBe('ok');
      }
      const r13 = await c.ctx.post(`${c.base}/api/v1/ide/notebooks/execute`, { data: { target, notebook_path: path(12), code: '1' } });
      expect(r13.status()).toBe(409);
      expect(await r13.text()).toContain('12 kernels are running. Close a notebook to start another.');
      const pid = daemonPid();
      const kernelsBefore = children(pid);
      console.log(`J14 daemon children with 12 kernels: ${kernelsBefore.length}`);
      // ~0 % CPU while idle.
      await new Promise((r) => setTimeout(r, 60_000));
      const cpu = execFileSync('ps', ['-o', '%cpu=', '-p', kernelsBefore.join(',')]).toString().trim().split(/\s+/).map(Number);
      console.log(`J14 idle CPU over 60 s per kernel: ${cpu.join(' ')}`);
      expect(Math.max(...cpu)).toBeLessThan(2);
      await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/notebooks/shutdown', { target, notebook_path: path(0) });
      await expect.poll(() => children(pid).length, { timeout: 5_000 }).toBe(kernelsBefore.length - 1);
    } finally {
      for (let i = 0; i < 13; i++) {
        await c.ctx.post(`${c.base}/api/v1/ide/notebooks/shutdown`, { data: { target, notebook_path: path(i) } });
      }
      await c.ctx.dispose();
    }
  });
});
