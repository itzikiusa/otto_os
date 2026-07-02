import { test, expect, type Locator } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Desktop BROWSER regression for the unified Connections hub (the DB-workbench
// page serving as the single Connections surface): ONE section tree
// holds SSH + DB connections AND a Kafka cluster, and the type-filter chips
// narrow it — hiding non-matching rows AND sections with no matching descendant.
// Seeds real rows via the API against the isolated throwaway daemon; asserts
// rendering + filtering only (opens route to other modules and are out of scope).
//
// NOTE: the throwaway E2E daemon is the INSTALLED ottod (pre-migration): it may
// still keep broker sections in a SEPARATE tree, so a cluster's section_id might
// not land in the shared connection-section tree. We detect that and skip ONLY
// the cluster-under-HUB-SEC placement assertion; the filter assertions run either
// way. The file is routed to the desktop-browser project (testMatch on
// desktop-*.spec.ts) and self-skips on the mobile/tablet device projects.

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
    const hubSec = page.locator('.sec-head', { hasText: 'HUB-SEC' });
    const ungrouped = page.locator('.sec-head.plain', { hasText: 'Ungrouped' });
    const y = async (l: Locator): Promise<number> => (await l.first().boundingBox())!.y;

    const ran: string[] = [];
    const skipped: string[] = [];

    // (1) all three rows visible in one tree.
    await expect(sshRow).toBeVisible();
    await expect(mysqlRow).toBeVisible();
    await expect(clusterRow).toBeVisible();
    await expect(hubSec).toBeVisible();
    ran.push('all-three-visible');

    // mysql sits under HUB-SEC (above the Ungrouped group); ssh is ungrouped.
    expect(await y(mysqlRow)).toBeGreaterThan(await y(hubSec));
    expect(await y(mysqlRow)).toBeLessThan(await y(ungrouped));
    expect(await y(sshRow)).toBeGreaterThan(await y(ungrouped));
    ran.push('mysql-under-HUB-SEC', 'ssh-ungrouped');

    if (clusterUnderSection) {
      expect(await y(clusterRow)).toBeGreaterThan(await y(hubSec));
      expect(await y(clusterRow)).toBeLessThan(await y(ungrouped));
      ran.push('cluster-under-HUB-SEC');
    } else {
      skipped.push('cluster-under-HUB-SEC (needs the new shared-tree daemon)');
    }

    // (2) Kafka chip → only the cluster; ssh + mysql hidden. A section shows only
    //     if it holds a matching descendant.
    await page.locator('[data-testid="connhub-filter-kafka"]').click();
    await expect(clusterRow).toBeVisible();
    await expect(sshRow).toHaveCount(0);
    await expect(mysqlRow).toHaveCount(0);
    if (clusterUnderSection) await expect(hubSec).toBeVisible();
    else await expect(hubSec).toHaveCount(0);
    ran.push('kafka-filter');

    // (3) SSH chip → only the ssh row; cluster + mysql hidden, HUB-SEC hidden
    //     (no ssh under it → zero matching descendants).
    await page.locator('[data-testid="connhub-filter-ssh"]').click();
    await expect(sshRow).toBeVisible();
    await expect(clusterRow).toHaveCount(0);
    await expect(mysqlRow).toHaveCount(0);
    await expect(hubSec).toHaveCount(0);
    ran.push('ssh-filter');

    // (4) All restores every row.
    await page.locator('[data-testid="connhub-filter-all"]').click();
    await expect(sshRow).toBeVisible();
    await expect(mysqlRow).toBeVisible();
    await expect(clusterRow).toBeVisible();
    ran.push('all-restores');

    // eslint-disable-next-line no-console
    console.log(`[connhub] assertions RAN: ${ran.join(', ')}`);
    if (skipped.length) {
      // eslint-disable-next-line no-console
      console.log(`[connhub] assertions SKIPPED: ${skipped.join(', ')}`);
    }
  });
});
