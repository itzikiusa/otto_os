import { test, expect, type Page, type Route } from '@playwright/test';
import {
  expectContentHasHeight,
  expectFullyInViewport,
  expectNoHorizontalOverflow,
  openPage,
} from './helpers';

// Real Playwright E2E coverage for Otto's native Containers module:
//   - Offline daemon + 1-click "Install & Start Kiri" flow + fallback context switcher
//   - Online Compose stack grouping, auto-selection of first running container
//   - Overview env secret masking + click-to-reveal, Logs find, Stats cards
//   - Images / Volumes / Networks tabs, Run container modal, viewport-clamped ⋯ menu
//   - Responsive desktop, tablet, and phone (push nav) + light/dark parity + zero overflow

test.use({ serviceWorkers: 'block' });

const SAMPLE_CONTAINERS = [
  {
    id: 'ctr-payments-api-111111111111',
    short_id: 'ctr-payments',
    name: 'payments-api',
    image: 'ghcr.io/acme/payments:1.4',
    image_id: 'sha256:aaa111',
    command: 'node server.js',
    created: 1_760_000_200,
    state: 'running',
    status: 'Up 3 hours (healthy)',
    health: 'healthy',
    compose_project: 'payments',
    compose_service: 'api',
    ports: [
      { ip: '0.0.0.0', private_port: 3000, public_port: 8080, protocol: 'tcp', url: 'http://localhost:8080' },
    ],
    labels: { 'com.docker.compose.project': 'payments', 'com.docker.compose.service': 'api' },
    networks: ['payments_default'],
    mounts_count: 1,
  },
  {
    id: 'ctr-payments-db-222222222222',
    short_id: 'ctr-db-22222',
    name: 'payments-db',
    image: 'postgres:16-alpine',
    image_id: 'sha256:bbb222',
    command: 'postgres',
    created: 1_760_000_100,
    state: 'exited',
    status: 'Exited (0) 2 hours ago',
    health: null,
    compose_project: 'payments',
    compose_service: 'db',
    ports: [],
    labels: { 'com.docker.compose.project': 'payments', 'com.docker.compose.service': 'db' },
    networks: ['payments_default'],
    mounts_count: 1,
  },
  {
    id: 'ctr-redis-solo-333333333333',
    short_id: 'ctr-redis-33',
    name: 'redis-cache',
    image: 'redis:7-alpine',
    image_id: 'sha256:ccc333',
    command: 'redis-server',
    created: 1_760_000_050,
    state: 'running',
    status: 'Up 1 hour',
    health: null,
    compose_project: null,
    compose_service: null,
    ports: [
      { ip: '127.0.0.1', private_port: 6379, public_port: 6379, protocol: 'tcp', url: 'http://127.0.0.1:6379' },
    ],
    labels: {},
    networks: ['bridge'],
    mounts_count: 0,
  },
];

function makeDetail(revealSecrets: boolean) {
  return {
    id: SAMPLE_CONTAINERS[0].id,
    short_id: SAMPLE_CONTAINERS[0].short_id,
    name: SAMPLE_CONTAINERS[0].name,
    image: SAMPLE_CONTAINERS[0].image,
    image_id: SAMPLE_CONTAINERS[0].image_id,
    created: new Date(Date.now() - 3 * 3600_000).toISOString(),
    state: 'running',
    status: 'Up 3 hours (healthy)',
    running: true,
    paused: false,
    restarting: false,
    oom_killed: false,
    pid: 4219,
    exit_code: 0,
    started_at: new Date(Date.now() - 3 * 3600_000).toISOString(),
    finished_at: null,
    health: 'healthy',
    restart_policy: 'unless-stopped',
    entrypoint: ['docker-entrypoint.sh'],
    cmd: ['node', 'server.js'],
    working_dir: '/app',
    hostname: 'payments-api',
    compose_project: 'payments',
    compose_service: 'api',
    ports: SAMPLE_CONTAINERS[0].ports,
    env: [
      { key: 'NODE_ENV', value: 'development', secret: false, masked: false },
      {
        key: 'DATABASE_PASSWORD',
        value: revealSecrets ? 's3cr3t-pass-42' : '••••••••',
        secret: true,
        masked: !revealSecrets,
      },
    ],
    mounts: [
      { mount_type: 'volume', source: 'payments_data', destination: '/var/lib/app', mode: 'rw', rw: true },
    ],
    networks: [
      { name: 'payments_default', network_id: 'net-1', ip_address: '172.24.0.3', gateway: '172.24.0.1', mac_address: '02:42:ac:18:00:03' },
    ],
    labels: SAMPLE_CONTAINERS[0].labels,
  };
}

function makeContexts(count = 2) {
  const base = [
    {
      name: 'kiri',
      socket_path: '/Users/dev/.kiri/run/docker.sock',
      host: 'unix:///Users/dev/.kiri/run/docker.sock',
      engine_kind: 'kiri',
      is_active: true,
      is_docker_default: false,
      reachable: true,
    },
    {
      name: 'colima',
      socket_path: '/Users/dev/.colima/default/docker.sock',
      host: 'unix:///Users/dev/.colima/default/docker.sock',
      engine_kind: 'colima',
      is_active: false,
      is_docker_default: false,
      reachable: true,
    },
  ];
  for (let i = base.length; i < count; i++) {
    base.push({
      name: `context-${i}`,
      socket_path: `/tmp/docker-${i}.sock`,
      host: `unix:///tmp/docker-${i}.sock`,
      engine_kind: 'custom',
      is_active: false,
      is_docker_default: false,
      reachable: i % 2 === 0,
    });
  }
  return {
    active_context: 'kiri',
    active_socket: '/Users/dev/.kiri/run/docker.sock',
    contexts: base,
  };
}

async function mockContainersApi(
  page: Page,
  opts: { initialOffline?: boolean; manyContexts?: number } = {},
): Promise<{ installCalls: () => number }> {
  let online = !opts.initialOffline;
  let installed = !opts.initialOffline;
  let installCalls = 0;

  await page.route('**/api/v1/containers**', async (route: Route) => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname.replace(/^\/api\/v1/, '');

    const json = (body: unknown, status = 200) =>
      route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

    if (path === '/containers/status') {
      return json({
        available: online,
        error: online ? null : 'Docker socket unreachable',
        active_context: 'kiri',
        socket_path: '/Users/dev/.kiri/run/docker.sock',
        engine_kind: 'kiri',
        engine_version: online ? '0.2.1' : null,
        api_version: online ? '1.47' : null,
        containers_running: online ? 2 : 0,
        containers_paused: 0,
        containers_stopped: online ? 1 : 0,
        images_count: online ? 2 : 0,
        docker_cli_available: true,
        vm: online
          ? { state: 'running', cpus: 4, memory_used_bytes: 128 * 1024 * 1024, memory_ceiling_bytes: 2048 * 1024 * 1024 }
          : null,
      });
    }

    if (path === '/containers/runtime') {
      return json({
        runtime: 'kiri',
        installed,
        binary_path: installed ? '/Users/dev/Library/Application Support/Otto/bin/kiri' : null,
        binary_source: installed ? 'data_dir' : null,
        running: online,
        daemon_reachable: online,
        vm_state: online ? 'running' : 'offline',
        socket_path: '/Users/dev/.kiri/run/docker.sock',
        managed_by_otto: online,
        can_start: installed && !online,
        can_stop: online,
        can_install: !installed,
        installing: false,
        guest_assets_present: installed,
        log_path: '/Users/dev/Library/Application Support/Otto/logs/kiri.log',
        entitlement_note: 'Embedded sidecar signed with com.apple.security.virtualization',
      });
    }

    if (path === '/containers/runtime/install' && req.method() === 'POST') {
      installCalls += 1;
      installed = true;
      online = true;
      return json({
        runtime: 'kiri',
        installed: true,
        binary_path: '/Users/dev/Library/Application Support/Otto/bin/kiri',
        binary_source: 'data_dir',
        running: true,
        daemon_reachable: true,
        vm_state: 'running',
        socket_path: '/Users/dev/.kiri/run/docker.sock',
        managed_by_otto: true,
        can_start: false,
        can_stop: true,
        can_install: false,
        installing: false,
        guest_assets_present: true,
        log_path: '/Users/dev/Library/Application Support/Otto/logs/kiri.log',
        entitlement_note: 'Embedded sidecar signed with com.apple.security.virtualization',
      });
    }

    if (path === '/containers/contexts') {
      return json(makeContexts(opts.manyContexts ?? 2));
    }

    if (path === '/containers' && req.method() === 'GET') {
      return json({
        containers: SAMPLE_CONTAINERS,
        stacks: [
          {
            project: 'payments',
            running: 1,
            total: 2,
            container_ids: [SAMPLE_CONTAINERS[0].id, SAMPLE_CONTAINERS[1].id],
          },
        ],
      });
    }

    if (path.startsWith('/containers/') && path.endsWith('/logs')) {
      return json({
        container_id: SAMPLE_CONTAINERS[0].id,
        lines: [
          { stream: 'stdout', timestamp: '2026-11-19T12:00:00Z', text: 'Payments API listening on :3000' },
          { stream: 'stderr', timestamp: '2026-11-19T12:00:02Z', text: 'WARN retrying ledger sync' },
        ],
        truncated: false,
      });
    }

    if (path.startsWith('/containers/') && path.endsWith('/stats')) {
      return json({
        container_id: SAMPLE_CONTAINERS[0].id,
        read_at: new Date().toISOString(),
        cpu_percent: 4.2,
        memory_usage_bytes: 64 * 1024 * 1024,
        memory_limit_bytes: 512 * 1024 * 1024,
        memory_percent: 12.5,
        net_rx_bytes: 1024 * 420,
        net_tx_bytes: 1024 * 180,
        block_read_bytes: 1024 * 64,
        block_write_bytes: 1024 * 16,
        pids: 14,
      });
    }

    if (path === '/containers/images') {
      return json({
        images: [
          {
            id: 'sha256:aaa111222333444',
            short_id: 'aaa111222333',
            repo_tags: ['ghcr.io/acme/payments:1.4'],
            repo_digests: ['ghcr.io/acme/payments@sha256:aaa111'],
            created: 1_760_000_000,
            size_bytes: 185 * 1024 * 1024,
            containers: 1,
            dangling: false,
            in_use: true,
          },
        ],
      });
    }

    if (path.startsWith('/containers/images/') && req.method() === 'GET') {
      // Image inspect: the fallback hint for images not in the curated table.
      return json({ id: 'sha256:aaa111222333444', exposed_ports: ['3000/tcp', '9229/udp'] });
    }

    if (path === '/containers/volumes') {
      return json({
        volumes: [
          {
            name: 'payments_data',
            driver: 'local',
            mountpoint: '/var/lib/docker/volumes/payments_data/_data',
            created_at: new Date(Date.now() - 86_400_000).toISOString(),
            scope: 'local',
            labels: {},
            size_bytes: 48 * 1024 * 1024,
            ref_count: 1,
            in_use: true,
          },
        ],
      });
    }

    if (path === '/containers/networks') {
      return json({
        networks: [
          {
            id: 'net-payments-111111',
            short_id: 'net-payments',
            name: 'payments_default',
            driver: 'bridge',
            scope: 'local',
            internal: false,
            attachable: true,
            built_in: false,
            subnet: '172.24.0.0/16',
            gateway: '172.24.0.1',
            containers: [{ id: SAMPLE_CONTAINERS[0].id, name: 'payments-api', ipv4_address: '172.24.0.3/16' }],
            labels: {},
          },
        ],
      });
    }

    if (/^\/containers\/[^/]+$/.test(path) && req.method() === 'GET') return json(makeDetail(false));
    if (/^\/containers\/[^/]+\/secrets\/reveal$/.test(path) && req.method() === 'POST') return json(makeDetail(true));

    return route.continue();
  });

  return { installCalls: () => installCalls };
}

test.describe('Containers — desktop', () => {
  test.use({ viewport: { width: 1280, height: 800 }, isMobile: false, hasTouch: false });

  test('offline state offers 1-click Install & Start Kiri and transitions into live containers', async ({ page }) => {
    const tracker = await mockContainersApi(page, { initialOffline: true, manyContexts: 18 });
    await openPage(page, 'containers');

    await expect(page.getByTestId('containers-page')).toBeVisible();
    await expect(page.getByRole('heading', { name: /Kiri isn’t running/i })).toBeVisible();

    const installBtn = page.getByRole('button', { name: 'Install & Start Kiri' });
    await expect(installBtn).toBeVisible();
    await expect(page.getByRole('button', { name: 'Switch to colima' })).toBeVisible();

    // Overflow ⋯ menu stays clamped inside viewport and offers Install Kiri runtime….
    const moreBtn = page.getByRole('button', { name: 'More actions', exact: true });
    await moreBtn.click();
    const menu = page.getByRole('menu', { name: 'Context menu' });
    await expect(menu.getByRole('menuitem', { name: 'Install Kiri runtime…' })).toBeVisible();
    await expectFullyInViewport(page, menu, 'Containers more menu');
    await page.keyboard.press('Escape');

    // Trigger 1-click Install & Start Kiri → transitions to online view.
    await installBtn.click();
    await expect.poll(() => tracker.installCalls()).toBe(1);
    await expect(page.getByTestId('container-detail')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('container-detail').getByRole('heading', { name: 'payments-api' })).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test('online views cover Compose groups, masked env reveal, logs, stats, resource tabs, and Run modal', async ({
    page,
  }) => {
    await mockContainersApi(page, { initialOffline: false, manyContexts: 18 });
    await openPage(page, 'containers');

    await expect(page.getByTestId('containers-page')).toBeVisible();
    await expectContentHasHeight(page);

    // Opens directly on first running container (no empty "pick one" pane).
    const detail = page.getByTestId('container-detail');
    await expect(detail.getByRole('heading', { name: 'payments-api' })).toBeVisible();

    // Masked secret reveals on demand via click-to-reveal eye button.
    await expect(detail.getByText('••••••••')).toBeVisible();
    await detail.getByRole('button', { name: 'Reveal DATABASE_PASSWORD' }).click();
    await expect(detail.getByText('s3cr3t-pass-42')).toBeVisible({ timeout: 10_000 });

    // Logs tab: renders log lines and supports in-buffer find.
    await detail.getByRole('tab', { name: 'Logs' }).click();
    await expect(detail.getByText('Payments API listening on :3000')).toBeVisible({ timeout: 10_000 });
    await detail.getByLabel('Find in logs').fill('ledger');
    await expect(detail.getByText('WARN retrying ledger sync')).toBeVisible();
    await expect(detail.getByText('Payments API listening on :3000')).toHaveCount(0);

    // Stats tab: polls live stats snapshot.
    await detail.getByRole('tab', { name: 'Stats' }).click();
    await expect(detail.getByText('4.2%')).toBeVisible({ timeout: 10_000 });
    await expect(detail.getByText('12.5%')).toBeVisible();

    // Images, Volumes, Networks tabs render cleanly without overflow.
    const sections = page.getByRole('tablist', { name: 'Containers sections' });
    await sections.getByRole('tab', { name: 'Images' }).click();
    await expect(page.getByText('ghcr.io/acme/payments')).toBeVisible({ timeout: 10_000 });
    await expectNoHorizontalOverflow(page);

    // Run from an image row: an unlisted image suggests the TCP ports it declares.
    await page.getByRole('button', { name: 'Run ghcr.io/acme/payments:1.4' }).click();
    const rowModal = page.getByRole('dialog', { name: 'Run container' });
    await expect(rowModal.getByTestId('run-suggestion')).toContainText('Suggested for acme/payments: 3000:3000');
    await rowModal.getByRole('button', { name: 'Cancel' }).click();

    await sections.getByRole('tab', { name: 'Volumes' }).click();
    await expect(page.getByText('payments_data')).toBeVisible({ timeout: 10_000 });
    await expectNoHorizontalOverflow(page);

    await sections.getByRole('tab', { name: 'Networks' }).click();
    await expect(page.getByText('payments_default')).toBeVisible({ timeout: 10_000 });
    await expectNoHorizontalOverflow(page);

    // Run… modal opens cleanly inside viewport.
    await page.getByRole('button', { name: 'Run…' }).click();
    const modal = page.getByRole('dialog', { name: 'Run container' });
    await expectFullyInViewport(page, modal, 'Run container modal');
    // A well-known image suggests its ports + env; Use fills only the empty fields.
    await modal.getByLabel('Image', { exact: true }).fill('mongo:9.0');
    await expect(modal.getByTestId('run-suggestion')).toContainText(
      'Suggested for mongo: 27017:27017 · MONGO_INITDB_ROOT_USERNAME, MONGO_INITDB_ROOT_PASSWORD',
    );
    await modal.getByLabel('Container name').fill('db');
    await modal.getByLabel('Environment variables').fill('TZ=UTC');
    await modal.getByRole('button', { name: 'Use suggested settings for mongo' }).click();
    await expect(modal.getByLabel('Ports', { exact: true })).toHaveValue('27017:27017');
    await expect(modal.getByLabel('Environment variables')).toHaveValue('TZ=UTC');
    await expectFullyInViewport(page, modal, 'Run container modal with a suggestion');
    await modal.getByRole('button', { name: 'Cancel' }).click();

    // More menu stays clamped inside viewport even with 18 contexts.
    await page.getByRole('button', { name: 'More actions', exact: true }).click();
    const overflowMenu = page.getByRole('menu', { name: 'Context menu' });
    await expect(overflowMenu.getByRole('menuitem').first()).toBeVisible();
    await expectFullyInViewport(page, overflowMenu, 'Containers overflow menu');
    await page.keyboard.press('Escape');

    await expectNoHorizontalOverflow(page);
  });
});

test.describe('Containers — responsive & themes', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`renders cleanly in ${scheme} scheme with zero horizontal overflow and phone push nav`, async ({
      page,
    }) => {
      await mockContainersApi(page, { initialOffline: false });
      await page.addInitScript((s) => {
        localStorage.setItem('otto_scheme', s);
      }, scheme);
      await openPage(page, 'containers');
      await expect(page.getByTestId('containers-page')).toBeVisible();
      await expectNoHorizontalOverflow(page);

      const width = page.viewportSize()?.width ?? 1280;
      if (width <= 640) {
        // Phone uses push navigation: tap container row → detail → back button → list.
        await page.getByTestId('container-row').first().click();
        await expect(page.getByTestId('container-detail')).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await page.getByRole('button', { name: 'Back to containers list', exact: true }).click();
        await expect(page.getByTestId('container-row').first()).toBeVisible();
      }
    });
  }
});
