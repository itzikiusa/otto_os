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
  if (!pw) return;
  browser = await launchChromium(pw);
  if (!browser) { skipReason = 'no Chromium binary available for playwright'; return; }

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
    await page.evaluate(() => (TP.app.goTab ? TP.app.goTab('estimates') : document.querySelector('[data-tab="estimates"]').click()));
    const fix = await page.waitForSelector('[data-fix]', { timeout: 10000 }).catch(() => null);
    if (!fix) return t.skip('no "Correct" action rendered for this fixture (no estimate misses)');
    const label = await fix.getAttribute('aria-label');
    const key = (label.match(/[A-Z][A-Z0-9]+-\d+/) || [])[0];
    assert.ok(key, 'Correct button names its ticket');
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
