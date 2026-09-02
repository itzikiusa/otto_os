import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// AWS console — REAL end-to-end against LocalStack (desktop-browser only).
//
// The daemon shells out to the `aws` CLI v2, so this spec needs three things
// on the machine: Docker (LocalStack container), the `aws` binary on PATH, and
// a daemon that has the `/aws/*` routes. Any of them missing ⇒ the whole file
// skips with the reason (never fails).
//
// Flow: start the LocalStack image (or reuse a running
// `otto-e2e-localstack-<slot>`), seed S3 / SQS / EC2 with the CLI, create an
// Otto `access_keys` account whose `endpoint_url` points at the container,
// then drive the UI: account card (identity + permission chips), S3 browse →
// preview → streamed download, SQS peek → send → purge (typed), EC2 stop
// (typed). Everything that LocalStack settles asynchronously is polled.
//
// Run:
//   cd ui && OTTO_E2E_BIN=<repo>/target/debug/ottod \
//     npx playwright test desktop-aws-localstack --project=desktop-browser
// ─────────────────────────────────────────────────────────────────────────────

test.describe.configure({ mode: 'serial', timeout: 120_000 });

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const CONTAINER = `otto-e2e-localstack-${SLOT}`;
// `localstack/localstack:latest` (2026.x) refuses to start without a
// LOCALSTACK_AUTH_TOKEN ("License activation failed", exit 55) — 4.14.0 is the
// last Community release that runs token-free. Override the image (and pass a
// token through) when you have a licence.
const IMAGE = process.env.OTTO_E2E_LOCALSTACK_IMAGE ?? 'localstack/localstack:4.14.0';
const SERVICES = ['s3', 'sqs', 'sts', 'iam', 'ec2'];
const REGION = 'us-east-1';
const BUCKET = 'otto-e2e-bucket';
const QUEUE = 'otto-e2e-queue';
const FIFO_QUEUE = 'otto-e2e-queue.fifo';
const INSTANCE_NAME = 'otto-e2e';
const ACCOUNT_NAME = `e2e-localstack-${Date.now().toString(36)}`;

const JSON_BODY = JSON.stringify(
  { service: 'otto-e2e', replicas: 3, tags: ['alpha', 'beta'], owner: { team: 'platform' } },
  null,
  2,
);
const CSV_BODY = 'id,name,qty\n1,widget,4\n2,gadget,9\n3,doohickey,1\n';
// 4 KiB of noise with a guaranteed NUL so the daemon classifies it binary.
const BIN_BODY = Buffer.concat([Buffer.from([0, 1, 2, 3]), randomBytes(4092)]);

interface Env {
  endpoint: string;
  port: number;
  startedContainer: boolean;
  accountId: string;
  instanceId: string;
  tmp: string;
}
const env: Env = { endpoint: '', port: 0, startedContainer: false, accountId: '', instanceId: '', tmp: '' };

function sh(cmd: string, args: string[], extraEnv: Record<string, string> = {}): string {
  return execFileSync(cmd, args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, ...extraEnv },
    timeout: 60_000,
  }).trim();
}

/** `aws …` against LocalStack with the well-known test credentials. */
function awsCli(args: string[]): string {
  return sh('aws', [...args, '--output', 'json'], {
    AWS_ACCESS_KEY_ID: 'test',
    AWS_SECRET_ACCESS_KEY: 'test',
    AWS_REGION: REGION,
    AWS_DEFAULT_REGION: REGION,
    AWS_ENDPOINT_URL: env.endpoint,
    AWS_EC2_METADATA_DISABLED: 'true',
    AWS_PAGER: '',
  });
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      const port = typeof addr === 'object' && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

/** Host port of an already-running `otto-e2e-localstack-<slot>`, if any. */
function runningContainerPort(): number | null {
  try {
    const ports = sh('docker', ['ps', '--filter', `name=^/${CONTAINER}$`, '--format', '{{.Ports}}']);
    const m = /:(\d+)->4566\/tcp/.exec(ports);
    return m ? Number(m[1]) : null;
  } catch {
    return null;
  }
}

async function waitForHealth(endpoint: string, budgetMs: number): Promise<void> {
  const deadline = Date.now() + budgetMs;
  let last = '';
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`${endpoint}/_localstack/health`);
      if (r.ok) {
        const body = (await r.json()) as { services?: Record<string, string> };
        const svc = body.services ?? {};
        // LocalStack lazy-loads: "available" = ready to start on first call,
        // "running" = already up. Both mean the API answers.
        const ready = SERVICES.every((s) => svc[s] === 'running' || svc[s] === 'available');
        if (ready) return;
        last = JSON.stringify(svc);
      } else {
        last = `HTTP ${r.status}`;
      }
    } catch (e) {
      last = String(e);
    }
    await new Promise((r) => setTimeout(r, 1_000));
  }
  throw new Error(`LocalStack at ${endpoint} not healthy after ${budgetMs} ms (last: ${last})`);
}

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  return errors;
}

function realErrors(errors: string[]): string[] {
  // Failed-resource noise (favicon, aborted fetches) is not a product bug.
  return errors.filter((e) => !/Failed to load resource|net::ERR_|404/i.test(e));
}

/** Wait for an element's text to satisfy `re`, clicking `refresh` between polls. */
async function pollWithRefresh(page: Page, refreshName: string, read: () => Promise<string>, re: RegExp, timeout = 40_000): Promise<void> {
  await expect
    .poll(
      async () => {
        const text = await read();
        if (re.test(text)) return text;
        const btn = page.getByRole('button', { name: refreshName });
        if (await btn.isEnabled().catch(() => false)) await btn.click().catch(() => {});
        return text;
      },
      { timeout, intervals: [1_000, 1_500, 2_000] },
    )
    .toMatch(re);
}

test.beforeAll(async () => {
  test.setTimeout(300_000); // image pull on a cold machine

  // ── prerequisites → skip (never fail) ──
  let reason = '';
  try {
    sh('docker', ['info']);
  } catch {
    reason = '`docker info` failed — Docker is not running';
  }
  if (!reason) {
    try {
      sh('aws', ['--version']);
    } catch {
      reason = '`aws` CLI not on PATH (brew install awscli)';
    }
  }
  if (!reason) {
    const { ctx, base } = await apiCtx();
    try {
      const r = await ctx.get(`${base}/api/v1/aws/status`);
      if (r.status() === 404) reason = 'GET /api/v1/aws/status → 404 (AWS routes not in this daemon — set OTTO_E2E_BIN to the worktree build)';
      else if (!r.ok()) reason = `GET /api/v1/aws/status → ${r.status()}`;
      else if ((await r.json()).installed !== true) reason = 'the test daemon cannot locate an `aws` binary';
    } catch (e) {
      reason = `status probe failed: ${String(e)}`;
    } finally {
      await ctx.dispose();
    }
  }
  if (reason) console.log(`[desktop-aws-localstack] skipping: ${reason}`);
  test.skip(!!reason, reason);

  // ── LocalStack container (reuse or start) ──
  const existing = runningContainerPort();
  if (existing) {
    env.port = existing;
    env.startedContainer = false;
    console.log(`[desktop-aws-localstack] reusing ${CONTAINER} on :${existing}`);
  } else {
    env.port = await freePort();
    sh('docker', [
      'run', '-d', '--rm',
      '--name', CONTAINER,
      '-p', `127.0.0.1:${env.port}:4566`,
      '-e', `SERVICES=${SERVICES.join(',')}`,
      // Path-style queue URLs on a host:port the CLI can actually reach, so
      // the URLs LocalStack hands back stay valid outside the container.
      '-e', 'SQS_ENDPOINT_STRATEGY=path',
      '-e', `LOCALSTACK_HOST=127.0.0.1:${env.port}`,
      ...(process.env.LOCALSTACK_AUTH_TOKEN ? ['-e', `LOCALSTACK_AUTH_TOKEN=${process.env.LOCALSTACK_AUTH_TOKEN}`] : []),
      IMAGE,
    ]);
    env.startedContainer = true;
    console.log(`[desktop-aws-localstack] started ${CONTAINER} on :${env.port}`);
  }
  env.endpoint = `http://127.0.0.1:${env.port}`;
  await waitForHealth(env.endpoint, 60_000);

  // ── seed with the CLI ──
  env.tmp = mkdtempSync(join(tmpdir(), 'otto-e2e-localstack-'));
  const jsonPath = join(env.tmp, 'config.json');
  const csvPath = join(env.tmp, 'rows.csv');
  const binPath = join(env.tmp, 'blob.bin');
  writeFileSync(jsonPath, JSON_BODY);
  writeFileSync(csvPath, CSV_BODY);
  writeFileSync(binPath, BIN_BODY);

  const buckets = JSON.parse(awsCli(['s3api', 'list-buckets'])) as { Buckets?: { Name: string }[] };
  if (!(buckets.Buckets ?? []).some((b) => b.Name === BUCKET)) awsCli(['s3api', 'create-bucket', '--bucket', BUCKET]);
  awsCli(['s3', 'cp', jsonPath, `s3://${BUCKET}/data/config.json`, '--content-type', 'application/json']);
  awsCli(['s3', 'cp', csvPath, `s3://${BUCKET}/data/rows.csv`, '--content-type', 'text/csv']);
  awsCli(['s3', 'cp', binPath, `s3://${BUCKET}/data/blob.bin`, '--content-type', 'application/octet-stream']);
  awsCli(['s3', 'cp', csvPath, `s3://${BUCKET}/readme.txt`, '--content-type', 'text/plain']);

  const q = JSON.parse(awsCli(['sqs', 'create-queue', '--queue-name', QUEUE])) as { QueueUrl: string };
  awsCli(['sqs', 'purge-queue', '--queue-url', q.QueueUrl]);
  for (let i = 1; i <= 3; i++) {
    awsCli(['sqs', 'send-message', '--queue-url', q.QueueUrl, '--message-body', JSON.stringify({ n: i, kind: 'seed' })]);
  }
  awsCli(['sqs', 'create-queue', '--queue-name', FIFO_QUEUE, '--attributes', 'FifoQueue=true']);

  const run = JSON.parse(
    awsCli([
      'ec2', 'run-instances',
      '--image-id', 'ami-12345678',
      '--instance-type', 't3.micro',
      '--tag-specifications', `ResourceType=instance,Tags=[{Key=Name,Value=${INSTANCE_NAME}}]`,
    ]),
  ) as { Instances: { InstanceId: string }[] };
  env.instanceId = run.Instances[0].InstanceId;

  // ── the Otto account, pointed at the container ──
  const { ctx, base } = await apiCtx();
  try {
    const r = await ctx.post(`${base}/api/v1/aws/accounts`, {
      data: {
        name: ACCOUNT_NAME,
        auth_mode: 'access_keys',
        region: REGION,
        access_key_id: 'test',
        secret_access_key: 'test',
        endpoint_url: env.endpoint,
        environment: 'dev',
        color: '#22c55e',
      },
    });
    expect(r.ok(), `POST /aws/accounts → ${r.status()} ${await r.text()}`).toBeTruthy();
    const acct = (await r.json()) as { id: string; endpoint_url?: string; identity?: { account: string } };
    env.accountId = acct.id;
    expect(acct.endpoint_url).toBe(env.endpoint);
  } finally {
    await ctx.dispose();
  }
});

test.afterAll(async () => {
  if (env.accountId) {
    const { ctx, base } = await apiCtx();
    await ctx.delete(`${base}/api/v1/aws/accounts/${env.accountId}`).catch(() => {});
    await ctx.dispose();
  }
  if (env.startedContainer) {
    try {
      sh('docker', ['rm', '-f', CONTAINER]);
    } catch {
      /* already gone */
    }
  }
  if (env.tmp) rmSync(env.tmp, { recursive: true, force: true });
});

test('validation: a plain-http endpoint on a non-loopback host is refused', async () => {
  const { ctx, base } = await apiCtx();
  try {
    const bad = await ctx.post(`${base}/api/v1/aws/accounts`, {
      data: { name: 'x', auth_mode: 'access_keys', access_key_id: 'test', secret_access_key: 'test', endpoint_url: 'http://minio.internal:9000' },
    });
    expect(bad.status()).toBe(400);
    expect(await bad.text()).toMatch(/https/);
    const junk = await ctx.post(`${base}/api/v1/aws/accounts`, {
      data: { name: 'x', auth_mode: 'access_keys', access_key_id: 'test', secret_access_key: 'test', endpoint_url: 'localhost:4566' },
    });
    expect(junk.status()).toBe(400);
    // PATCH with "" clears; PATCH with the endpoint again restores (round trip).
    const cleared = await ctx.patch(`${base}/api/v1/aws/accounts/${env.accountId}`, { data: { endpoint_url: '' } });
    expect(cleared.ok()).toBeTruthy();
    expect(((await cleared.json()) as { endpoint_url?: string }).endpoint_url).toBeUndefined();
    const restored = await ctx.patch(`${base}/api/v1/aws/accounts/${env.accountId}`, { data: { endpoint_url: `${env.endpoint}/` } });
    expect(restored.ok()).toBeTruthy();
    expect(((await restored.json()) as { endpoint_url?: string }).endpoint_url).toBe(env.endpoint);
  } finally {
    await ctx.dispose();
  }
});

test('account card shows the LocalStack identity, the endpoint, and green S3/SQS/EC2 chips', async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto('/#/aws');
  const card = page.getByTestId('aws-account-card').filter({ hasText: ACCOUNT_NAME });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card.getByTestId('aws-account-endpoint')).toHaveText(env.endpoint);
  // `sts get-caller-identity` against LocalStack → the fixed dev account.
  await expect(card).toContainText('000000000000', { timeout: 20_000 });
  const chip = (label: string) => card.locator('a.chip', { hasText: new RegExp(`^\\s*${label}\\s*$`) });
  for (const s of ['S3', 'SQS', 'EC2']) await expect(chip(s)).toHaveClass(/\ballowed\b/, { timeout: 30_000 });
  // Athena / EKS are not in SERVICES — LocalStack answers with a non-IAM
  // error, so they must NOT be green (denied or unknown are both fine).
  for (const s of ['Athena', 'EKS']) await expect(chip(s)).not.toHaveClass(/\ballowed\b/);
  expect(realErrors(errors), `console errors: ${errors.join('\n')}`).toEqual([]);
});

test('S3: bucket → folder first → JSON preview → download matches the seeded size', async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto(`/#/aws/${env.accountId}/s3`);
  await expect(page.getByRole('heading', { name: 'S3', level: 2 })).toBeVisible({ timeout: 15_000 });
  const bucketRow = page.locator('tr.trow', { hasText: BUCKET });
  await expect(bucketRow).toBeVisible({ timeout: 20_000 });
  await bucketRow.click();

  // Folder rows sort first: `data/` before `readme.txt`.
  const rows = page.locator('table.tbl tbody tr.trow');
  await expect(rows.first()).toContainText('data/', { timeout: 20_000 });
  await expect(rows).toHaveCount(2);
  await rows.first().click();

  const jsonRow = page.locator('tr.trow', { hasText: 'config.json' });
  await expect(jsonRow).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('tr.trow', { hasText: 'rows.csv' })).toBeVisible();
  await expect(page.locator('tr.trow', { hasText: 'blob.bin' })).toBeVisible();
  await jsonRow.click();

  // Pretty JSON: the JsonTree renders keys + values of the parsed body.
  const drawer = page.getByRole('complementary', { name: 'Object preview' });
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText('replicas', { timeout: 20_000 });
  await expect(drawer).toContainText('platform');
  await expect(drawer.locator('pre.text')).toHaveCount(0);

  // Streamed download of the JSON (via the row button) — exact byte size.
  const [dlJson] = await Promise.all([
    page.waitForEvent('download', { timeout: 30_000 }),
    jsonRow.getByRole('button', { name: 'Download config.json' }).click(),
  ]);
  expect(dlJson.suggestedFilename()).toBe('config.json');
  expect(statSync((await dlJson.path()) as string).size).toBe(Buffer.byteLength(JSON_BODY));

  // And the binary object: preview says binary, download is byte-exact.
  const binRow = page.locator('tr.trow', { hasText: 'blob.bin' });
  await binRow.click();
  await expect(drawer).toContainText(/Binary content/, { timeout: 20_000 });
  const [dlBin] = await Promise.all([
    page.waitForEvent('download', { timeout: 30_000 }),
    drawer.getByRole('button', { name: /Download/ }).click(),
  ]);
  expect(statSync((await dlBin.path()) as string).size).toBe(BIN_BODY.length);
  expect(realErrors(errors), `console errors: ${errors.join('\n')}`).toEqual([]);
});

test('SQS: count 3 → peek 3 bodies → send → 4 → purge (typed) → 0', async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto(`/#/aws/${env.accountId}/sqs`);
  await expect(page.getByRole('heading', { name: 'SQS', level: 2 })).toBeVisible({ timeout: 15_000 });
  const row = page.locator('tr.trow').filter({ has: page.locator('.qn', { hasText: new RegExp(`^${QUEUE}$`) }) });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('tr.trow', { hasText: FIFO_QUEUE }).locator('.tag', { hasText: 'FIFO' })).toBeVisible();
  await expect(row.locator('td.num').first()).toHaveText('3', { timeout: 20_000 });
  await row.click();

  const counts = page.locator('.dhead .counts');
  await expect(counts).toContainText('3 avail', { timeout: 20_000 });

  await page.getByRole('button', { name: 'Peek', exact: true }).click();
  const msgs = page.locator('li.msg');
  await expect(msgs).toHaveCount(3, { timeout: 20_000 });
  for (let i = 0; i < 3; i++) await expect(msgs.nth(i).locator('pre.msg-preview')).toContainText('"kind"');

  await page.getByRole('tab', { name: 'Send' }).click();
  await page.locator('.form textarea').fill(JSON.stringify({ n: 4, kind: 'ui' }));
  await page.getByRole('button', { name: 'Send message' }).click();
  await expect(page.getByText('Message sent')).toBeVisible({ timeout: 15_000 });
  await pollWithRefresh(page, 'Refresh', () => counts.innerText(), /^4 avail/);

  // Purge lives in the ⋯ menu and needs the queue name typed.
  await page.getByRole('button', { name: 'Queue actions' }).click();
  await page.getByRole('menuitem', { name: /Purge queue/ }).click();
  const dlg = page.getByRole('dialog', { name: 'Purge queue' });
  await expect(dlg).toBeVisible();
  await dlg.locator('input.cf-input').fill(QUEUE);
  await dlg.getByRole('button', { name: 'Purge', exact: true }).click();
  await expect(page.getByText('Purge started')).toBeVisible({ timeout: 15_000 });
  await pollWithRefresh(page, 'Refresh', () => counts.innerText(), /^0 avail/);
  expect(realErrors(errors), `console errors: ${errors.join('\n')}`).toEqual([]);
});

test('EC2: the seeded instance is running → Stop with typed id → stopped', async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto(`/#/aws/${env.accountId}/ec2`);
  await expect(page.getByRole('heading', { name: 'EC2', level: 2 })).toBeVisible({ timeout: 15_000 });
  const row = page.locator('tr.trow', { hasText: env.instanceId });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(row).toContainText(INSTANCE_NAME);
  await expect(row.locator('.pill')).toHaveText('running');

  await row.getByRole('button', { name: `Actions for ${env.instanceId}` }).click();
  await page.getByRole('menuitem', { name: 'Stop', exact: true }).click();
  const dlg = page.getByRole('dialog', { name: 'Stop instance' });
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText(env.instanceId);
  await dlg.locator('input.cf-input').fill(env.instanceId);
  await dlg.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(page.getByText('stop sent')).toBeVisible({ timeout: 15_000 });
  await pollWithRefresh(page, 'Refresh', () => row.locator('.pill').innerText(), /^stopp(ed|ing)$/);
  expect(realErrors(errors), `console errors: ${errors.join('\n')}`).toEqual([]);
});
