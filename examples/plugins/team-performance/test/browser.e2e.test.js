// Browser E2E: drives the REAL UI (ui/index.html) in Chromium against the REAL
// sidecar fed by mock data (mock Otto host API + mock Jira with many projects +
// a scripted git repo). Checks what unit tests can't: light/dark at phone and
// desktop widths, no horizontal page scroll, no native dialogs, the project and
// "more" menus clamped fully into the viewport, and that Settings → time off and
// an estimate correction persist through the API.
//
// Playwright is OPTIONAL (the plugin is zero-dep): the suite skips when neither
// the module nor a Chromium binary is available. Resolution order for the
// module: the plugin dir, then the Otto repo's ui/node_modules. Browser: the
// bundled one, then $PLAYWRIGHT_CHROMIUM_PATH, then system Chrome, then any
// cached ms-playwright Chromium.
// Run (from the plugin dir): node --test test/browser.e2e.test.js
'use strict';
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { startMockJira } = require('./fixtures/mock-jira.js');

const PLUGIN_DIR = path.join(__dirname, '..');
const UI_DIR = path.join(PLUGIN_DIR, 'ui');
const MANY_PROJECTS = 60; // enough to overflow any viewport

function loadPlaywright() {
  for (const base of [PLUGIN_DIR, path.join(PLUGIN_DIR, '..', '..', '..', 'ui')]) {
    try {
      return require(require.resolve('playwright', { paths: [base] }));
    } catch {
      /* next */
    }
  }
  return null;
}

async function launchChromium(pw) {
  const tries = [{}];
  if (process.env.PLAYWRIGHT_CHROMIUM_PATH) tries.push({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH });
  tries.push({ channel: 'chrome' });
  const cache = path.join(os.homedir(), 'Library', 'Caches', 'ms-playwright');
  try {
    for (const d of fs.readdirSync(cache).filter((n) => /^chromium-\d+$/.test(n)).sort().reverse()) {
      for (const rel of [
        'chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing',
        'chrome-mac/Chromium.app/Contents/MacOS/Chromium',
        'chrome-linux/chrome',
      ]) {
        const p = path.join(cache, d, rel);
        if (fs.existsSync(p)) tries.push({ executablePath: p });
      }
    }
  } catch {
    /* no cache dir */
  }
  for (const opts of tries) {
    try {
      return await pw.chromium.launch({ headless: true, ...opts });
    } catch {
      /* next */
    }
  }
  return null;
}

const pw = loadPlaywright();
let browser = null;
let skipReason = pw ? '' : 'playwright not installed';

let mockJira, jiraProxy, hostServer, uiServer, plugin;
let pluginPort, uiPort;
let dataDir, repoDir;

function req(port, method, pathname, body) {
  return new Promise((resolve, reject) => {
    const data = body ? JSON.stringify(body) : null;
    const r = http.request({ method, hostname: '127.0.0.1', port, path: pathname, headers: data ? { 'Content-Type': 'application/json' } : {} }, (res) => {
      let buf = '';
      res.on('data', (c) => (buf += c));
      res.on('end', () => {
        let json = null;
        try { json = buf ? JSON.parse(buf) : null; } catch { /* not json */ }
        resolve({ status: res.statusCode, json });
      });
    });
    r.on('error', reject);
    if (data) r.write(data);
    r.end();
  });
}
const api = (m, p, b) => req(pluginPort, m, p, b);

function gitRepo() {
  repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-br-repo-'));
  const g = (args, when) =>
    execFileSync('git', ['-C', repoDir, ...args], {
      encoding: 'utf8',
      env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t', ...(when ? { GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when } : {}) },
    });
  const commit = (msg, when) => {
    fs.appendFileSync(path.join(repoDir, 'f.txt'), msg + '\n');
    g(['add', '.']);
    g(['commit', '-q', '-m', msg], when);
  };
  g(['init', '-q', '-b', 'main']);
  commit('init', '2026-05-01T09:00:00Z');
  g(['checkout', '-q', '-b', 'develop']);
  for (const [key, dev, merge] of [
    ['TP-1', '2026-06-02T12:00:00Z', '2026-06-05T11:00:00Z'],
    ['TP-2', '2026-06-09T10:00:00Z', '2026-06-10T16:00:00Z'],
    ['TP-3', '2026-06-11T10:00:00Z', '2026-06-16T11:00:00Z'],
    ['TP-4', '2026-06-05T10:00:00Z', '2026-06-10T10:00:00Z'],
  ]) {
    g(['checkout', '-q', '-b', `feature/${key}-work`]);
    commit(`${key}: implement`, dev);
    g(['checkout', '-q', 'develop']);
    g(['merge', '-q', '--no-ff', '-m', `Merge branch 'feature/${key}-work' into develop`, `feature/${key}-work`], merge);
  }
  g(['tag', '-a', 'v1.0-DEPLOYED', '-m', 'prod'], '2026-06-23T09:00:00Z');
}

const listen = (srv) => new Promise((r) => srv.listen(0, '127.0.0.1', () => r(srv.address().port)));

/** Forward a request to 127.0.0.1:port (used for the Jira overlay and the /api proxy). */
function forward(req0, res0, port, pathname) {
  const r = http.request({ method: req0.method, hostname: '127.0.0.1', port, path: pathname, headers: { ...req0.headers, host: `127.0.0.1:${port}` } }, (res) => {
    res0.writeHead(res.statusCode, res.headers);
    res.pipe(res0);
  });
  r.on('error', () => { res0.writeHead(502); res0.end(); });
  req0.pipe(r);
}

before(async () => {
  if (!pw) {
    if (process.env.OTTO_TP_REQUIRE_BROWSER === '1') throw new Error('Team Performance browser gate requires ui/npm ci and Playwright Chromium');
    return;
  }
  browser = await launchChromium(pw);
  if (!browser) {
    if (process.env.OTTO_TP_REQUIRE_BROWSER === '1') throw new Error('Install the browser with: cd ui && npx playwright install chromium');
    skipReason = 'no Chromium binary available for playwright'; return;
  }

  mockJira = await startMockJira();
  // Jira overlay: many projects (TP + fillers), everything else → mock Jira.
  jiraProxy = http.createServer((q, s) => {
    const u = new URL(q.url, 'http://x');
    if (u.pathname === '/rest/api/3/project/search') {
      const values = [{ key: 'TP', name: 'Team Performance Fixture' }];
      for (let i = 1; i < MANY_PROJECTS; i++) values.push({ key: `ABC${i}`, name: `Filler project number ${i} with a long descriptive name` });
      s.writeHead(200, { 'Content-Type': 'application/json' });
      return s.end(JSON.stringify({ values, isLast: true }));
    }
    return forward(q, s, mockJira.port, q.url);
  });
  const jiraPort = await listen(jiraProxy);
  gitRepo();
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-br-data-'));

  hostServer = http.createServer((q, s) => {
    const u = new URL(q.url, 'http://x');
    const send = (code, obj) => { s.writeHead(code, { 'Content-Type': 'application/json' }); s.end(JSON.stringify(obj)); };
    if (u.pathname === '/repos') return send(200, [{ id: 'r1', name: 'fixture', path: repoDir, remote_url: null }]);
    if (u.pathname === '/jira/accounts') return send(200, [{ id: 'acc1', label: 'Fixture Jira', base_url: `http://127.0.0.1:${jiraPort}`, email: 'e@e' }]);
    if (u.pathname === '/jira/credentials') return send(200, { base_url: `http://127.0.0.1:${jiraPort}`, email: 'e@e', token: 'tok' });
    if (u.pathname === '/agents/run') {
      let b = '';
      q.on('data', (c) => (b += c));
      q.on('end', () => {
        const body = JSON.parse(b || '{}');
        const prompt = String(body.prompt || '');
        if (prompt.includes('STRICT JSON')) {
          const keys = [...prompt.matchAll(/key=(\S+)/g)].map((m) => m[1]);
          return send(200, { text: JSON.stringify(keys.map((k) => ({ key: k, days: 0.5, routine: false }))) });
        }
        send(200, { text: 'ok' });
      });
      return undefined;
    }
    return send(404, {});
  });
  const hostPort = await listen(hostServer);

  pluginPort = 20000 + Math.floor(Math.random() * 20000);
  plugin = spawn(process.execPath, ['server.js'], {
    cwd: PLUGIN_DIR,
    env: { ...process.env, OTTO_PLUGIN_PORT: String(pluginPort), OTTO_HOST_API: `http://127.0.0.1:${hostPort}`, OTTO_PLUGIN_TOKEN: 'ptok', OTTO_PLUGIN_DATA_DIR: dataDir },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  plugin.stderr.on('data', () => {});
  const deadline = Date.now() + 10000;
  for (;;) {
    try { if ((await api('GET', '/health')).status === 200) break; } catch { /* not up */ }
    if (Date.now() > deadline) throw new Error('sidecar never became healthy');
    await new Promise((r) => setTimeout(r, 100));
  }
  // Fixed June 2026 tickets remain eligible regardless of the execution date.
  // Estimates still come through the real scan and mocked provider boundary.
  assert.equal((await api('PUT', '/config', { estimate_window_months: 0, estimate_since: '', git_fetch: false, auto_scan_minutes: 0 })).status, 200);
  // Scan so every view has data.
  const s = await api('POST', '/scan', { account: 'acc1', project: 'TP' });
  assert.equal(s.status, 200);
  const until = Date.now() + 60000;
  for (;;) {
    const st = (await api('GET', '/scan/status?account=acc1&project=TP')).json;
    if (st && st.state === 'done') break;
    if (st && st.state === 'error') throw new Error(`scan errored: ${st.error}`);
    if (Date.now() > until) throw new Error('scan did not finish');
    await new Promise((r) => setTimeout(r, 150));
  }

  // UI server: static ui/ + same-origin /api → sidecar (what the Otto host proxy does).
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
  uiServer = http.createServer((q, s) => {
    const u = new URL(q.url, 'http://x');
    if (u.pathname.startsWith('/api/')) return forward(q, s, pluginPort, q.url.slice(4));
    const rel = u.pathname === '/' ? 'index.html' : decodeURIComponent(u.pathname.slice(1));
    const file = path.resolve(UI_DIR, rel);
    if (!file.startsWith(UI_DIR + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { s.writeHead(404); return s.end(); }
    s.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(s);
  });
  uiPort = await listen(uiServer);
});

after(async () => {
  if (browser) await browser.close().catch(() => {});
  if (plugin) plugin.kill('SIGKILL');
  for (const srv of [uiServer, hostServer, jiraProxy]) if (srv) await new Promise((r) => srv.close(r));
  if (mockJira) await mockJira.close();
  for (const d of [dataDir, repoDir]) if (d) fs.rmSync(d, { recursive: true, force: true });
});

/**
 * Open the UI like Otto does: the page is top-level here, so window.parent ===
 * window and a self-postMessage passes the source check in app.js.
 */
async function openUi({ width, scheme }) {
  const ctx = await browser.newContext({ viewport: { width, height: width < 600 ? 844 : 900 }, colorScheme: scheme });
  const page = await ctx.newPage();
  const dialogs = [];
  const errors = [];
  page.on('dialog', (d) => { dialogs.push(`${d.type()}: ${d.message()}`); d.dismiss().catch(() => {}); });
  page.on('pageerror', (e) => errors.push(String(e && e.message)));
  await page.goto(`http://127.0.0.1:${uiPort}/index.html?theme=${scheme}`);
  await page.evaluate((s) => window.postMessage({ type: 'otto:init', apiBase: '/api', token: '', scheme: s, providers: ['claude'] }, '*'), scheme);
  await page.waitForFunction(() => window.TP && TP.app && TP.app.projects && TP.app.projects.length > 0, null, { timeout: 15000 });
  return { ctx, page, dialogs, errors };
}

const noHScroll = (page) => page.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth }));

/** The open popover's rect must lie inside the viewport and its overflow must scroll. */
async function assertPopoverInViewport(page, what) {
  await page.waitForSelector('.popover', { timeout: 5000 });
  const r = await page.evaluate(() => {
    const el = document.querySelector('.popover');
    const b = el.getBoundingClientRect();
    return { top: b.top, left: b.left, bottom: b.bottom, right: b.right, vw: window.innerWidth, vh: window.innerHeight };
  });
  assert.ok(r.top >= 0 && r.left >= 0, `${what}: top/left inside viewport (${JSON.stringify(r)})`);
  assert.ok(r.bottom <= r.vh + 0.5 && r.right <= r.vw + 0.5, `${what}: bottom/right inside viewport (${JSON.stringify(r)})`);
}

for (const width of [390, 1280]) {
  for (const scheme of ['light', 'dark']) {
    test(`layout @${width}px ${scheme}: themed, no horizontal scroll, menus clamped, no native dialogs`, async (t) => {
      if (skipReason) return t.skip(skipReason);
      const { ctx, page, dialogs, errors } = await openUi({ width, scheme });
      try {
        assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), scheme);
        const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
        const lum = (() => { const m = bg.match(/\d+(\.\d+)?/g) || ['0', '0', '0']; return (+m[0] + +m[1] + +m[2]) / 3; })();
        assert.ok(scheme === 'dark' ? lum < 110 : lum > 150, `body background matches ${scheme} (${bg})`);

        const tabs = await page.evaluate(() => (TP.app && TP.app.tab) || null);
        assert.ok(tabs, 'app booted to a tab');
        // Every tab: no horizontal page scroll.
        for (const tab of ['overview', 'flow', 'quality', 'investment', 'people', 'estimates', 'reports', 'settings']) {
          const ok = await page.evaluate(async (k) => {
            const btn = document.querySelector(`[data-tab="${k}"], #tab-${k}, [aria-controls][data-k="${k}"]`);
            if (btn) btn.click();
            else if (TP.app.goTab) await TP.app.goTab(k);
            else return false;
            return true;
          }, tab);
          if (!ok) continue;
          await page.waitForTimeout(300);
          const { sw, cw } = await noHScroll(page);
          assert.ok(sw <= cw + 1, `${tab}: no horizontal page scroll (${sw} > ${cw})`);
          if (tab === 'overview') {
            const details = page.locator('.tile.weak .guard-details').first();
            await details.waitFor();
            assert.equal(await details.evaluate((el) => el.open), false);
            assert.equal(await details.locator('.guard-reason').isVisible(), false);
            assert.ok(await details.evaluate((el) => el.closest('.tile').getBoundingClientRect().height < 280), 'collapsed weak metric remains compact');
            await details.locator('summary').press('Enter');
            assert.equal(await details.locator('.guard-reason').isVisible(), true);
            assert.ok((await details.locator('.guard-reason').textContent()).length > 20, 'full reasons remain accessible');
            await details.locator('summary').press('Enter');
            assert.equal(await details.evaluate((el) => el.open), false);
          }
          if (process.env.OTTO_TP_SCREENSHOT_DIR && ['overview', 'settings'].includes(tab)) {
            fs.mkdirSync(process.env.OTTO_TP_SCREENSHOT_DIR, { recursive: true });
            await page.screenshot({ path: path.join(process.env.OTTO_TP_SCREENSHOT_DIR, `${tab}-${width}-${scheme}.png`), animations: 'disabled' });
          }
        }

        await page.click('#proj-btn');
        await assertPopoverInViewport(page, 'projects menu');
        const items = await page.$$eval('.popover #proj-items label', (l) => l.length);
        assert.equal(items, MANY_PROJECTS, 'every project listed');
        const scrolls = await page.evaluate(() => {
          const el = document.querySelector('.popover');
          const sc = [el, ...el.querySelectorAll('*')].some((n) => n.scrollHeight > n.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(n).overflowY));
          return sc;
        });
        assert.ok(scrolls, 'the long project list scrolls inside the popover');
        await page.keyboard.press('Escape');
        await page.waitForSelector('.popover', { state: 'detached', timeout: 3000 });

        await page.click('#more-btn');
        await assertPopoverInViewport(page, 'more menu');
        await page.keyboard.press('Escape');

        assert.deepEqual(dialogs, [], 'no native alert/confirm/prompt');
        assert.deepEqual(errors, [], 'no uncaught page errors');
      } finally {
        await ctx.close();
      }
    });
  }
}

test('Settings → time off persists through Save', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page, dialogs } = await openUi({ width: 1280, scheme: 'light' });
  try {
    await page.evaluate(() => (TP.app.goTab ? TP.app.goTab('settings') : document.querySelector('[data-tab="settings"]').click()));
    const add = await page.waitForSelector('[data-off-add]', { timeout: 10000 });
    const pid = await add.getAttribute('data-off-add');
    const row = await add.evaluateHandle((b) => b.closest('tr'));
    await (await row.$('.off-from')).fill('2026-06-08');
    await (await row.$('.off-to')).fill('2026-06-12');
    await add.click();
    await page.waitForSelector(`[data-chips="${pid}"] .badge`, { timeout: 3000 });
    await page.click('#save-config');
    const until = Date.now() + 8000;
    let saved = null;
    while (Date.now() < until) {
      const p = (await api('GET', '/people?account=acc1')).json;
      saved = p && p.people && p.people[pid];
      if (saved && (saved.time_off || []).some((o) => o.from === '2026-06-08' && o.to === '2026-06-12')) break;
      await new Promise((r) => setTimeout(r, 150));
    }
    assert.ok(saved && saved.time_off.some((o) => o.from === '2026-06-08' && o.to === '2026-06-12'), `time off persisted for ${pid}: ${JSON.stringify(saved && saved.time_off)}`);
    assert.deepEqual(dialogs, []);
  } finally {
    await ctx.close();
  }
});

test('Estimates → Correct persists an estimate override', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page, dialogs } = await openUi({ width: 1280, scheme: 'dark' });
  try {
    await page.selectOption('#period', '');
    await page.getByRole('tab', { name: 'Estimates', exact: true }).click();
    const key = 'TP-1';
    const fix = await page.waitForSelector('[aria-label="Correct estimate for TP-1"]', { timeout: 10000 });
    assert.ok(fix, 'the real scan must expose the known estimated ticket for correction');
    const histogram = page.getByRole('img', { name: /^Actual ÷ estimate distribution/ });
    await histogram.waitFor({ state: 'visible' });
    await page.locator('.chart').filter({ has: histogram }).locator('summary').click();
    const accuracyTable = page.getByRole('table', { name: 'Tickets per accuracy bucket' });
    await accuracyTable.waitFor({ state: 'visible' });
    assert.ok(await accuracyTable.locator('tbody tr').count(), 'real accuracy bins render through the table disclosure');
    await fix.click();
    await page.waitForSelector('#ce-days', { timeout: 3000 });
    await page.fill('#ce-days', '3.5');
    await page.fill('#ce-reason', 'includes a "hidden" migration');
    await page.getByRole('button', { name: 'Save correction' }).click();
    const until = Date.now() + 8000;
    let est = null;
    while (Date.now() < until) {
      const a = (await api('GET', '/overview?account=acc1&projects=TP')).json;
      const ids = ((a && a.assignees) || []).map((x) => x.assignee_id);
      for (const id of ids) {
        const d = (await api('GET', `/assignee?account=acc1&projects=TP&assignee=${encodeURIComponent(id)}`)).json;
        const hit = d && (d.completed || []).find((x) => x.key === key);
        if (hit) est = hit;
      }
      if (est && est.est_overridden) break;
      await new Promise((r) => setTimeout(r, 200));
    }
    assert.ok(est && est.est_overridden, `override for ${key} persisted`);
    assert.equal(est.est_days_ai, 3.5);
    assert.equal(est.est_reason, 'includes a "hidden" migration');
    assert.deepEqual(dialogs, []);
  } finally {
    await ctx.close();
  }
});

test('quality: nested overlay owns Escape, focus and inertness', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  try {
    await page.evaluate(() => {
      TP.modal({ title: 'Report', body: '<textarea id="draft-comment">Unsent comment</textarea><button id="download">Download</button>', onOpen(el) {
        el.querySelector('#download').onclick = () => TP.confirmer.ask({ title: 'Download report?', message: 'Download locally' });
      } });
    });
    await page.click('#download');
    assert.equal(await page.locator('.modal-backdrop').first().getAttribute('inert'), '');
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('.modal-backdrop').count(), 1);
    assert.equal(await page.inputValue('#draft-comment'), 'Unsent comment');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'download');
    await page.evaluate(() => TP.popover(document.querySelector('#download'), '<button id="pop-action">Action</button>'));
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('.modal-backdrop').count(), 1);
    assert.equal(await page.locator('.popover').count(), 0);
    await page.keyboard.press('Tab');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'draft-comment');
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('.modal-backdrop').count(), 0);
  } finally { await ctx.close(); }
});

test('quality: settings drafts survive refresh, scope/tab switches and edits during Save', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'dark' });
  try {
    await page.evaluate(() => TP.app.goTab('settings'));
    await page.fill('#cfg-est-instr', 'unsaved calibration');
    await page.evaluate(() => TP.app.refresh());
    assert.equal(await page.inputValue('#cfg-est-instr'), 'unsaved calibration');
    await page.evaluate(() => { TP.app.goTab('people'); TP.app.goTab('settings'); });
    await page.waitForSelector('#cfg-est-instr');
    assert.equal(await page.inputValue('#cfg-est-instr'), 'unsaved calibration');
    let release;
    const pending = new Promise((r) => { release = r; });
    await page.route('**/api/config', async (route) => {
      if (route.request().method() !== 'PUT') return route.continue();
      await pending;
      await route.fulfill({ json: route.request().postDataJSON() });
    });
    await page.click('#save-config');
    await page.waitForFunction(() => document.querySelector('#save-config').disabled);
    await page.fill('#cfg-est-instr', 'newer calibration');
    release();
    await page.waitForFunction(() => !document.querySelector('#save-config')?.disabled);
    assert.equal(await page.inputValue('#cfg-est-instr'), 'newer calibration');
    await page.evaluate(() => TP.app.refresh());
    assert.equal(await page.inputValue('#cfg-est-instr'), 'newer calibration');
  } finally { await ctx.close(); }
});

test('quality: newest scope response owns metrics including stale failures', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  try {
    await page.waitForFunction(() => TP.app.overview);
    let release;
    const held = new Promise((r) => { release = r; });
    await page.route('**/api/overview?**', async (route) => {
      if (new URL(route.request().url()).searchParams.get('projects') === 'OLD') {
        await held;
        return route.fulfill({ status: 503, json: { error: 'old failure' } });
      }
      return route.fulfill({ json: { completed: 123, assignees: [], freshness: {} } });
    });
    await page.evaluate(() => { TP.app.selected = ['OLD']; window.oldRefresh = TP.app.refresh(); });
    await page.evaluate(() => { TP.app.selected = ['NEW']; return TP.app.refresh(); });
    release();
    await page.evaluate(() => window.oldRefresh);
    assert.equal(await page.evaluate(() => TP.app.overview?.completed), 123);
    assert.equal(await page.evaluate(() => TP.app.overviewErr), null);
  } finally { await ctx.close(); }
});

test('quality: unreachable scan status shows stale progress and Retry recovers', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  try {
    let unavailable = true;
    await page.route('**/api/scan', (route) => route.fulfill({ json: { started: true } }));
    await page.route('**/api/scan/status?**', (route) => route.fulfill(unavailable
      ? { status: 503, json: { error: 'offline' } }
      : { json: { state: 'stopped', fetched: 2 } }));
    await page.evaluate(() => TP.app.startScan(false));
    await page.waitForSelector('#scan-retry', { timeout: 4000 });
    assert.match(await page.locator('#scan-status').innerText(), /status unavailable/i);
    unavailable = false;
    await page.click('#scan-retry');
    await page.waitForFunction(() => !document.querySelector('#scan').disabled);
    assert.match(await page.locator('#scan-status').innerText(), /stopped/i);
  } finally { await ctx.close(); }
});

test('quality: late account projects and people cannot overwrite current account', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  try {
    await page.waitForFunction(() => TP.app.overview);
    let release;
    const held = new Promise((r) => { release = r; });
    await page.route('**/api/projects?**', async (route) => {
      const account = new URL(route.request().url()).searchParams.get('account');
      if (account === 'old') await held;
      await route.fulfill({ json: [{ key: account === 'old' ? 'OLD' : 'NEW', name: account }] });
    });
    await page.route('**/api/people?**', async (route) => {
      const account = new URL(route.request().url()).searchParams.get('account');
      if (account === 'acc1') { await held; return route.fulfill({ status: 503, json: { error: 'old people failed' } }); }
      await route.fulfill({ json: { people: { current: { name: account } } } });
    });
    await page.evaluate(() => {
      window.oldPeople = TP.app.refreshPeople();
      document.querySelector('#account').insertAdjacentHTML('beforeend', '<option value="old">old</option><option value="new">new</option>');
      TP.app.accounts.push({ id: 'old' }, { id: 'new' });
    });
    await page.selectOption('#account', 'old');
    await page.selectOption('#account', 'new');
    await page.waitForFunction(() => TP.app.projects[0]?.key === 'NEW' && TP.app.people.current);
    release();
    await page.evaluate(() => window.oldPeople);
    await page.waitForTimeout(100);
    assert.equal(await page.evaluate(() => TP.app.projects[0].key), 'NEW');
    assert.equal(await page.evaluate(() => TP.app.people.current?.name), 'new');
  } finally { await ctx.close(); }
});

test('quality edge: popover above modal traps Tab in its active layer', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  try {
    await page.evaluate(() => {
      TP.modal({ title: 'Parent', body: '<button id="parent-anchor">More</button>' });
      TP.popover(document.querySelector('#parent-anchor'), '<button id="only-pop">Only popover action</button>');
    });
    await page.keyboard.press('Tab');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'only-pop');
    await page.keyboard.press('Shift+Tab');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'only-pop');
  } finally { await ctx.close(); }
});

test('quality edge: settings keep account drafts and failed saves; scoped maps save together', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  try {
    await page.waitForFunction(() => TP.app.overview);
    await page.route('**/api/statuses?**', (route) => route.fulfill({ json: [{ name: 'Review', mapped: 'waiting' }] }));
    await page.evaluate(() => { TP.app.selected = ['TP']; TP.app.goTab('settings'); });
    await page.fill('#cfg-est-instr', 'account one draft');
    await page.selectOption('.st-map', 'implementation');
    await page.evaluate(() => { TP.app.selected = ['OTHER']; TP.app.render(); });
    await page.selectOption('.st-map', 'design');
    let sent;
    await page.route('**/api/config', (route) => {
      if (route.request().method() !== 'PUT') return route.continue();
      sent = route.request().postDataJSON();
      return route.fulfill({ status: 503, json: { error: 'fixture save failure' } });
    });
    await page.click('#save-config');
    await page.waitForFunction(() => document.querySelector('.toasts')?.textContent.includes('Couldn’t save settings'));
    assert.equal(sent.status_map.TP.Review, 'implementation');
    assert.equal(sent.status_map.OTHER.Review, 'design');
    assert.equal(await page.inputValue('#cfg-est-instr'), 'account one draft');
    await page.evaluate(() => { TP.app.account = 'acc2'; TP.app.render(); });
    await page.fill('#cfg-est-instr', 'account two draft');
    await page.evaluate(() => { TP.app.account = 'acc1'; TP.app.render(); });
    assert.equal(await page.inputValue('#cfg-est-instr'), 'account one draft');
    await page.evaluate(() => { TP.app.account = 'acc2'; TP.app.render(); });
    assert.equal(await page.inputValue('#cfg-est-instr'), 'account two draft');
  } finally { await ctx.close(); }
});


for (const refreshRoute of ['people', 'overview']) test(`quality review: edits during post-save ${refreshRoute} refresh survive the next render`, async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  let release;
  try {
    await page.evaluate(() => TP.app.goTab('settings'));
    await page.fill('#cfg-est-instr', 'submitted calibration');
    const pending = new Promise((resolve) => { release = resolve; });
    let entered = false;
    await page.route(`**/api/${refreshRoute}?**`, async (route) => {
      entered = true;
      await pending;
      await route.continue();
    });
    await page.click('#save-config');
    for (let i = 0; !entered && i < 100; i++) await new Promise((resolve) => setTimeout(resolve, 20));
    assert.equal(entered, true, 'post-save followup request reached the held response');
    await page.fill('#cfg-est-instr', 'typed after config success');
    release();
    await page.waitForFunction(() => !document.querySelector('#save-config')?.disabled);
    await page.evaluate(() => TP.app.refresh());
    assert.equal(await page.inputValue('#cfg-est-instr'), 'typed after config success');
  } finally { release?.(); await ctx.close(); }
});

// Real report view and scan presentation, with only transport payloads scripted.
// No report generation, comment submission, external scan or file download occurs.
for (const width of [390, 1280]) for (const scheme of ['light', 'dark']) {
  test(`quality visual closure @${width}px ${scheme}: report, nested Download and scan recovery`, async (t) => {
    if (skipReason) return t.skip(skipReason);
    const { ctx, page, dialogs, errors } = await openUi({ width, scheme });
    const capture = async (name) => {
      if (!process.env.OTTO_TP_SCREENSHOT_DIR) return;
      fs.mkdirSync(process.env.OTTO_TP_SCREENSHOT_DIR, { recursive: true });
      await page.screenshot({ path: path.join(process.env.OTTO_TP_SCREENSHOT_DIR, `${name}-${width}-${scheme}.png`), animations: 'disabled' });
    };
    const inViewport = async (locator, label) => {
      const r = await locator.evaluate((el) => {
        const b = el.getBoundingClientRect();
        return { x: b.x, y: b.y, right: b.right, bottom: b.bottom, width: b.width, height: b.height, vw: innerWidth, vh: innerHeight };
      });
      assert.ok(r.width > 0 && r.height > 0 && r.x >= -1 && r.y >= -1 && r.right <= r.vw + 1 && r.bottom <= r.vh + 1, `${label}: ${JSON.stringify(r)}`);
    };
    try {
      const report = { id: 'visual-fixture', report_scope: 'team', label: 'June 2026 delivery review', masked: true, created_at: '2026-07-01T09:00:00Z' };
      await page.route('**/api/reports?**', (route) => route.fulfill({ json: { reports: [report] } }));
      await page.route('**/api/reports/active?**', (route) => route.fulfill({ json: { active: [] } }));
      await page.route('**/api/report/html?**', (route) => route.fulfill({ json: { ...report, html: '<!doctype html><html><body style="font:16px system-ui;padding:16px"><h1>Delivery review</h1><p>Names are masked. Review the source limitations before sharing.</p><h2>Completed work</h2><p>Six changes completed. Two require follow-up.</p></body></html>' } }));
      let submittedComments = 0;
      await page.route('**/api/report/comments?**', (route) => {
        if (route.request().method() !== 'GET') submittedComments++;
        return route.fulfill({ json: { comments: [{ author: 'Reviewer', text: 'Confirm the incomplete deployment sample before sharing.', at: '2026-07-01T10:00:00Z' }] } });
      });
      await page.getByRole('tab', { name: 'Reports', exact: true }).click();
      const open = page.locator('[data-rep="visual-fixture"]');
      await open.press('Enter');
      await page.frameLocator('.rv-frame').getByRole('heading', { name: 'Delivery review', exact: true }).waitFor();
      await inViewport(page.locator('.rv-modal'), 'report dialog');
      await inViewport(page.locator('#rv-dl'), 'report Download action');
      await capture('report');
      const comment = page.getByRole('textbox', { name: 'Add a comment', exact: true });
      await comment.fill('Unsubmitted review note');
      await comment.scrollIntoViewIfNeeded();
      await inViewport(comment, 'reachable comment field');
      await page.locator('#rv-dl').press('Enter');
      const confirmation = page.getByRole('dialog', { name: 'Download this report?', exact: true });
      await confirmation.waitFor();
      assert.match(await confirmation.innerText(), /Anyone you forward it to/);
      assert.match(await confirmation.innerText(), /names are masked/);
      assert.equal(await page.locator('.rv-modal').evaluate((el) => el.closest('.modal-backdrop').inert), true);
      await inViewport(confirmation, 'nested Download confirmation');
      const download = confirmation.getByRole('button', { name: 'Download', exact: true });
      await download.focus();
      await page.keyboard.press('Tab');
      assert.equal(await confirmation.getByRole('button', { name: 'Cancel', exact: true }).evaluate((el) => el === document.activeElement), true, 'Tab wraps inside the top layer');
      await capture('report-download');
      await page.keyboard.press('Escape');
      await confirmation.waitFor({ state: 'detached' });
      assert.equal(await page.locator('.modal-backdrop').count(), 1);
      assert.equal(await comment.inputValue(), 'Unsubmitted review note');
      assert.equal(await page.locator('#rv-dl').evaluate((el) => el === document.activeElement), true);
      await page.keyboard.press('Escape');
      await page.locator('.rv-modal').waitFor({ state: 'detached' });
      assert.equal(await open.evaluate((el) => el === document.activeElement), true, 'report returns focus to its actual list trigger');
      assert.equal(submittedComments, 0);

      await page.getByRole('tab', { name: 'Overview', exact: true }).click();
      let state = 'running';
      let unavailable = false;
      let stops = 0;
      await page.route('**/api/scan', (route) => route.fulfill({ json: { started: true } }));
      await page.route('**/api/scan/status?**', (route) => route.fulfill(unavailable
        ? { status: 503, json: { error: 'Synthetic status interruption' } }
        : { json: { state, step: 'git: reading repository history', fetched: 2, total: 8, project: 'TP' } }));
      await page.route('**/api/scan/stop', (route) => {
        stops++; state = 'stopping';
        return route.fulfill({ json: { stopping: true } });
      });
      await page.locator('#scan').click();
      await page.waitForFunction(() => document.querySelector('#scan-status')?.textContent.includes('2/8'));
      const scanStatus = page.locator('#scan-status');
      assert.equal(await scanStatus.getAttribute('role'), 'status');
      assert.equal(await scanStatus.getAttribute('aria-live'), 'polite');
      assert.equal(await scanStatus.getByRole('list', { name: 'Scan steps', exact: true }).count(), 1);
      assert.match(await scanStatus.innerText(), /git.*in progress/s, 'current phase is conveyed in text, not colour alone');
      unavailable = true;
      await page.locator('#scan-retry').waitFor({ timeout: 5000 });
      assert.match(await page.locator('#scan-status').innerText(), /2\/8/);
      assert.match(await page.locator('#scan-status').innerText(), /status unavailable/i);
      await inViewport(page.locator('#scan-retry'), 'stale-status Retry');
      await capture('scan-stale');
      unavailable = false;
      await page.locator('#scan-retry').press('Enter');
      await page.locator('#scan-retry').waitFor({ state: 'detached' });
      await page.locator('#scan-stop').press('Enter');
      await page.waitForFunction(() => document.querySelector('#scan-stop')?.textContent.includes('Stopping'));
      assert.equal(await page.locator('#scan-stop').isDisabled(), true);
      assert.equal(stops, 1);
      await inViewport(page.locator('#scan-stop'), 'stopping state');
      await capture('scan-stopping');
      state = 'stopped';
      await page.waitForFunction(() => document.querySelector('#scan-status')?.textContent.includes('Completed results were kept'));
      await page.waitForFunction(() => !document.querySelector('#scan')?.disabled);
      await capture('scan-stopped');
      const { sw, cw } = await noHScroll(page);
      assert.ok(sw <= cw + 1, 'transient flow leaves no horizontal page overflow');
      assert.deepEqual(dialogs, []);
      assert.deepEqual(errors, []);
    } finally { await ctx.close(); }
  });
}

test('quality: report opening owns focus while its HTML is still loading', async (t) => {
  if (skipReason) return t.skip(skipReason);
  const { ctx, page } = await openUi({ width: 1280, scheme: 'light' });
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  try {
    const report = { id: 'focus-fixture', report_scope: 'team', label: 'Focus fixture', masked: true, created_at: '2026-07-01T09:00:00Z' };
    await page.route('**/api/reports?**', (route) => route.fulfill({ json: { reports: [report] } }));
    await page.route('**/api/reports/active?**', (route) => route.fulfill({ json: { active: [] } }));
    await page.route('**/api/report/html?**', async (route) => {
      await pending;
      await route.fulfill({ json: { ...report, html: '<h1>Loaded report</h1>' } }).catch(() => {});
    });
    await page.route('**/api/report/comments?**', (route) => route.fulfill({ json: { comments: [] } }));
    await page.getByRole('tab', { name: 'Reports', exact: true }).click();
    await page.locator('[data-rep="focus-fixture"]').press('Enter');
    await page.locator('#rv-close').waitFor();
    assert.equal(await page.locator('#rv-dl').isDisabled(), true, 'initial Download control remains disabled during loading');
    const focus = await page.locator('.rv-modal').evaluate((el) => ({
      owned: el.contains(document.activeElement),
      id: document.activeElement?.id,
      tag: document.activeElement?.tagName,
    }));
    assert.equal(focus.owned, true, `opening dialog must own focus before async content loads: ${JSON.stringify(focus)}`);
    release();
    await page.frameLocator('.rv-frame').getByRole('heading', { name: 'Loaded report', exact: true }).waitFor();
    assert.equal(await page.locator('.rv-modal').evaluate((el) => el.contains(document.activeElement)), true);
    await page.keyboard.press('Escape');
    await page.locator('.rv-modal').waitFor({ state: 'detached' });
    assert.equal(await page.locator('[data-rep="focus-fixture"]').evaluate((el) => el === document.activeElement), true);
  } finally { release(); await ctx.close(); }
});

// Exercise the production renderer inside the production sandboxed viewer, not
// a miniature HTML stand-in. Synthetic inputs deliberately mix measured values,
// missing phase data, a weak-input caveat, long names and dense ticket tables.
function populatedReviewReport() {
  const { buildReportModel, renderReport } = require('../lib/reportmodel.js');
  const names = ['Alex Example — Platform Reliability', 'Blair Sample — Developer Experience', 'Casey Tester — Delivery Systems'];
  const model = buildReportModel({
    scope: 'Platform team',
    jiraBase: 'https://example.invalid/',
    period: { start: '2026-07-01', end: '2026-09-30' },
    compare: { label: 'Q2', kpis: { throughput: 40 } },
    meta: { title: 'Q3 platform delivery review', data_as_of: '2026-10-01T08:00:00Z', ruler_version: 'r3' },
    kpis: [{ id: 'throughput', label: 'Tickets delivered', value: 46, better: 'up' }, { id: 'pace', label: 'Points per capacity day', value: 0.8, kind: 'ratio', capacity_days: 120 }],
    phases: {
      rows: [{ phase: 'design', median_days: null, tickets_tracked: 0, tickets_total: 30 }, { phase: 'dev', median_days: 3.5, p85_days: 8, total_days: 140, tickets_tracked: 30, tickets_total: 30 }, { phase: 'review', median_days: 0.8, tickets_tracked: 25, tickets_total: 30 }],
      tickets: names.map((person, i) => ({ key: `SAMPLE-${i + 1}`, title: 'Improve reliability of the deployment approval workflow', person, design: null, dev: i + 2, review: 0.5 })),
    },
    dora: { deployments: 12, deploy_frequency_per_week: 0.9, change_failure_rate: 0.17 },
    pr_flow: { prs: 50, pickup_hours_median: 6 },
    flow: { wip_avg: 7, throughput: [{ week: '2026-W27', count: 4 }], investment: [{ label: 'Story', share: 0.6 }] },
    capacity: names.map(person => ({ person, working_days: 64, time_off_days: 4, capacity_days: 60, delivered_points: 40 })),
    guardrails: [{ level: 'warn', metric: 'estimates', message: 'Only 40% of tickets have an estimate. Treat comparisons as incomplete.' }],
    people: names.map((name, i) => ({ name, role: 'Engineer', capacity_days: 60, delivered_points: 40, design_days: null, dev_days: i + 2, tickets: [{ key: `SAMPLE-${i + 1}`, title: 'Improve the deployment approval workflow', status: 'Done', points: 5 }], notes: ['Review the missing design evidence before comparing individual delivery.'] })),
  });
  return renderReport(model, { summary: 'Delivery increased from 40 to 46 tickets. Estimate coverage remains incomplete; discuss the missing inputs before drawing conclusions.', strengths: ['Review time is tracked for 25 of 30 tickets.'], goals: ['Improve estimate coverage before the next review.'] });
}

for (const width of [390, 1280]) for (const scheme of ['light', 'dark']) {
  test(`quality populated report @${width}px ${scheme}: hierarchy, accessible data and keyboard navigation`, async (t) => {
    if (skipReason) return t.skip(skipReason);
    const { ctx, page, dialogs, errors } = await openUi({ width, scheme });
    const capture = async (name) => {
      if (!process.env.OTTO_TP_SCREENSHOT_DIR) return;
      fs.mkdirSync(process.env.OTTO_TP_SCREENSHOT_DIR, { recursive: true });
      await page.screenshot({ path: path.join(process.env.OTTO_TP_SCREENSHOT_DIR, `populated-${name}-${width}-${scheme}.png`), animations: 'disabled' });
    };
    try {
      const report = { id: 'populated-fixture', report_scope: 'team', label: 'Q3 platform delivery review', masked: false, created_at: '2026-10-01T09:00:00Z' };
      await page.route('**/api/reports?**', route => route.fulfill({ json: { reports: [report] } }));
      await page.route('**/api/reports/active?**', route => route.fulfill({ json: { active: [] } }));
      const originalHtml = populatedReviewReport();
      await page.route('**/api/report/html?**', route => route.fulfill({ json: { ...report, html: originalHtml } }));
      await page.route('**/api/report/comments?**', route => route.fulfill({ json: { comments: [] } }));
      await page.getByRole('tab', { name: 'Reports', exact: true }).click();
      const open = page.locator('[data-rep="populated-fixture"]');
      await open.press('Enter');
      await page.emulateMedia({ reducedMotion: 'reduce' });
      const frame = page.frameLocator('.rv-frame');
      await frame.getByRole('heading', { name: report.label, level: 1, exact: true }).waitFor();
      assert.equal(await page.locator('.rv-frame').getAttribute('title'), `Team — ${report.label}`);
      assert.equal(await page.locator('.rv-frame').getAttribute('sandbox'), 'allow-popups', 'report scripts remain disabled');
      assert.equal(await frame.getByRole('heading', { level: 1 }).count(), 1);
      assert.equal(await frame.getByRole('main').count(), 1);
      assert.equal(await frame.getByRole('navigation', { name: 'Contents', exact: true }).count(), 1);
      const semantics = await frame.locator('body').evaluate(el => {
        const warning = el.querySelector('[role="alert"]');
        const firstMetric = el.querySelector('#kpis');
        return {
          caveatBeforeMetrics: !!(warning.compareDocumentPosition(firstMetric) & Node.DOCUMENT_POSITION_FOLLOWING),
          caveat: warning.textContent,
          unnamedSections: [...el.querySelectorAll('main > section')].filter(section => !document.getElementById(section.getAttribute('aria-labelledby'))?.textContent.trim()).length,
          headings: [...el.querySelectorAll('main h2, main h3')].map(h => Number(h.tagName[1])),
          scrollWidth: document.documentElement.scrollWidth, viewport: document.documentElement.clientWidth,
        };
      });
      assert.equal(semantics.caveatBeforeMetrics, true, 'input limitations precede numerical claims in reading order');
      assert.match(semantics.caveat, /Only 40%.*incomplete/s);
      assert.equal(semantics.unnamedSections, 0);
      assert.equal(semantics.headings[0], 2);
      assert.ok(semantics.headings.every((level, i, a) => i === 0 || level <= a[i - 1] + 1));
      assert.ok(semantics.scrollWidth <= semantics.viewport + 1, `report itself must reflow: ${JSON.stringify(semantics)}`);
      await capture('top');

      // The actual link and native disclosure work even in the script-disabled
      // report sandbox. No programmatic DOM scrolling substitutes for the link.
      const phaseLink = frame.getByRole('navigation', { name: 'Contents' }).getByRole('link', { name: 'Where the time goes', exact: true });
      const beforeJump = await phaseLink.evaluate(el => ({ url: location.href, base: document.baseURI, resolvedLink: el.href }));
      await phaseLink.press('Enter');
      const afterJump = await frame.locator('body').evaluate(() => ({ url: location.href, base: document.baseURI, title: document.title, heading: document.querySelector('h1')?.textContent, hasPhase: !!document.querySelector('#phases') }));
      assert.equal(afterJump.hasPhase, true, `Contents must keep the report document: ${JSON.stringify({ beforeJump, afterJump })}`);
      const phase = frame.locator('#phases');
      const headingPosition = await phase.evaluate(el => ({ y: el.getBoundingClientRect().top, vh: innerHeight }));
      assert.ok(headingPosition.y >= -1 && headingPosition.y < headingPosition.vh, `Contents reaches phase section: ${JSON.stringify(headingPosition)}`);
      assert.equal(await phase.getByRole('img', { name: /Team cycle time by phase.*Not tracked: Design/s }).count(), 1, 'chart has a descriptive accessible name including missing evidence');
      const chart = frame.locator('#ch-phases');
      const disclosure = chart.locator('summary');
      await disclosure.press('Enter');
      assert.equal(await chart.locator('details').getAttribute('open'), '');
      const table = chart.getByRole('table');
      assert.equal(await table.getByRole('columnheader', { name: 'Design (d)', exact: true }).count(), 1);
      assert.equal(await table.getByRole('columnheader', { name: 'Dev (d)', exact: true }).count(), 1);
      assert.match(await table.innerText(), /not tracked/);
      assert.match(await table.innerText(), /3[.,]5/);
      assert.equal(await table.locator('th:not([scope="col"]):not([scope="row"])').count(), 0);
      assert.equal(await table.getByRole('rowheader', { name: 'This period (medians)', exact: true }).count(), 1);
      await capture('phases');

      await frame.getByRole('navigation', { name: 'Contents' }).getByRole('link', { name: 'People', exact: true }).press('Enter');
      const people = frame.locator('#people');
      assert.equal(await people.getByRole('heading', { level: 3 }).count(), 3);
      const person = people.locator('.person').first();
      await person.locator('summary', { hasText: 'Tickets (1)' }).press('Enter');
      assert.equal(await person.locator('details').getAttribute('open'), '');
      assert.equal(await people.getByRole('columnheader', { name: 'Status', exact: true }).count(), 1);
      assert.equal(await people.getByRole('link', { name: 'SAMPLE-1', exact: true }).evaluate(el => el.href), 'https://example.invalid/browse/SAMPLE-1', 'absolute ticket destinations are preserved without visiting them');
      const overflow = await frame.locator('body').evaluate(() => ({ sw: document.documentElement.scrollWidth, vw: document.documentElement.clientWidth, badTables: [...document.querySelectorAll('.tbl-wrap')].filter(el => el.scrollWidth > el.clientWidth + 1 && getComputedStyle(el).overflowX !== 'auto').length }));
      assert.ok(overflow.sw <= overflow.vw + 1, `expanded populated report stays contained: ${JSON.stringify(overflow)}`);
      assert.equal(overflow.badTables, 0, 'wide tables scroll within their own region');
      await capture('people');
      const comment = page.getByRole('textbox', { name: 'Add a comment', exact: true });
      await comment.fill('Draft after reading source limitations');
      await page.locator('#rv-dl').press('Enter');
      await page.getByRole('dialog', { name: 'Download this report?', exact: true }).waitFor();
      await page.keyboard.press('Escape');
      assert.equal(await comment.inputValue(), 'Draft after reading source limitations');
      assert.equal(await page.locator('#rv-dl').evaluate(el => el === document.activeElement), true);
      await page.locator('#rv-dl').press('Enter');
      const savedFile = page.waitForEvent('download');
      await page.getByRole('dialog', { name: 'Download this report?', exact: true }).getByRole('button', { name: 'Download', exact: true }).press('Enter');
      const download = await savedFile;
      const stream = await download.createReadStream();
      const chunks = [];
      for await (const chunk of stream) chunks.push(chunk);
      assert.equal(Buffer.concat(chunks).toString('utf8'), originalHtml, 'local download keeps the original standalone report byte-for-byte');
      await page.keyboard.press('Escape');
      await page.locator('.rv-modal').waitFor({ state: 'detached' });
      assert.equal(await open.evaluate(el => el === document.activeElement), true);
      assert.deepEqual(dialogs, []);
      assert.deepEqual(errors, []);
    } finally { await ctx.close(); }
  });
}
