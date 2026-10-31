// The shot list: each entry drives the UI to one state and writes stills
// (<name>.jpg) and/or flow clips (<name>.mp4) into public/capture/. The film's
// scene specs (src/scenes.ts) reference these file names.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { UI, API } from './stack.mjs';
import { record } from './recorder.mjs';

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function open(c, route, { settle = 2500, ready = '.shell', ...ctxOpts } = {}) {
  const context = await c.newContext(ctxOpts);
  const page = await context.newPage();
  page.on('pageerror', (e) => console.log(`  [pageerror] ${e.message.slice(0, 160)}`));
  await page.goto(`${UI}/#/${route}`);
  // #/tray and #/bar render without the app shell; they pass their own `ready`.
  await page.locator(ready).first().waitFor({ timeout: 60_000 }).catch(() => {});
  await wait(settle);
  return { context, page };
}

async function still(page, c, name, opts = {}) {
  await page.mouse.move(-20, -20).catch(() => {});
  await wait(150);
  await page.screenshot({ path: join(c.out, `${name}.jpg`), type: 'jpeg', quality: 90, ...opts });
  console.log(`  still ${name}.jpg`);
}

/** Smoothly glide the (visible) cursor to an element's center, then click it. */
async function glide(page, locator, { steps = 26, pause = 220, click = true } = {}) {
  await locator.waitFor({ timeout: 15_000 });
  const box = await locator.boundingBox();
  if (!box) throw new Error('glide: element has no box');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps });
  await wait(pause);
  if (click) await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
}

async function typeSlow(page, text, delay = 55) {
  for (const ch of text) {
    await page.keyboard.type(ch);
    await wait(delay + Math.random() * 35);
  }
}

/** Best-effort click: logs instead of failing the whole shot. */
async function tryClick(locator, label) {
  try {
    await locator.first().click({ timeout: 6000 });
    return true;
  } catch {
    console.log(`  (skip: ${label})`);
    return false;
  }
}

const route = (name, path, settle = 3000, scheme = 'dark') => async (c) => {
  const { context, page } = await open(c, path, { settle, scheme });
  await still(page, c, name);
  await context.close();
};

const api = async (c, method, path, body) => {
  const r = await fetch(`${API}${path}`, {
    method,
    headers: { Authorization: `Bearer ${c.meta.token}`, 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  });
  return r.ok ? r.json().catch(() => null) : null;
};

export const SHOTS = {
  // ── Shell ─────────────────────────────────────────────────────────────────
  home: async (c) => {
    const { context, page } = await open(c, 'home', { settle: 6000 });
    await still(page, c, 'home');
    await context.close();
  },
  'home-light': async (c) => {
    const { context, page } = await open(c, 'home', { settle: 6000, scheme: 'light' });
    await still(page, c, 'home-light');
    await context.close();
  },
  // ⌘K: focus the bar, type a command, run it (it docks off Home), switch spaces.
  cmdk: async (c) => {
    const { context, page } = await open(c, 'home', { settle: 5000 });
    const rec = await record(page);
    await wait(900);
    await page.keyboard.press('Meta+k');
    await wait(700);
    await typeSlow(page, 'go to threads', 80);
    await wait(1300);
    await still(page, c, 'cmdk-typed');
    await page.keyboard.press('Enter');
    await wait(2600);
    // Docked as a status-bar chip on a work page — reopen it and ask.
    await page.keyboard.press('Meta+k');
    await wait(800);
    await typeSlow(page, 'what needs me today?', 70);
    await wait(1200);
    await still(page, c, 'cmdk-ask');
    for (const k of ['Control+2', 'Control+3', 'Control+1']) {
      await page.keyboard.press(k);
      await wait(650);
    }
    await wait(900);
    await page.keyboard.press('Escape');
    await wait(1200);
    await rec.stop(join(c.out, 'cmdk.mp4'));
    await context.close();
  },
  // The desktop-app surfaces rendered from their own routes.
  desktop: async (c) => {
    {
      const { context, page } = await open(c, 'tray', { settle: 3500, ready: 'body', viewport: { width: 540, height: 390 } });
      await still(page, c, 'desktop-tray');
      await context.close();
    }
    {
      const { context, page } = await open(c, 'bar', { settle: 3500, ready: '[data-testid="floating-bar"]', viewport: { width: 1000, height: 420 } });
      await page.keyboard.press('Meta+k').catch(() => {});
      const input = page.locator('input').first();
      await input.click().catch(() => {});
      await typeSlow(page, 'review PR 214', 40);
      await wait(1800);
      // Crop to the rendered bar (the window grows upward from the pill).
      const bar = page.getByTestId('floating-bar');
      const box = await bar.boundingBox().catch(() => null);
      await page.screenshot({ path: join(c.out, 'desktop-bar.png'), type: 'png', ...(box ? { clip: box } : {}) });
      console.log('  still desktop-bar.png');
      await context.close();
    }
    {
      const s = c.meta.ids.sessions[0];
      const { context, page } = await open(c, `agents/${s}`, { settle: 4000, viewport: { width: 900, height: 532 }, extra: [{ name: 'otto_rail_expanded', value: '0' }] });
      await still(page, c, 'desktop-popout');
      await context.close();
    }
  },
  // ── Work ──────────────────────────────────────────────────────────────────
  agents: async (c) => {
    const [s0] = c.meta.ids.sessions;
    const { context, page } = await open(c, `agents/${s0}`, { settle: 5000 });
    await still(page, c, 'agents-tab');
    const rec = await record(page);
    await wait(600);
    await glide(page, page.getByRole('button', { name: 'Tiled view' }));
    await wait(2600);
    await still(page, c, 'agents-tiled');
    await page.keyboard.press('Meta+Shift+b');
    await wait(1500);
    await page.keyboard.type('Rebase on main and re-run the tests', { delay: 35 });
    await wait(1200);
    await still(page, c, 'agents-broadcast');
    await rec.stop(join(c.out, 'agents.mp4'));
    await page.keyboard.press('Escape');
    await context.close();
  },
  threads: async (c) => {
    const [s0, s1, s2] = c.meta.ids.sessions;
    const primary = s1 ?? s0;
    const secondary = s2 ?? s1;
    const { context, page } = await open(c, `threads/${primary}`, {
      settle: 4000,
      extra: [
        { name: 'otto_rail_expanded', value: '0' },
        {
          name: 'otto_threads_v1',
          value: JSON.stringify({
            pinnedIds: s0 ? [s0] : [],
            paneIds: [primary, secondary].filter(Boolean),
            focusedSessionId: primary ?? null,
            layoutMode: 'split',
            sidebarOpen: true,
            inspectorOpen: true,
            inspectorTab: 'files',
            openFilesByWorkspace: {
              [c.meta.ids.ws]: {
                tabs: [
                  { path: 'src/middleware/rateLimit.ts', pinned: true },
                  { path: 'docs/rate-limits.md' },
                ],
                activeFilePath: 'src/middleware/rateLimit.ts',
              },
            },
          }),
        },
      ],
    });
    await wait(2000);
    await still(page, c, 'threads-split');

    await tryClick(page.getByTestId('threads-tab-peers'), 'peers tab');
    await wait(900);
    const peerInput = page.getByTestId('peers-message-input');
    if (await peerInput.isVisible().catch(() => false)) {
      await peerInput.fill('Rate-limit middleware landed on feat/rate-limit — safe to rebase cart totals and re-run.');
      await tryClick(page.getByTestId('peers-send-btn'), 'send peer relay');
      await wait(1400);
    } else {
      await wait(1200);
    }
    await still(page, c, 'threads-peers');
    await context.close();

    // New Thread stage with expanded main sidebar so Threads' sidebar entry is showcased too.
    {
      const { context: stageCtx, page: stagePage } = await open(c, `threads/${primary}`, {
        settle: 3200,
      });
      await tryClick(stagePage.getByTestId('threads-new-btn'), 'new thread stage');
      await wait(1600);
      await still(stagePage, c, 'threads-stage');
      await stageCtx.close();
    }

    // Also capture Light mode + RTL stills for visual QA and README docs.
    {
      const { context: lightCtx, page: lightPage } = await open(c, `threads/${primary}`, {
        settle: 3500,
        scheme: 'light',
        extra: [
          { name: 'otto_rail_expanded', value: '0' },
          {
            name: 'otto_threads_v1',
            value: JSON.stringify({
              pinnedIds: s0 ? [s0] : [],
              paneIds: [primary, secondary].filter(Boolean),
              focusedSessionId: primary ?? null,
              layoutMode: 'split',
              sidebarOpen: true,
              inspectorOpen: true,
              inspectorTab: 'changes',
            }),
          },
        ],
      });
      await wait(1800);
      await still(lightPage, c, 'threads-light');
      await lightCtx.close();
    }
    {
      const { context: rtlCtx, page: rtlPage } = await open(c, `threads/${primary}`, {
        settle: 3500,
        extra: [
          { name: 'otto_dir', value: 'rtl' },
          { name: 'otto_rail_expanded', value: '0' },
          {
            name: 'otto_threads_v1',
            value: JSON.stringify({
              pinnedIds: s0 ? [s0] : [],
              paneIds: [primary].filter(Boolean),
              focusedSessionId: primary ?? null,
              layoutMode: 'single',
              sidebarOpen: true,
              inspectorOpen: true,
              inspectorTab: 'files',
              openFilesByWorkspace: {
                [c.meta.ids.ws]: {
                  tabs: [{ path: 'docs/rate-limits.md', pinned: true }],
                  activeFilePath: 'docs/rate-limits.md',
                },
              },
            }),
          },
        ],
      });
      await rtlPage.evaluate(() => {
        document.documentElement.setAttribute('dir', 'rtl');
      });
      await wait(1400);
      await still(rtlPage, c, 'threads-rtl');
      await rtlCtx.close();
    }
  },
  // ── Automate ──────────────────────────────────────────────────────────────
  loops: async (c) => {
    const { context, page } = await open(c, 'loops', { settle: 3500 });
    await tryClick(page.getByText('Checkout p95 under 300 ms'), 'loop card');
    await wait(3000);
    await still(page, c, 'loops');
    await context.close();
  },
  workflows: async (c) => {
    const { context, page } = await open(c, c.meta.ids.workflow ? `workflows/${c.meta.ids.workflow}` : 'workflows', { settle: 3500 });
    await tryClick(page.getByText('PR review pipeline', { exact: true }), 'pick workflow');
    await wait(2500);
    await still(page, c, 'workflows');
    await context.close();
  },
  'scheduled-tasks': async (c) => {
    const { context, page } = await open(c, 'scheduled-tasks', { settle: 3000 });
    await still(page, c, 'scheduled-tasks');
    await context.close();
  },
  'personal-agents': route('personal-agents', 'personal-agents', 3500),
  // ── Build ─────────────────────────────────────────────────────────────────
  git: async (c) => {
    const { context, page } = await open(c, `git/${c.meta.ids.repo}/graph`, { settle: 4500 });
    await still(page, c, 'git');
    const rec = await record(page);
    await wait(500);
    await glide(page, page.getByText('// WIP').first());
    await wait(2200);
    await still(page, c, 'git-wip');
    await glide(page, page.getByText('Review', { exact: true }).first());
    await wait(3000);
    await still(page, c, 'git-review');
    await rec.stop(join(c.out, 'git.mp4'));
    await context.close();
  },
  product: route('product', 'product', 3500),
  vault: async (c) => {
    const { context, page } = await open(c, 'vault', { settle: 4000 });
    await tryClick(page.getByText('services', { exact: true }), 'services folder');
    await wait(800);
    await tryClick(page.getByText('checkout-api', { exact: true }).or(page.getByText('Checkout API', { exact: true })), 'note');
    await wait(2200);
    await still(page, c, 'vault');
    if (await tryClick(page.getByRole('button', { name: 'Graph view' }), 'graph view')) {
      await wait(5000);
      await still(page, c, 'vault-graph');
    }
    await context.close();
  },
  design: async (c) => {
    const { context, page } = await open(c, 'design', { settle: 4500 });
    await still(page, c, 'design');
    for (const [id, name, settle] of [
      [c.meta.ids.frame, 'design-frame', 4000],
      [c.meta.ids.brand, 'design-brand', 3500],
      [c.meta.ids.scene3d, 'design-3d', 7000],
      [c.meta.ids.site, 'design-site', 4500],
    ]) {
      if (!id) continue;
      await page.goto(`${UI}/#/design/a/${id}`);
      await wait(settle);
      await still(page, c, name);
    }
    await page.goto(`${UI}/#/design/learned`);
    await wait(3000);
    await still(page, c, 'design-learned');
    await context.close();
  },
  'skills-eval': route('skills-eval', 'skills-eval', 3500),
  // ── Infrastructure ────────────────────────────────────────────────────────
  connections: route('connections', 'connections', 3500),
  'db-builder': async (c) => {
    if (!c.meta.ids.mysql) return console.log('  (no mysql)');
    const { context, page } = await open(c, `database/${c.meta.ids.mysql}`, { settle: 4500 });
    await tryClick(page.locator('.main-tabs .mt', { hasText: 'Builder' }), 'builder tab');
    await page.getByTitle('Add orders to the canvas', { exact: true }).waitFor({ timeout: 20_000 });
    await wait(800);
    const rec = await record(page);
    await wait(500);
    await glide(page, page.getByTitle('Add orders to the canvas', { exact: true }));
    await wait(900);
    await glide(page, page.getByTitle('Add customers to the canvas', { exact: true }));
    await wait(1100);
    await glide(page, page.getByLabel('Select orders.region'));
    await wait(700);
    await glide(page, page.getByRole('button', { name: 'Aggregate' }));
    await wait(700);
    const agg = page.locator('.sel-row').nth(1);
    await agg.getByLabel('Aggregate', { exact: true }).selectOption('SUM');
    await wait(500);
    await agg.getByLabel('Aggregate of').selectOption('orders.total_cents');
    await wait(900);
    const having = page.locator('section[aria-labelledby="qb-having"]');
    await glide(page, having.getByRole('button', { name: 'Condition', exact: true }));
    await wait(600);
    await having.locator('.qb-row.cond').first().getByLabel('Value').fill('20');
    await wait(700);
    await glide(page, page.getByRole('button', { name: 'Sort', exact: true }));
    await wait(1400);
    await still(page, c, 'db-builder');
    await glide(page, page.locator('.sql-actions .btn.primary', { hasText: 'Run' }));
    await wait(3500);
    await still(page, c, 'db-results');
    await rec.stop(join(c.out, 'db-builder.mp4'));
    await context.close();
  },
  'db-mongo': async (c) => {
    if (!c.meta.ids.mongo) return console.log('  (no mongo)');
    const { context, page } = await open(c, `database/${c.meta.ids.mongo}`, { settle: 5000 });
    await tryClick(page.locator('.main-tabs .mt', { hasText: 'Query' }), 'query tab');
    await wait(1200);
    const content = page.locator('.qe-edit .cm-content').first();
    await content.click({ timeout: 15_000 });
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.insertText('db.orders.find({ status: "paid" }).sort({ totalCents: -1 })');
    await wait(500);
    await page.keyboard.press('ControlOrMeta+Enter');
    await wait(4500);
    await still(page, c, 'db-mongo');
    await context.close();
  },
  'db-dashboard': async (c) => {
    if (!c.meta.ids.mysql) return;
    const { context, page } = await open(c, `database/${c.meta.ids.mysql}`, { settle: 4000 });
    await tryClick(page.locator('.main-tabs .mt', { hasText: 'Dashboards' }), 'dashboards tab');
    await wait(5000);
    await still(page, c, 'db-dashboard');
    await context.close();
  },
  brokers: async (c) => {
    const { context, page } = await open(c, c.meta.ids.kafka ? `brokers/${c.meta.ids.kafka}` : 'brokers', { settle: 4000 });
    await tryClick(page.getByText('orders.events').first(), 'topic');
    await wait(2500);
    await tryClick(page.getByRole('tab', { name: /Messages/ }).or(page.getByRole('button', { name: /^Messages/ })), 'messages');
    await wait(3000);
    await still(page, c, 'brokers');
    await context.close();
  },
  aws: route('aws', 'aws', 3500),
  kubernetes: route('kubernetes', 'kubernetes', 3500),
  api: async (c) => {
    const { context, page } = await open(c, 'api', { settle: 3500 });
    await tryClick(page.getByText('Health check', { exact: true }), 'request');
    await wait(1500);
    await tryClick(page.getByRole('button', { name: /^Send/ }), 'send');
    await wait(3000);
    await still(page, c, 'api');
    await context.close();
  },
  browser: async (c) => {
    const { context, page } = await open(c, 'browser', { settle: 3000 });
    // Browser v2 (the default): the address field is labelled "Address".
    const url = page.getByRole('textbox', { name: 'Address' });
    await url.click({ timeout: 15_000 });
    await page.keyboard.type('https://www.rust-lang.org/learn');
    await page.keyboard.press('Enter');
    await wait(7000);
    await still(page, c, 'browser');
    await context.close();
  },
  mcp: async (c) => {
    const { context, page } = await open(c, 'mcp/servers', { settle: 3500 });
    await still(page, c, 'mcp');
    await page.goto(`${UI}/#/mcp/activity`);
    await wait(3000);
    await still(page, c, 'mcp-activity');
    await context.close();
  },
  // ── Insight ───────────────────────────────────────────────────────────────
  insights: route('insights', 'insights', 4000),
  usage: async (c) => {
    const { context, page } = await open(c, 'usage', { settle: 3500 });
    await tryClick(page.getByRole('button', { name: 'All', exact: true }), 'all');
    await wait(4000);
    await still(page, c, 'usage');
    await context.close();
  },
  // ── Everywhere ────────────────────────────────────────────────────────────
  channels: route('channels', 'settings/channels', 3000),
  snip: async (c) => {
    // Snips are PNG-only: grab one from the checkout frame in Design Hall.
    let png;
    {
      const { context, page } = await open(c, c.meta.ids.frame ? `design/a/${c.meta.ids.frame}` : 'design', { settle: 4000, dpr: 1 });
      png = (await page.screenshot({ type: 'png', clip: { x: 160, y: 60, width: 1100, height: 520 } })).toString('base64');
      await context.close();
    }
    const s = await api(c, 'POST', '/snips', { data_b64: png, filename: 'checkout.png' });
    const id = s?.id ?? s?.snip?.id;
    if (!id) return console.log('  (snip create failed)');
    const { context, page } = await open(c, `snip/${id}`, { settle: 4000 });
    // Annotate: box the pay button, point an arrow at it.
    const drag = async (x0, y0, x1, y1) => {
      await page.mouse.move(x0, y0);
      await page.mouse.down();
      await page.mouse.move(x1, y1, { steps: 12 });
      await page.mouse.up();
      await wait(400);
    };
    if (await tryClick(page.getByTitle('Rectangle (R)'), 'box tool')) await drag(455, 612, 882, 688);
    if (await tryClick(page.getByTitle('Arrow (A)'), 'arrow tool')) await drag(1060, 790, 900, 668);
    await wait(800);
    await still(page, c, 'snip');
    await context.close();
  },
  phone: async (c) => {
    const { context, page } = await open(c, 'home', { settle: 6000, viewport: { width: 390, height: 844 }, dpr: 3, mobile: true });
    await still(page, c, 'phone');
    await context.close();
  },
};

export { open, still, glide, typeSlow, wait, record };
