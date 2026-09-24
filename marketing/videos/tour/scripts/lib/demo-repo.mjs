// Build a small, fictional git repository ("acme-checkout") with a believable
// history: feature branches, merges, several authors, and a dirty working tree
// so the Git page shows a WIP row. Everything lives under the capture's temp dir.
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

const AUTHORS = [
  ['Maya Chen', 'maya@example.com'],
  ['Noah Levi', 'noah@example.com'],
  ['Iris Tanaka', 'iris@example.com'],
  ['Leo Martin', 'leo@example.com'],
];

export function buildDemoRepo(root, home) {
  mkdirSync(root, { recursive: true });
  let t = Date.parse('2026-09-08T09:12:00Z');
  const git = (args, author = AUTHORS[0]) => {
    const when = new Date(t).toISOString();
    return execFileSync('git', args, {
      cwd: root,
      encoding: 'utf8',
      env: {
        ...process.env,
        HOME: home,
        GIT_CONFIG_NOSYSTEM: '1',
        GIT_AUTHOR_NAME: author[0],
        GIT_AUTHOR_EMAIL: author[1],
        GIT_COMMITTER_NAME: author[0],
        GIT_COMMITTER_EMAIL: author[1],
        GIT_AUTHOR_DATE: when,
        GIT_COMMITTER_DATE: when,
      },
    });
  };
  const write = (rel, body) => {
    const f = join(root, rel);
    mkdirSync(dirname(f), { recursive: true });
    writeFileSync(f, body);
  };
  const commit = (msg, files, author = AUTHORS[0]) => {
    for (const [rel, body] of Object.entries(files)) write(rel, body);
    git(['add', '-A'], author);
    git(['commit', '-q', '-m', msg], author);
    t += 1000 * 60 * (47 + ((msg.length * 13) % 300));
  };

  git(['init', '-q', '-b', 'main']);
  commit('chore: scaffold checkout service', {
    'README.md': '# acme-checkout\n\nCheckout API for the Acme storefront.\n',
    'package.json': '{\n  "name": "acme-checkout",\n  "version": "1.4.0",\n  "type": "module"\n}\n',
    'src/router.ts': "import { Router } from './http';\n\nexport const router = new Router();\n",
  });
  commit('feat(cart): add cart totals with tax rules', {
    'src/cart/totals.ts': 'export function total(items: { price: number; qty: number }[], tax = 0.2) {\n  const net = items.reduce((s, i) => s + i.price * i.qty, 0);\n  return Math.round(net * (1 + tax));\n}\n',
  }, AUTHORS[1]);
  commit('test(cart): cover rounding and empty carts', {
    'test/cart.test.ts': "import { total } from '../src/cart/totals';\n\ntest('empty', () => expect(total([])).toBe(0));\n",
  }, AUTHORS[1]);

  git(['checkout', '-q', '-b', 'feat/payments-retry']);
  commit('feat(payments): retry declined cards with backoff', {
    'src/payments/retry.ts': 'export async function withRetry<T>(fn: () => Promise<T>, tries = 3): Promise<T> {\n  for (let i = 0; ; i++) {\n    try { return await fn(); } catch (e) { if (i + 1 >= tries) throw e; await sleep(2 ** i * 250); }\n  }\n}\nconst sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));\n',
  }, AUTHORS[2]);
  commit('fix(payments): never retry hard declines', {
    'src/payments/retry.ts': 'export async function withRetry<T>(fn: () => Promise<T>, tries = 3): Promise<T> {\n  for (let i = 0; ; i++) {\n    try { return await fn(); } catch (e: any) { if (e?.hard || i + 1 >= tries) throw e; await sleep(2 ** i * 250); }\n  }\n}\nconst sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));\n',
  }, AUTHORS[2]);

  git(['checkout', '-q', 'main']);
  commit('docs: document the checkout flow', {
    'docs/flow.md': '# Checkout flow\n\n1. Cart totals\n2. Payment intent\n3. Confirmation email\n',
  }, AUTHORS[3]);
  git(['merge', '-q', '--no-ff', 'feat/payments-retry', '-m', "Merge branch 'feat/payments-retry'"], AUTHORS[0]);
  t += 1000 * 60 * 90;

  git(['checkout', '-q', '-b', 'feat/rate-limit']);
  commit('feat(api): token-bucket rate limiter middleware', {
    'src/middleware/rateLimit.ts': 'export function tokenBucket({ capacity, refillPerSec }: { capacity: number; refillPerSec: number }) {\n  const buckets = new Map<string, { tokens: number; at: number }>();\n  return (key: string) => {\n    const now = Date.now();\n    const b = buckets.get(key) ?? { tokens: capacity, at: now };\n    b.tokens = Math.min(capacity, b.tokens + ((now - b.at) / 1000) * refillPerSec);\n    b.at = now;\n    if (b.tokens < 1) return false;\n    b.tokens -= 1;\n    buckets.set(key, b);\n    return true;\n  };\n}\n',
  });
  commit('feat(api): return 429 with Retry-After', {
    'src/router.ts': "import { Router } from './http';\nimport { tokenBucket } from './middleware/rateLimit';\n\nexport const router = new Router();\nconst allow = tokenBucket({ capacity: 60, refillPerSec: 1 });\nrouter.use('/checkout', (req, res, next) => (allow(req.ip) ? next() : res.status(429).set('Retry-After', '1').end()));\n",
  });

  git(['checkout', '-q', 'main']);
  commit('perf(cart): memoize tax rules per region', {
    'src/cart/tax.ts': 'const cache = new Map<string, number>();\nexport const taxFor = (region: string) => cache.get(region) ?? 0.2;\n',
  }, AUTHORS[1]);
  commit('chore(release): 1.5.0', {
    'package.json': '{\n  "name": "acme-checkout",\n  "version": "1.5.0",\n  "type": "module"\n}\n',
    'CHANGELOG.md': '## 1.5.0\n\n- Card retries with backoff\n- Regional tax cache\n',
  }, AUTHORS[0]);
  git(['tag', 'v1.5.0']);

  git(['checkout', '-q', 'feat/rate-limit']);
  commit('test(api): burst + refill coverage for the limiter', {
    'test/rateLimit.test.ts': "import { tokenBucket } from '../src/middleware/rateLimit';\n\ntest('burst', () => {\n  const allow = tokenBucket({ capacity: 2, refillPerSec: 0 });\n  expect([allow('a'), allow('a'), allow('a')]).toEqual([true, true, false]);\n});\n",
  });

  // Dirty working tree → the graph's WIP row + staged/unstaged lists.
  write('src/middleware/rateLimit.ts', 'export function tokenBucket({ capacity, refillPerSec }: { capacity: number; refillPerSec: number }) {\n  const buckets = new Map<string, { tokens: number; at: number }>();\n  // Evict idle buckets so memory stays bounded under key churn.\n  setInterval(() => { const cut = Date.now() - 60_000; for (const [k, b] of buckets) if (b.at < cut) buckets.delete(k); }, 30_000).unref();\n  return (key: string) => {\n    const now = Date.now();\n    const b = buckets.get(key) ?? { tokens: capacity, at: now };\n    b.tokens = Math.min(capacity, b.tokens + ((now - b.at) / 1000) * refillPerSec);\n    b.at = now;\n    if (b.tokens < 1) return false;\n    b.tokens -= 1;\n    buckets.set(key, b);\n    return true;\n  };\n}\n');
  write('docs/rate-limits.md', '# Rate limits\n\nCheckout allows 60 requests per minute per client. Over the limit returns 429 with `Retry-After`.\n');
  git(['add', 'docs/rate-limits.md']);
  return root;
}
