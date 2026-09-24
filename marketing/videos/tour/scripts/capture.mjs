// Capture the tour film's footage from the NEW app, safely:
//   • an ISOLATED throwaway ottod (temp data dir + fake HOME, its own port),
//   • throwaway demo databases in Docker (MariaDB, MongoDB, Redpanda),
//   • fictional seed data through the daemon's HTTP API (scripts/lib/seed.mjs),
//   • Playwright (one worker) driving the production UI build.
// Stills (JPEG, 3200×1800) and flow clips (H.264, 2560×1440) land in
// public/capture/. Everything started here is stopped at the end.
//
//   node scripts/capture.mjs                    # everything
//   node scripts/capture.mjs --only home,git    # a subset of shots
//   node scripts/capture.mjs --serve            # bring up + seed, keep running (iterate with --attach)
//   node scripts/capture.mjs --attach --only db # reuse a --serve stack
//
// Env: OTTO_E2E_BIN (daemon binary; default <repo>/target/debug/ottod),
//      OTTO_E2E_PORT (7811), OTTO_E2E_PW_PORT (5211), OTTO_TOUR_DIST (ui/dist),
//      OTTO_TOUR_NO_DOCKER=1 to skip the demo databases.
import { chromium } from 'playwright';
import { execFileSync, execSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { client, startStack, storageState, UI, PORT } from './lib/stack.mjs';
import { seedAll, clearCredentialNotices } from './lib/seed.mjs';
import { writeTranscripts } from './lib/transcripts.mjs';
import { CURSOR_SCRIPT } from './lib/recorder.mjs';
import { SHOTS } from './lib/shots.mjs';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(tour, '../../..');
const OUT = join(tour, 'public/capture');
const CACHE = join(tour, '.cache');
mkdirSync(OUT, { recursive: true });
mkdirSync(CACHE, { recursive: true });

const argv = process.argv.slice(2);
const flag = (f) => argv.includes(`--${f}`);
const opt = (f) => {
  const i = argv.indexOf(`--${f}`);
  return i >= 0 ? argv[i + 1] : undefined;
};
const only = opt('only')?.split(',').map((s) => s.trim()).filter(Boolean);
const bin = process.env.OTTO_E2E_BIN ?? join(repo, 'target/debug/ottod');
const dist = process.env.OTTO_TOUR_DIST ?? join(repo, 'ui/dist');
const META = join(CACHE, 'stack.json');

// ── demo containers ─────────────────────────────────────────────────────────
const DOCKER = [
  {
    key: 'mysql', name: 'otto-tour-mariadb', image: 'mariadb:11.8.4', port: 13811, inner: 3306,
    args: ['-e', 'MARIADB_ROOT_PASSWORD=rootpw', '-e', 'MARIADB_DATABASE=shopdb', '-e', 'MARIADB_USER=otto', '-e', 'MARIADB_PASSWORD=ottopw', '-v', `${join(tour, 'scripts/demo-db/mysql')}:/docker-entrypoint-initdb.d:ro`],
    ready: ['mariadb', '-uotto', '-pottopw', 'shopdb', '-e', 'SELECT COUNT(*) FROM order_items'],
  },
  {
    key: 'mongo', name: 'otto-tour-mongo', image: 'mongo:8.2', port: 17811, inner: 27017,
    args: ['-e', 'MONGO_INITDB_ROOT_USERNAME=otto', '-e', 'MONGO_INITDB_ROOT_PASSWORD=ottopw', '-e', 'MONGO_INITDB_DATABASE=shopdb', '-v', `${join(tour, 'scripts/demo-db/mongo')}:/docker-entrypoint-initdb.d:ro`],
    ready: ['mongosh', '-u', 'otto', '-p', 'ottopw', '--authenticationDatabase', 'admin', '--quiet', 'shopdb', '--eval', 'db.orders.countDocuments() > 0 || quit(1)'],
  },
  {
    key: 'kafka', name: 'otto-tour-redpanda', image: 'redpandadata/redpanda:v24.2.7', port: 19811, inner: 19811,
    args: [],
    cmd: ['redpanda', 'start', '--mode=dev-container', '--smp=1', '--memory=512M', '--default-log-level=warn', '--kafka-addr=PLAINTEXT://0.0.0.0:19811', '--advertise-kafka-addr=PLAINTEXT://127.0.0.1:19811'],
    ready: ['rpk', 'cluster', 'health', '-X', 'brokers=127.0.0.1:19811'],
  },
];

function docker(args, opts = {}) {
  return execFileSync('docker', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts });
}
function hasImage(image) {
  try {
    return docker(['images', '-q', image]).trim().length > 0;
  } catch {
    return false;
  }
}
async function startContainers() {
  const ports = {};
  if (process.env.OTTO_TOUR_NO_DOCKER === '1') return ports;
  try {
    docker(['ps']);
  } catch {
    console.warn('[tour] docker is not running — skipping the demo databases');
    return ports;
  }
  for (const c of DOCKER) {
    if (!hasImage(c.image)) {
      console.warn(`[tour] image ${c.image} not present locally — skipping ${c.key} (docker pull ${c.image} to include it)`);
      continue;
    }
    try {
      docker(['rm', '-f', c.name]);
    } catch {
      /* not there */
    }
    docker(['run', '-d', '--rm', '--name', c.name, '--cpus', '1', '-p', `127.0.0.1:${c.port}:${c.inner}`, ...c.args, c.image, ...(c.cmd ?? [])]);
    console.log(`[tour] started ${c.name} on 127.0.0.1:${c.port}`);
  }
  for (const c of DOCKER) {
    if (!hasImage(c.image)) continue;
    const deadline = Date.now() + 150_000;
    let ok = false;
    // Init scripts restart the server once; require two consecutive passes.
    let passes = 0;
    while (Date.now() < deadline) {
      try {
        docker(['exec', c.name, ...c.ready]);
        passes++;
        if (passes >= 2) {
          ok = true;
          break;
        }
      } catch {
        passes = 0;
      }
      await new Promise((r) => setTimeout(r, 2000));
    }
    if (ok) ports[c.key] = c.port;
    else console.warn(`[tour] ${c.name} never became ready — continuing without it`);
  }
  return ports;
}
function stopContainers() {
  for (const c of DOCKER) {
    try {
      docker(['rm', '-f', c.name]);
    } catch {
      /* gone */
    }
  }
}

// ── stack up / down ─────────────────────────────────────────────────────────
async function bringUp() {
  const dockerPorts = await startContainers();
  const stack = await startStack({
    bin,
    dist,
    log: join(CACHE, 'daemon.log'),
    prepare: ({ home }) => {
      writeTranscripts(home, { cwds: [join(home, 'Code/acme-checkout'), join(home, 'Code/acme-web')] });
    },
  });
  const api = client(stack.token);
  console.log('[tour] seeding demo data…');
  const ids = await seedAll(api, { dataDir: stack.dataDir, home: stack.home, fakeBin: stack.fakeBin, repoRoot: repo, docker: dockerPorts });
  writeFileSync(META, JSON.stringify({ token: stack.token, dataDir: stack.dataDir, home: stack.home, pid: stack.pid, ids, docker: dockerPorts }, null, 2));
  console.log('[tour] seeded:', Object.keys(ids).join(', '));
  return { stack, api, ids, token: stack.token, meta: JSON.parse(readFileSync(META, 'utf8')) };
}

// ── browser ─────────────────────────────────────────────────────────────────
export const VIEW = { width: 1600, height: 900 };

async function newContext(browser, meta, { scheme = 'dark', viewport = VIEW, dpr = 2, extra = [], mobile = false } = {}) {
  const views = [
    {
      id: 'tourview1',
      name: 'Overview',
      boxes: [
        { id: 'b1', kind: 'sessions', w: 5, h: 4, config: {} },
        { id: 'b2', kind: 'mission-control', w: 7, h: 4, config: {} },
        ...(meta.ids.dashboard ? [{ id: 'b3', kind: 'db-dashboard', w: 7, h: 5, config: { dashboardId: meta.ids.dashboard } }] : []),
        { id: 'b4', kind: 'usage', w: 5, h: 5, config: {} },
        { id: 'b5', kind: 'insights', w: 12, h: 4, config: {} },
      ],
    },
    { id: 'tourview2', name: 'Release', boxes: [{ id: 'c1', kind: 'mission-control', w: 12, h: 5, config: {} }] },
  ];
  const ctx = await browser.newContext({
    viewport,
    deviceScaleFactor: dpr,
    colorScheme: scheme,
    isMobile: mobile,
    hasTouch: mobile,
    storageState: storageState(meta.token, [
      { name: 'otto_workspace', value: meta.ids.ws },
      { name: 'otto_firstrun_dismissed', value: '1' },
      { name: 'otto_rail_expanded', value: '1' },
      { name: 'otto_home_views', value: JSON.stringify(views) },
      { name: 'otto_orch_fallback', value: '0' },
      ...extra,
    ]),
  });
  await ctx.addInitScript(CURSOR_SCRIPT);
  // Hide scrollbars + the blinking caret jitter for cleaner footage.
  await ctx.addInitScript(() => {
    const css = '::-webkit-scrollbar{width:0!important;height:0!important}';
    const add = () => {
      const s = document.createElement('style');
      s.textContent = css;
      document.head.appendChild(s);
    };
    if (document.head) add();
    else addEventListener('DOMContentLoaded', add);
  });
  return ctx;
}

// ── main ────────────────────────────────────────────────────────────────────
let up = null;
let meta;
if (flag('attach')) {
  if (!existsSync(META)) throw new Error('[tour] --attach needs a running `--serve` stack (.cache/stack.json)');
  meta = JSON.parse(readFileSync(META, 'utf8'));
} else {
  up = await bringUp();
  meta = up.meta;
}
const cleanup = async () => {
  if (up) {
    await up.stack.stop();
    stopContainers();
    rmSync(META, { force: true });
  }
};
process.on('SIGINT', async () => {
  await cleanup();
  process.exit(130);
});
process.on('SIGTERM', async () => {
  await cleanup();
  process.exit(143);
});

if (flag('serve')) {
  console.log(`[tour] serving: UI ${UI}  daemon :${PORT}  (Ctrl+C to stop)`);
  setInterval(() => {}, 1 << 30);
} else {
  const api = client(meta.token);
  const browser = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
  const failures = [];
  try {
    const names = only ?? Object.keys(SHOTS);
    for (const name of names) {
      const fn = SHOTS[name];
      if (!fn) {
        console.warn(`[tour] unknown shot ${name}`);
        continue;
      }
      await clearCredentialNotices(api);
      const t0 = Date.now();
      try {
        await fn({ browser, meta, api, out: OUT, newContext: (o) => newContext(browser, meta, o) });
        console.log(`[tour] ✓ ${name} (${((Date.now() - t0) / 1000).toFixed(1)}s)`);
      } catch (e) {
        failures.push(name);
        console.error(`[tour] ✗ ${name}: ${e.message.split('\n')[0]}`);
      }
    }
  } finally {
    await browser.close();
    await cleanup();
  }
  if (failures.length) {
    console.error(`[tour] failed shots: ${failures.join(', ')}`);
    process.exitCode = 1;
  }
  try {
    const size = execSync(`du -sh "${OUT}"`, { encoding: 'utf8' }).split('\t')[0];
    console.log(`[tour] captures: ${size} in public/capture`);
  } catch {
    /* ignore */
  }
}
