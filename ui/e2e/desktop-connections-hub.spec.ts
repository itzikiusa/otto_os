import { test, expect, type Locator } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { pickMenuItem } from './helpers';

// Desktop BROWSER regression for the Connections list page (#/connections):
// ONE list holds SSH + DB connections AND a Kafka cluster, sections are group
// headers, each row says its state in words, and ONE filter menu (kind,
// section) plus a search narrows it. Seeds real rows via the API against the
// isolated throwaway daemon. Routed to the desktop-browser project.

test.describe('connections hub', () => {
  test.beforeEach(async ({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  });

  test('renders SSH + DB + cluster in one filterable tree', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const wsId = await seedWorkspace(ctx, base);

    // Section FIRST, so the DB connection + cluster can be filed under it.
    const secRes = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connection-sections`, {
      data: { name: 'HUB-SEC' },
    });
    expect(secRes.ok(), `create section → ${secRes.status()} ${await secRes.text()}`).toBeTruthy();
    const sectionId = (await secRes.json()).id as string;
    // A folder with nothing in it: it must stay visible (as a drop target) under
    // every type filter — there is nothing in it to filter out.
    const emptyRes = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connection-sections`, {
      data: { name: 'EMPTY-SEC' },
    });
    expect(emptyRes.ok(), `create empty section → ${emptyRes.status()}`).toBeTruthy();

    // ssh — ungrouped, no secret.
    const sshRes = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, {
      data: {
        name: 'hub-ssh',
        kind: 'ssh',
        params: { host: 'ssh.example.com', user: 'deploy' },
        secret: null,
        environment: 'dev',
        read_only: false,
      },
    });
    expect(sshRes.ok(), `create ssh → ${sshRes.status()} ${await sshRes.text()}`).toBeTruthy();

    // mysql — filed under HUB-SEC at create time, no password. section_id on a
    // connection is a normal field, so this lands in the shared tree on the old
    // daemon too (the reliable "row under section" assertion).
    const myRes = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, {
      data: {
        name: 'hub-mysql',
        kind: 'mysql',
        params: { host: '127.0.0.1', port: 3306, user: 'root', db: 'app' },
        secret: null,
        section_id: sectionId,
        environment: 'dev',
        read_only: false,
      },
    });
    expect(myRes.ok(), `create mysql → ${myRes.status()} ${await myRes.text()}`).toBeTruthy();

    // A PLAINTEXT cluster — never connected, just listed.
    const clRes = await ctx.post(`${base}/api/v1/workspaces/${wsId}/brokers/clusters`, {
      data: {
        name: 'hub-kafka',
        bootstrap_servers: '127.0.0.1:19099',
        security_protocol: 'plaintext',
        sasl_mechanism: null,
        sasl_username: null,
        tls_skip_verify: false,
        schema_registry_url: null,
        schema_registry_username: null,
        metrics_url: null,
        color: null,
        environment: 'dev',
        read_only: false,
      },
    });
    expect(clRes.ok(), `create cluster → ${clRes.status()} ${await clRes.text()}`).toBeTruthy();
    const clusterId = (await clRes.json()).id as string;

    // File the cluster under HUB-SEC via the cluster PATCH, mirroring the
    // store's moveCluster: UpsertClusterReq REQUIRES name + bootstrap_servers
    // (a section_id-only body 422s), omitted secrets/ssh are kept by PATCH
    // semantics. On a pre-migration daemon the shared-tree section id isn't in
    // broker_cluster_sections, so the FK write can fail — detect and skip.
    let clusterUnderSection = false;
    try {
      const patch = await ctx.patch(`${base}/api/v1/brokers/clusters/${clusterId}`, {
        data: {
          name: 'hub-kafka',
          bootstrap_servers: '127.0.0.1:19099',
          security_protocol: 'plaintext',
          environment: 'dev',
          read_only: false,
          section_id: sectionId,
        },
      });
      if (patch.ok()) {
        const list = await ctx.get(`${base}/api/v1/workspaces/${wsId}/brokers/clusters`);
        const arr = (await list.json()) as { id: string; section_id?: string | null }[];
        clusterUnderSection = arr.find((c) => c.id === clusterId)?.section_id === sectionId;
      }
    } catch {
      clusterUnderSection = false;
    }
    await ctx.dispose();

    // ---- Drive the hub UI ----------------------------------------------------
    await page.addInitScript((w) => {
      localStorage.setItem('otto_workspace', w as string);
      localStorage.setItem('otto_connhub_filter', 'all'); // chip choice persists — pin it
    }, wsId);
    await page.goto('/#/connections');
    await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

    const sshRow = page.locator('.conn-row', { hasText: 'hub-ssh' });
    const mysqlRow = page.locator('.conn-row', { hasText: 'hub-mysql' });
    const clusterRow = page.locator('.conn-row', { hasText: 'hub-kafka' });
    const hubSec = page.getByRole('region', { name: 'HUB-SEC' }).last();
    const emptySec = page.getByRole('region', { name: 'EMPTY-SEC' }).last();
    const ungrouped = page.getByRole('region', { name: 'Ungrouped' });

    // (1) one list: every kind, sections as group headers, rows in words.
    await expect(sshRow).toBeVisible();
    await expect(mysqlRow).toBeVisible();
    await expect(clusterRow).toBeVisible();
    await expect(hubSec.locator('.conn-row', { hasText: 'hub-mysql' })).toBeVisible();
    await expect(ungrouped.locator('.conn-row', { hasText: 'hub-ssh' })).toBeVisible();
    if (clusterUnderSection) await expect(hubSec.locator('.conn-row', { hasText: 'hub-kafka' })).toBeVisible();
    await expect(emptySec).toContainText('No connections in this section.');
    // Line one: name … the state WORD (never tested just because it rendered);
    // line two: kind · host · section · last opened.
    await expect(mysqlRow).toContainText('Not checked');
    // Under the HUB-SEC heading the row does not repeat the section's name.
    await expect(mysqlRow).toContainText('MySQL · 127.0.0.1:3306 · never opened');
    await expect(sshRow).toContainText('SSH · ssh.example.com');

    // (2) ONE filter menu (no chip rows): Kafka → only the cluster; the
    //     filter shows as one removable chip.
    await page.getByTestId('conn-filter').click();
    await page.getByRole('menuitemcheckbox', { name: 'Kafka', exact: true }).click();
    await expect(clusterRow).toBeVisible();
    await expect(sshRow).toHaveCount(0);
    await expect(mysqlRow).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'EMPTY-SEC' })).toHaveCount(0);

    // (3) SSH → only the ssh row; then the chip clears the filter.
    await page.getByTestId('conn-filter').click();
    await page.getByRole('menuitemcheckbox', { name: 'SSH', exact: true }).click();
    await expect(sshRow).toBeVisible();
    await expect(clusterRow).toHaveCount(0);
    await expect(mysqlRow).toHaveCount(0);
    await page.getByRole('button', { name: 'Clear the SSH filter' }).click();
    await expect(mysqlRow).toBeVisible();
    await expect(clusterRow).toBeVisible();

    // (4) Search narrows by name / host / section.
    await page.getByRole('searchbox', { name: 'Search connections' }).fill('hub-my');
    await expect(mysqlRow).toBeVisible();
    await expect(sshRow).toHaveCount(0);
    await page.getByRole('searchbox', { name: 'Search connections' }).fill('');

    // (5) The row ⋯ holds the row actions; Test runs on demand only.
    await mysqlRow.hover();
    await page.getByRole('button', { name: 'Actions for hub-mysql' }).click();
    for (const item of ['Open', 'Test', 'Edit', 'Duplicate without password', 'Delete']) {
      await expect(page.getByRole('menuitem', { name: item, exact: true })).toBeVisible();
    }
    await page.keyboard.press('Escape');
  });

  test('loading is said in words, never "0 saved"', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const wsId = await seedWorkspace(ctx, base);
    await ctx.dispose();
    let release: () => void = () => {};
    const held = new Promise<void>((r) => (release = r));
    await page.route(/\/workspaces\/[^/]+\/connections$/, async (route) => {
      await held;
      await route.continue();
    });
    await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), wsId);
    await page.goto('/#/connections');
    await expect(page.getByRole('status', { name: 'Loading connections' })).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText(/\b0 saved\b/)).toHaveCount(0);
    release();
    await expect(page.getByText(/\d+ saved/)).toBeVisible();
  });

  test('Delete hides the row at once and Undo brings it back without deleting', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const wsId = await seedWorkspace(ctx, base);
    const name = `undo-me-${Date.now()}`;
    const res = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, {
      data: { name, kind: 'ssh', params: { host: 'undo.invalid' }, secret: null },
    });
    expect(res.ok()).toBeTruthy();
    const { id } = await res.json();
    let deletes = 0;
    page.on('request', (r) => {
      if (r.method() === 'DELETE' && r.url().endsWith(`/connections/${id}`)) deletes++;
    });
    await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), wsId);
    await page.goto('/#/connections');
    const row = page.locator('.conn-row', { hasText: name });
    await pickMenuItem(page, () => row.click({ button: 'right' }), 'Delete');
    await expect(row).toHaveCount(0);
    await page.getByRole('button', { name: 'Undo', exact: true }).click();
    await expect(row).toBeVisible();
    await page.waitForTimeout(7000); // ui-guards: allow — proves NO delete is sent after the toast is gone
    expect(deletes).toBe(0);
    const still = await ctx.get(`${base}/api/v1/workspaces/${wsId}/connections`);
    expect(((await still.json()) as { id: string }[]).some((c) => c.id === id)).toBe(true);
    await ctx.dispose();
  });
});
