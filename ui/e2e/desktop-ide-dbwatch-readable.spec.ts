import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, realpathSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, tempDir } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import {
  auditDoc,
  MONGO_COLUMNS,
  REDIS_COLUMNS,
  SESSION_INFO_COLUMNS,
  SESSION_ROUTE_COLUMNS,
  sessionInfoRow,
  sessionRouteRow,
} from './dbwatchFixture';

// DB Watch readability gate (the owner's "very not readable and not usable"):
// the panel at the dock's real heights with realistic data — a 40-column MySQL
// table, a Redis prefix with TTLs, a Mongo collection with nested documents.
// The watch engine's routes are scripted here (page.route) so the layout is
// deterministic; the engine itself is covered by otto-server's ide_db_watch
// suite and the real-engine flow in desktop-ide-scale-and-dbwatch.spec.ts.
// Screenshots land in this test's output folder (light + dark, 260 / 400 / max).

test.use({ serviceWorkers: 'block' });

type Scenario = 'mysql' | 'redis' | 'mongo';

function fixture(root: string, scenario: Scenario) {
  const now = Date.now();
  const latest = { seq: 4, reason: 'stop', file: join(root, 'db', 'dao', 'session_dao.go'), line: 135, captured_at_ms: now, duration_ms: 18, changed_rows: 12, changed_items: 2, failed_items: 0 };
  const target = scenario === 'mysql'
    ? { kind: 'ephemeral', ephemeral_id: 'tc1', engine: 'mysql', label: 'mariadb Testcontainer :55123', local: true }
    : scenario === 'redis'
      ? { kind: 'ephemeral', ephemeral_id: 'mr1', engine: 'redis', label: 'miniredis :61234', local: true }
      : { kind: 'saved', connection_id: 'mongo-stg', engine: 'mongodb', label: 'audit staging', environment: 'staging', local: false };
  const items = scenario === 'mysql'
    ? [{ object: 'session_info', kind: 'table', primary_key: ['id'], row_estimate: 1240 }, { object: 'session_route', kind: 'table', primary_key: ['session_id', 'node'], row_estimate: 37 }, { object: 'player_session_info', kind: 'table', primary_key: ['id'], row_estimate: 12 }]
    : scenario === 'redis'
      ? [{ object: 'session:*', kind: 'key_prefix', primary_key: ['key'] }]
      : [{ object: 'audit_events', kind: 'collection', primary_key: ['_id'], row_estimate: 5400 }];
  const watch = { id: 'w1', root, target, database: scenario === 'mysql' ? 'pr_bo_1' : null, items, bind: { debug_session_id: 'dbg1' }, checkpoint_count: 5, latest, busy: false, created_at_ms: now - 60_000 };
  const base = { executed_query: '', window_full: false, more_count: 0, error: null };
  let readItems: unknown[];
  if (scenario === 'mysql') {
    const modified = [17, 9, 21, 22, 23, 24, 25, 26].map((id, i) => {
      const over = i === 0 ? { status: 'closed', closed_reason: 'player_logout', updated_at: '2026-11-25 12:04:31', closed_at: '2026-11-25 12:04:31' }
        : i === 1 ? { balance: 98.5, bet_total: 69 } : { round_count: 19 + i, updated_at: '2026-11-25 12:04:31' };
      const before = sessionInfoRow(id);
      const after = sessionInfoRow(id, over);
      return { key: String(id), kind: 'modified', row: after, previous_row: before, changes: Object.entries(over).map(([column, v]) => ({ column, before: before[SESSION_INFO_COLUMNS.indexOf(column)], after: v })) };
    });
    readItems = [
      { ...base, object: 'session_info', engine: 'mysql', executed_query: 'SELECT * FROM `session_info` ORDER BY `id` DESC LIMIT 150', window_note: 'newest 150 by id of ~1.2k — older rows not compared', window_full: true, columns: SESSION_INFO_COLUMNS,
        added_count: 1, modified_count: 8, removed_count: 1, unchanged_count: 140,
        diffs: [{ key: '1241', kind: 'added', row: sessionInfoRow(1241, { status: 'open' }), changes: [] }, ...modified, { key: '3', kind: 'removed', row: sessionInfoRow(3), previous_row: sessionInfoRow(3), changes: [] }] },
      { ...base, object: 'session_route', engine: 'mysql', executed_query: 'SELECT * FROM `session_route` ORDER BY `id` DESC LIMIT 150', window_note: 'all 37 rows', columns: SESSION_ROUTE_COLUMNS,
        added_count: 0, modified_count: 1, removed_count: 0, unchanged_count: 36,
        diffs: [{ key: '9', kind: 'modified', row: sessionRouteRow(9, { gp_session: 'gp-409-d1e0', node: 'gs-node-5' }), previous_row: sessionRouteRow(9), changes: [{ column: 'gp_session', before: 'gp-409-77c2', after: 'gp-409-d1e0' }, { column: 'node', before: 'gs-node-3', after: 'gs-node-5' }] }] },
      { ...base, object: 'player_session_info', engine: 'mysql', executed_query: 'SELECT …', window_note: 'all 12 rows', columns: ['id'], added_count: 0, modified_count: 0, removed_count: 0, unchanged_count: 12, diffs: [] },
    ];
  } else if (scenario === 'redis') {
    readItems = [{ ...base, object: 'session:*', engine: 'redis', executed_query: 'SCAN 0 MATCH session:* COUNT 150', window_note: 'all 14 keys', columns: REDIS_COLUMNS,
      added_count: 1, modified_count: 2, removed_count: 1, unchanged_count: 10,
      diffs: [
        { key: 'session:1004', kind: 'added', row: ['session:1004', 'string', '{"player":"liam","balance":90}', 1800], changes: [] },
        { key: 'session:1001', kind: 'modified', row: ['session:1001', 'string', '{"player":"ava","balance":175}', 1793], previous_row: ['session:1001', 'string', '{"player":"ava","balance":50}', 1795], changes: [{ column: 'value', before: '{"player":"ava","balance":50}', after: '{"player":"ava","balance":175}' }] },
        { key: 'session:1002:route', kind: 'modified', row: ['session:1002:route', 'hash', { node: 'gs-node-5', zone: 'eu-west-1a' }, 3600], previous_row: ['session:1002:route', 'hash', { node: 'gs-node-3', zone: 'eu-west-1a' }, 412], changes: [{ column: 'value', before: { node: 'gs-node-3', zone: 'eu-west-1a' }, after: { node: 'gs-node-5', zone: 'eu-west-1a' } }, { column: 'ttl', before: 412, after: 3600 }] },
        { key: 'session:1003', kind: 'removed', row: ['session:1003', 'string', '{"player":"noam","balance":10}', 12], previous_row: ['session:1003', 'string', '{"player":"noam","balance":10}', 12], changes: [] },
      ] }];
  } else {
    const before = auditDoc(2);
    const after = auditDoc(2, { event: 'session.close', context: { device: { os: 'ios', model: 'iPhone16,2' }, geo: { country: 'MT', city: 'Sliema' }, limits: { daily_loss: 300 } } });
    readItems = [{ ...base, object: 'audit_events', engine: 'mongodb', executed_query: 'db.audit_events.find({}).sort({_id:-1}).limit(150)', window_note: 'newest 150 by _id of ~5.4k — older documents not compared', window_full: true, columns: MONGO_COLUMNS,
      added_count: 1, modified_count: 1, removed_count: 0, unchanged_count: 148,
      diffs: [
        { key: 'ObjectId("6744b1c2e4b0a90000000003")', kind: 'added', row: auditDoc(3, { event: 'session.open' }), changes: [] },
        { key: 'ObjectId("6744b1c2e4b0a90000000002")', kind: 'modified', row: after, previous_row: before, changes: [{ column: 'event', before: 'session.open', after: 'session.close' }, { column: 'context', before: before[3], after: after[3] }] },
      ] }];
  }
  const detected = Array.from({ length: 60 }, (_, i) => ({ engine: target.engine, object_name: i < 3 ? (items[i]?.object ?? `table_${i}`) : `table_${String(i).padStart(2, '0')}`, kind: 'table', operations: [i % 2 ? 'read' : 'write'], source_file: join(root, 'db', 'dao', 'session_dao.go'), line: 55 + i, verified: true }));
  detected.push({ engine: target.engine, object_name: 'GetAllConfigs', kind: 'table', operations: ['read'], source_file: join(root, 'db', 'dao', 'session_dao.go'), line: 9, verified: false });
  return {
    watch,
    read: { watch, compare: 'previous_stop', from_seq: 3, to_seq: 4, items: readItems },
    detect: {
      detected_objects: detected,
      saved_connections: [{ target_kind: 'saved', connection_id: 'stg', name: 'pr_bo staging', engine: 'mysql', host: '10.0.4.12', port: 3306, database: 'pr_bo_1', environment: 'staging', local: false }, { target_kind: 'saved', connection_id: 'prod', name: 'pr_bo prod', engine: 'mysql', host: '10.0.9.3', port: 3306, database: 'pr_bo_1', environment: 'production', local: false }],
      ephemeral_targets: [{ target_kind: 'ephemeral', ephemeral_id: 'tc1', name: 'mariadb Testcontainer', engine: 'mysql', host: '127.0.0.1', port: 55123, database: 'pr_bo_1', source: 'testcontainers', container_image: 'mariadb:11.4' }, { target_kind: 'ephemeral', ephemeral_id: 'mr1', name: 'miniredis', engine: 'redis', host: '127.0.0.1', port: 61234, source: 'process' }],
    },
  };
}

async function mockWatch(page: Page, root: string, scenario: Scenario, sessionId = 'dbg1'): Promise<void> {
  const f = fixture(root, scenario);
  await page.route('**/api/v1/ide/db-watch/**', (r) => {
    const url = r.request().url();
    if (url.includes('/detect')) return r.fulfill({ json: f.detect });
    if (url.includes('/objects')) return r.fulfill({ json: { target: f.watch.target, objects: Array.from({ length: 60 }, (_, i) => ({ name: `table_${i}`, kind: 'table' })), truncated: false } });
    if (url.includes('/watches?')) return r.fulfill({ json: { watches: [f.watch] } });
    if (url.includes('rows=1')) {
      // Composite key (session_id, node): rows share session_id (C1-018 crashed the panel here).
      const current = { columns: SESSION_ROUTE_COLUMNS, rows: [sessionRouteRow(1, { session_id: 400 }), sessionRouteRow(2, { session_id: 400, node: 'gs-node-5' }), sessionRouteRow(3, { session_id: 401 })] };
      return r.fulfill({ json: { ...f.read, items: (f.read.items as { object: string }[]).map((i) => (i.object === 'session_route' ? { ...i, current } : i)) } });
    }
    return r.fulfill({ json: f.read });
  });
  // A paused debug session of this project, so the Debug panel shows its DB line.
  const session = { id: sessionId, title: 'TestSessionDao', language: 'go', adapter: 'dlv', state: 'paused', stop_seq: 4, stopped_reason: 'breakpoint', paused_file: join(root, 'db', 'dao', 'session_dao.go'), paused_line: 135, threads: [], stack_frames: [], scopes: [], inline_values: [], breakpoints: [], output: [], target_root: root };
  await page.route('**/api/v1/ide/debug/sessions**', (r) => r.fulfill({ json: r.request().url().includes(`/sessions/${sessionId}`) ? { session } : { sessions: [session] } }));
}

function seedRoot(): string {
  const root = realpathSync(tempDir('otto-dbw-'));
  mkdirSync(join(root, 'db', 'dao'), { recursive: true });
  writeFileSync(join(root, 'go.mod'), 'module example.com/koala\n\ngo 1.22\n');
  writeFileSync(join(root, 'db', 'dao', 'session_dao.go'), 'package dao\n\nconst q = "UPDATE session_info SET status = ? WHERE id = ?"\n');
  return root;
}

async function openIde(page: Page, root: string, wsId: string, opts: { height: number; scheme: 'light' | 'dark'; tab: 'dbwatch' | 'debug' }): Promise<void> {
  await page.addInitScript(([ws, h, sch, tab]) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_ide_dock_h', h);
    localStorage.setItem('otto_scheme', sch);
    localStorage.setItem('otto.ide.dockOpen', '1');
    localStorage.setItem('otto.ide.dockTab', tab);
  }, [wsId, String(opts.height || 300), opts.scheme, opts.tab]);
  await page.emulateMedia({ colorScheme: opts.scheme });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/#/ide?root=${encodeURIComponent(root)}`);
  await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });
  if (!opts.height) {
    await page.getByTestId('ide-dock-view-menu').click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Maximize dock height' }).click();
  }
}

test.describe('DB Watch is readable at the dock’s real heights', () => {
  let wsId = '';
  let root = '';
  test.beforeAll(async () => {
    const { ctx, base } = await apiCtx();
    root = seedRoot();
    const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'DB Watch layout', root_path: root } });
    wsId = ((await r.json()) as { id: string }).id;
  });

  for (const scheme of ['light', 'dark'] as const) {
    for (const height of [260, 400, 0]) {
      test(`MySQL, 40 columns: ${height || 'max'} px, ${scheme}`, async ({ page }, info) => {
        test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
        await mockWatch(page, root, 'mysql');
        await openIde(page, root, wsId, { height, scheme, tab: 'dbwatch' });
        const panel = page.getByTestId('ide-dbwatch-panel');
        const first = panel.getByTestId('ide-dbw-table').first();
        await expect(first).toContainText('session_info');
        await expect(first).toContainText('newest 150 by id of ~1.2k');
        // A changed row shows its key and ONLY the changed cells.
        const changed = panel.locator('[data-testid="ide-dbw-diff-row"][data-kind="modified"]').first();
        await expect(changed).toContainText('status');
        await expect(changed).toContainText('open');
        await expect(changed).toContainText('closed');
        await expect(changed).not.toContainText('user_agent');
        await expect(panel.getByTestId('ide-dbw-quiet')).toContainText('player_session_info');
        await expect(page.getByTestId('ide-tab-dbwatch')).toContainText('11');
        await expectNoHorizontalOverflow(page);
        if (height === 260) {
          // More than 5 rows are readable in the shortest dock.
          const body = await panel.locator('.dbw-body').boundingBox();
          const boxes = await panel.getByTestId('ide-dbw-diff-row').evaluateAll((els) => els.map((e) => e.getBoundingClientRect()).map((r) => ({ top: r.top, bottom: r.bottom })));
          const visible = boxes.filter((b) => body && b.top >= body.y && b.bottom <= body.y + body.height).length;
          expect(visible).toBeGreaterThan(5);
        }
        await page.mouse.move(0, 0);
        await page.screenshot({ path: info.outputPath(`dbwatch-after-mysql-${height || 'max'}-${scheme}.png`) });
        if (height === 400) {
          await changed.click();
          const rec = panel.getByTestId('ide-dbw-record');
          await expect(rec).toContainText('closed_reason');
          await expect(rec).toContainText('user_agent');
          await page.screenshot({ path: info.outputPath(`dbwatch-after-mysql-record-${scheme}.png`) });
        }
      });
    }
    for (const scenario of ['redis', 'mongo'] as const) {
      test(`${scenario}: 400 px, ${scheme}`, async ({ page }, info) => {
        test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
        await mockWatch(page, root, scenario);
        await openIde(page, root, wsId, { height: 400, scheme, tab: 'dbwatch' });
        const panel = page.getByTestId('ide-dbwatch-panel');
        if (scenario === 'redis') {
          await expect(panel.locator('[data-kind="modified"]').nth(1)).toContainText('value.node');
          await expect(panel.locator('[data-kind="modified"]').nth(1)).toContainText('gs-node-5');
        } else {
          await expect(panel.locator('[data-kind="modified"]').first()).toContainText('context.geo.city');
          await expect(panel.locator('[data-kind="modified"]').first()).toContainText('Sliema');
          await expect(page.getByTestId('ide-dbw-target')).toContainText('staging');
        }
        await page.mouse.move(0, 0);
        await page.screenshot({ path: info.outputPath(`dbwatch-after-${scenario}-400-${scheme}.png`) });
      });
    }
    test(`Debug panel DB line: ${scheme}`, async ({ page }, info) => {
      test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
      await mockWatch(page, root, 'mysql');
      await openIde(page, root, wsId, { height: 300, scheme, tab: 'debug' });
      const strip = page.getByTestId('ide-dbw-strip');
      await expect(strip).toContainText('session_info');
      await expect(strip).toContainText('+1 ~8 −1');
      await expect(strip).toContainText('session_route');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: info.outputPath(`dbwatch-after-debug-strip-${scheme}.png`) });
      await strip.getByRole('button', { name: /session_route/ }).click();
      await expect(page.getByTestId('ide-dbwatch-panel')).toBeVisible();
    });
  }

  test('Show current rows on a composite primary key lists every row', async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await mockWatch(page, root, 'mysql');
    await openIde(page, root, wsId, { height: 400, scheme: 'light', tab: 'dbwatch' });
    const route = page.locator('[data-testid="ide-dbw-table"][data-object="session_route"]');
    await route.getByRole('button', { name: 'session_route options' }).click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Show current rows' }).click();
    await expect(route.getByTestId('ide-dbw-diff-row')).toHaveCount(3);
    await expect(route.getByTestId('ide-dbw-diff-row').nth(1)).toContainText('400, gs-node-5');
    expect(errors).toEqual([]);
  });

  test('a remote saved watch does not re-arm on a new debug session: one line offers the click, nothing is sent', async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
    const sent: string[] = [];
    page.on('request', (r) => {
      if (/\/ide\/db-watch\/(watches\/|objects)/.test(r.url()) && r.method() !== 'GET') sent.push(`${r.method()} ${r.url()}`);
    });
    await mockWatch(page, root, 'mongo', 'dbg2');
    await openIde(page, root, wsId, { height: 300, scheme: 'light', tab: 'debug' });
    await expect(page.getByTestId('ide-dbw-strip-prod')).toHaveText('Remote database audit staging — click to watch');
    expect(sent).toEqual([]);
    await page.screenshot({ path: info.outputPath('dbwatch-after-remote-waiting-light.png') });
  });

  for (const scheme of ['light', 'dark'] as const) {
    test(`a picked target that cannot be reached shows its error with Retry: ${scheme}`, async ({ page }, info) => {
      test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
      await mockWatch(page, root, 'mysql');
      const named: string[] = [];
      await page.route('**/api/v1/ide/db-watch/**', async (r) => {
        const url = r.request().url();
        if (url.includes('/watches?')) return r.fulfill({ json: { watches: [] } });
        if (url.includes('/detect') && (r.request().postDataJSON() as { target?: unknown })?.target) {
          named.push(url);
          return r.fulfill({ status: 502, json: { code: 'upstream', message: 'upstream: error communicating with database: dial tcp 10.0.4.12:3306: i/o timeout' } });
        }
        return r.fallback();
      });
      await openIde(page, root, wsId, { height: 400, scheme, tab: 'dbwatch' });
      await page.getByTestId('ide-dbw-target').click();
      await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: /pr_bo staging/ }).click();
      const panel = page.getByTestId('ide-dbwatch-panel');
      await expect(panel).toContainText('Can’t reach pr_bo staging');
      await expect(panel).toContainText('error communicating with database: dial tcp 10.0.4.12:3306: i/o timeout');
      await expect(page.getByTestId('ide-dbw-tables')).toBeDisabled();
      await expect(panel.getByTestId('ide-dbw-other-target')).toBeVisible();
      expect(named.length).toBe(1);
      await page.mouse.move(0, 0);
      await page.screenshot({ path: info.outputPath(`dbwatch-target-error-${scheme}.png`) });
      await panel.getByRole('button', { name: 'Retry' }).click();
      await expect.poll(() => named.length).toBe(2);
    });
  }

  test('Target ▾ and Tables ▾ stay in the viewport with 60 entries', async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop dock');
    await mockWatch(page, root, 'mysql');
    await openIde(page, root, wsId, { height: 260, scheme: 'light', tab: 'dbwatch' });
    await page.getByTestId('ide-dbw-tables').click();
    const menu = page.locator('.ctx-menu');
    await expect(menu).toContainText('Add table…');
    await expect(menu).toContainText('name in code not found in pr_bo_1');
    await expectFullyInViewport(page, menu, 'Tables menu');
    await page.screenshot({ path: info.outputPath('dbwatch-after-tables-menu.png') });
    await page.keyboard.press('Escape');
    await page.getByTestId('ide-dbw-target').click();
    await expect(menu).toContainText('mariadb Testcontainer');
    await expect(menu).toContainText('pr_bo prod');
    await expectFullyInViewport(page, menu, 'Target menu');
    await page.screenshot({ path: info.outputPath('dbwatch-after-target-menu.png') });
  });
});
