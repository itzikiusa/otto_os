import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
test.use({ serviceWorkers: 'block' });
const record = { slug: 'synthetic-plugin', name: 'Synthetic reporting plugin with a deliberately long identity', version: '2026.09.25-preview.enterprise.integration.1234567890', description: 'Reports from local synthetic fixtures', source: '/tmp/synthetic-plugin-with-a-long-fully-readable-source/location', exec: [], ui_dir: 'ui', health: 'healthy', enabled: true, installed_at: '2026-09-25T12:00:00Z', icon: 'box' };
async function plugins(page: Page) {
 await page.route('**/api/v1/plugin-admin', r => r.fulfill({ json: [record] }));
 await page.route('**/api/v1/plugins', r => r.fulfill({ json: [{ slug: record.slug, name: record.name, icon: 'box', has_ui: true }] }));
}
async function skills(page: Page) {
 await page.route('**/api/v1/library/provider-skills', r => r.fulfill({ json: [] }));
 await page.route('**/api/v1/library/bundled', r => r.fulfill({ json: [] }));
 const { ctx, base } = await apiCtx();
 const names = [`r3-alpha-${Date.now()}`, `r3-beta-${Date.now()}`];
 for (const name of names) expect((await ctx.post(`${base}/api/v1/library/skills`, { data: { name, category: 'review', description: 'Synthetic skill', body: `# ${name}\n\nA document for editing.` } })).ok()).toBeTruthy();
 await page.goto('/#/skills-eval');
 await page.getByRole('searchbox', { name: 'Search skills' }).fill('r3-');
 await page.getByTestId('skill-row').filter({ hasText: names[0] }).click();
 return { ctx, names };
}
test('phone plugin metadata is fully readable without hover', async ({ page }) => {
 await page.setViewportSize({ width: 375, height: 812 }); await plugins(page); await page.goto('/#/settings/plugins');
 await expect(page.locator('.plist .prow')).toBeVisible();
 await page.screenshot({ path: '/tmp/otto-ux-r3-settings-plugin-before.png' });
 await page.getByText('Plugin details', { exact: true }).click();
 const details = page.locator('.plugin-details');
 for (const value of [record.name, record.version, record.source]) await expect(details).toContainText(value);
 for (const item of await details.locator('dd').all()) expect(await item.evaluate(e => e.scrollWidth <= e.clientWidth + 1)).toBeTruthy();
 await expectNoHorizontalOverflow(page); await page.screenshot({ path: '/tmp/otto-ux-r3-settings-plugin-details-phone.png' });
});
test('tablet Skills can hide navigation while editing and preserve its draft', async ({ page }) => {
 await page.setViewportSize({ width: 834, height: 1112 }); const { ctx } = await skills(page);
 await page.getByRole('tab', { name: 'Files', exact: true }).click();
 const editor = page.getByTestId('skill-editor').locator('.cm-content'); await expect(editor).toContainText('A document for editing.');
 await page.screenshot({ path: '/tmp/otto-ux-r3-settings-skills-tablet-before.png' }); await editor.fill('# Keep tablet draft');
 await page.getByRole('button', { name: 'Hide skills list', exact: true }).click();
 expect((await page.getByTestId('skill-editor').boundingBox())!.width).toBeGreaterThan(480); await expect(editor).toContainText('# Keep tablet draft');
 await page.screenshot({ path: '/tmp/otto-ux-r3-settings-skills-tablet-focused.png' });
 await page.getByRole('button', { name: 'Show skills list', exact: true }).click();
 await expect(page.getByRole('searchbox', { name: 'Search skills' })).toBeVisible(); await expect(editor).toContainText('# Keep tablet draft'); await ctx.dispose();
});
test('an old skill load failure cannot replace the newly selected skill', async ({ page }) => {
 const { ctx, names } = await skills(page); await expect(page.getByTestId('skill-preview')).toContainText(names[0]);
 let release!: () => void; const held = new Promise<void>(resolve => release = resolve); let started = false;
 await page.route(`**/api/v1/library/skills/${names[1]}/file?path=*`, async r => { started = true; await held; await r.fulfill({ status: 503, json: { code: 'upstream', message: 'Old skill failed' } }); });
 await page.getByTestId('skill-row').filter({ hasText: names[1] }).click(); await expect.poll(() => started).toBe(true);
 await page.getByTestId('skill-row').filter({ hasText: names[0] }).click(); await expect(page.getByTestId('skill-name')).toHaveText(names[0]);
 const response = page.waitForResponse(r => r.url().includes(names[1]) && r.url().includes('/file?')); release(); await (await response).finished();
 await expect(page.getByTestId('skill-detail')).not.toContainText('Old skill failed'); await expect(page.getByTestId('skill-preview')).toContainText(names[0]); await ctx.dispose();
});
const html = `<!doctype html><html><body><h1>Synthetic plugin dashboard</h1><p id="state">Waiting for Otto</p><button>Open command palette</button><script>addEventListener('message', e => { if(e.data.type === 'otto:init' || e.data.type === 'otto:theme'){ for(const [k,v] of Object.entries(e.data.theme)) document.documentElement.style.setProperty(k,v); document.body.style.cssText='background:var(--bg);color:var(--text);font:13px system-ui;padding:20px'; document.querySelector('#state').textContent=e.data.scheme+' theme connected'; }});document.querySelector('button').onclick=()=>parent.postMessage({type:'otto:keydown',key:'k',metaKey:true},'*');</script></body></html>`;
test('installed PluginFrame loads, reloads, retries and explains unavailable pages', async ({ page }) => {
 await plugins(page); let status = 503;
 await page.route('**/plugins/synthetic-plugin/ui/', r => r.fulfill({ status, contentType: 'text/html', body: status === 200 ? html : 'Synthetic unavailable' }));
 await page.goto('/#/plugin/synthetic-plugin'); await expect(page.getByTestId('load-error')).toContainText('503'); status = 200;
 await page.getByRole('button', { name: 'Retry', exact: true }).click(); const frame = page.frameLocator('iframe[data-plugin]');
 await expect(frame.getByRole('heading')).toHaveText('Synthetic plugin dashboard'); await expect(frame.locator('#state')).toContainText('theme connected');
 await page.getByRole('button', { name: /^Reload Synthetic/ }).click(); await expect(frame.locator('#state')).toContainText('theme connected');
 await frame.getByRole('button', { name: 'Open command palette' }).click();
 await expect(page.getByRole('combobox', { name: 'Ask Otto or search commands' }).or(page.getByRole('combobox', { name: 'Search commands', exact: true }))).toBeFocused();
 await page.keyboard.press('Escape');
 await page.screenshot({ path: '/tmp/otto-ux-r3-settings-plugin-frame.png' }); status = 404;
 await page.getByRole('button', { name: /^Reload Synthetic/ }).click(); await expect(page.getByText(/isn’t available/)).toBeVisible();
 await expect(page.getByText(/disabled, uninstalled/)).toBeVisible(); await page.getByRole('button', { name: 'Open plugin settings' }).click(); await expect(page).toHaveURL(/settings\/plugins/);
});

test('Groups preserve unsaved fields when another group is selected', async ({ page }) => {
 const groups = [{ id: 'synthetic-alpha', name: 'Alpha readers', description: 'Read-only team' }, { id: 'synthetic-beta', name: 'Beta operators', description: 'Operations team' }];
 await page.route('**/api/v1/access/groups', r => r.fulfill({ json: groups }));
 await page.route('**/api/v1/access/roles', r => r.fulfill({ json: [{ id: 'preset-one', name: 'Readers', kind: 'connection', operations: ['read'], grantable_operations: [] }, { id: 'preset-two', name: 'Operators', kind: 'connection', operations: ['read'], grantable_operations: [] }] }));
 await page.route('**/api/v1/access/groups/*/members', r => r.fulfill({ json: [] }));
 await page.goto('/#/settings/access-groups');
 await expect(page.getByLabel('Group name', { exact: true })).toHaveValue('Alpha readers');
 await page.getByLabel('Group name', { exact: true }).fill('Alpha draft');
 await page.getByRole('button', { name: 'Beta operators', exact: true }).click();
 await expect(page.getByRole('dialog')).toContainText('Discard');
 await page.getByRole('dialog').getByRole('button', { name: 'Keep editing' }).click();
 await expect(page.getByLabel('Group name', { exact: true })).toHaveValue('Alpha draft');
 await page.getByRole('button', { name: 'Beta operators', exact: true }).click();
 await page.getByRole('dialog').getByRole('button', { name: 'Discard', exact: true }).click();
 await expect(page.getByLabel('Group name', { exact: true })).toHaveValue('Beta operators');
 await page.getByRole('button', { name: /^Role presets/ }).click();
 await page.getByLabel('Preset name', { exact: true }).fill('Readers draft');
 await page.getByRole('button', { name: 'Operators', exact: true }).click();
 await page.getByRole('dialog').getByRole('button', { name: 'Keep editing' }).click();
 await expect(page.getByLabel('Preset name', { exact: true })).toHaveValue('Readers draft');
 await page.getByRole('button', { name: 'New preset', exact: true }).click();
 await page.getByRole('dialog').getByRole('button', { name: 'Discard', exact: true }).click();
 await expect(page.getByLabel('Preset name', { exact: true })).toHaveValue('');
});

test('Skills Review and Evaluator list failures recover without starting agents', async ({ page }) => {
 const { ctx, base } = await apiCtx(); await seedWorkspace(ctx, base);
 let fail = true;
 for (const path of ['skill-reviews', 'skill-evaluations']) await page.route(`**/api/v1/workspaces/*/${path}`, r => fail ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic history unavailable' } }) : r.fulfill({ json: [] }));
 await page.route('**/api/v1/library/provider-skills', r => r.fulfill({ json: [] }));
 for (const tab of ['review', 'evaluator']) {
  fail = true; await page.goto(`/#/skills-eval/${tab}`);
  await expect(page.getByRole('alert')).toContainText('Synthetic history unavailable');
  fail = false; await page.getByRole('alert').getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByText(tab === 'review' ? 'No skill reviews yet. Start one with the form.' : 'No evaluations yet. Fill in the form to run the first one.')).toBeVisible();
  await page.screenshot({ path: `/tmp/otto-ux-r3-settings-${tab}-recovery.png` });
 }
 await ctx.dispose();
});

test('MCP external discovery fails honestly and recovers using safe fixtures', async ({ page }) => {
 const { ctx, base } = await apiCtx(); const ws = await seedWorkspace(ctx, base);
 const server = { id: 'synthetic-server', workspace_id: ws, name: 'Synthetic reports', transport: 'stdio', command: 'fixture-command', args: [], env: {}, url: null, description: 'Contract fixture only', headers: {}, secret_env_keys: [], secret_header_keys: [], has_secret: false, injection_risk: 'low', managed: false, default_tool_access: 'deny', enabled: true, health_status: 'healthy', health_checked_at: null, health_latency_ms: 7, health_error: null, tools_count: 0, tools_discovered_at: null, created_by: 'root', created_at: '2026-09-25T12:00:00Z', updated_at: '2026-09-25T12:00:00Z' };
 await page.route('**/api/v1/workspaces/*/mcp/servers', r => r.fulfill({ json: [server] }));
 await page.route('**/api/v1/access/mcp_server/synthetic-server/capabilities', r => r.fulfill({ json: { kind: 'mcp_server', resource_id: server.id, user_id: 'root', child: null, mode: 'legacy', operations: {} } }));
 let fail = true;
 await page.route('**/api/v1/mcp/servers/synthetic-server/discover', r => fail ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Discovery fixture failed' } }) : r.fulfill({ json: [] }));
 await page.goto('/#/mcp/servers');
 await page.getByRole('button', { name: 'Discover', exact: true }).click();
 await expect(page.getByText('Discovery failed', { exact: true })).toBeVisible();
 await expect(page.getByRole('button', { name: 'Discover', exact: true })).toBeEnabled();
 fail = false; await page.getByRole('button', { name: 'Discover', exact: true }).click();
 await expect(page.getByText('Discovered tools', { exact: true })).toBeVisible();
 await page.getByRole('button', { name: 'More actions for Synthetic reports' }).click();
 await page.getByRole('menuitem', { name: 'Delete…', exact: true }).click();
 await expect(page.getByRole('dialog')).toContainText('discovered tools and allowlist entries');
 await page.keyboard.press('Escape'); await expect(page.getByRole('dialog')).toHaveCount(0);
 await ctx.dispose();
});

test('installed plugins retain readable metadata and theme initialization in five variants', async ({ page }) => {
 await plugins(page); await page.route('**/plugins/synthetic-plugin/ui/', r => r.fulfill({ contentType: 'text/html', body: html }));
 const variants = [
  { key: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900, direction: 'ltr' },
  { key: 'native-dark-phone', theme: 'native', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
  { key: 'warm-light-tablet-rtl', theme: 'warm', scheme: 'light', width: 834, height: 1112, direction: 'rtl' },
  { key: 'warm-dark', theme: 'warm', scheme: 'dark', width: 1440, height: 900, direction: 'ltr' },
  { key: 'pro-dark-phone', theme: 'pro-dark', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
 ];
 await page.goto('/#/settings/plugins');
 for (const v of variants) {
  await page.setViewportSize(v); await page.evaluate(v => { localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme); localStorage.setItem('otto_direction', v.direction); }, v); await page.reload();
  await page.goto('/#/settings/plugins'); await page.getByText('Plugin details', { exact: true }).click();
  await expect(page.locator('.plugin-details')).toContainText(record.source); await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: `/tmp/otto-ux-r3-settings-metadata-${v.key}.png` });
  await page.goto('/#/plugin/synthetic-plugin'); await expect(page.frameLocator('iframe[data-plugin]').locator('#state')).toHaveText(`${v.scheme} theme connected`);
  await expectNoHorizontalOverflow(page); await page.screenshot({ path: `/tmp/otto-ux-r3-settings-frame-${v.key}.png` });
 }
});

test('backup restore preview requires review and a fresh preview after failure using intercepted fixtures', async ({ page }) => {
 let restores = 0;
 const preview = { preview_token: 'synthetic-preview', can_restore: true, record_count: 1, file_count: 0, table_counts: { workflows: 1 }, conflicts: [], excluded: ['Credentials excluded'], reconnect: ['Reconnect synthetic account'], warnings: [] };
 await page.route('**/api/v1/state/archive/preview', r => r.fulfill({ json: preview }));
 await page.route('**/api/v1/state/archive/restore', r => { restores++; return restores === 1 ? r.fulfill({ status: 409, json: { code: 'conflict', message: 'Preview expired' } }) : r.fulfill({ json: { records_inserted: 1, records_skipped: 0, files_restored: 0, files_skipped: 0, restore_root: '/tmp/synthetic-restore', reconnect: [] } }); });
 await page.goto('/#/settings/backup');
 const section = page.getByRole('region', { name: 'Otto data backup' });
 await section.locator('input[type="file"]').setInputFiles({ name: 'synthetic-archive.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify({ archive_format: 2, schema_version: 1, daemon_version: 'fixture', snapshot_at: '2026-09-25T12:00:00Z', records: { workflows: [{ id: 'fixture' }] }, roots: [], files: [], excluded: [], reconnect: [] })) });
 await section.getByRole('button', { name: 'Preview restore', exact: true }).click();
 const restore = section.getByRole('button', { name: 'Restore new items', exact: true });
 await expect(restore).toBeDisabled();
 await section.getByRole('checkbox', { name: /I reviewed/ }).check(); await restore.click();
 await expect(section.getByRole('alert')).toContainText('Preview expired'); await expect(restore).toHaveCount(0); expect(restores).toBe(1);
 await section.getByRole('button', { name: 'Preview restore', exact: true }).click(); await expect(restore).toBeDisabled();
 await section.getByRole('checkbox', { name: /I reviewed/ }).check(); await restore.click(); await expect(section.getByRole('status')).toContainText('Restored 1 records and 0 files'); expect(restores).toBe(2);
});
