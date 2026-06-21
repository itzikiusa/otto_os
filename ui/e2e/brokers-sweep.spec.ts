import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// LIVE mobile/responsive sweep for the Message Brokers (Kafka) page, driven
// against a real single-node Redpanda dev stack (container otto-redpanda):
//   Kafka bootstrap  127.0.0.1:19092  (plaintext)
//   Schema Registry  http://127.0.0.1:18081
//   Admin/metrics    http://127.0.0.1:19644/public_metrics
//
// Unlike brokers-mobile.spec.ts (which seeds a FAKE cluster and asserts LAYOUT
// only), this spec connects to the live broker, seeds a topic + messages, and
// exercises REAL data flows in the UI on every device project — phone + tablet,
// portrait + landscape: overview, topics list, topic detail PEEK, PRODUCE via
// the UI, consumer groups, schema registry. For each tab it asserts the page
// fits the viewport width (internal scrollers may scroll, the page may not) and
// that overflowing lists scroll vertically.
//
// The cluster + topic are seeded once via the REST surface (the exact endpoints
// from crates/otto-brokers/tests/kafka_e2e.rs). To avoid cross-project races on
// the shared Redpanda, the topic name is unique per worker via the
// TEST_PARALLEL_INDEX env that Playwright sets per worker.

const BOOTSTRAP = '127.0.0.1:19092';
const SR_URL = 'http://127.0.0.1:18081';
const METRICS_URL = 'http://127.0.0.1:19644/public_metrics';
// Partitions per seeded topic. Kept small (2) so the shared single-node dev
// Redpanda — which caps total partitions for "hardware constraints" — doesn't
// reject creation when many topics already exist. The UI assertion follows this
// constant rather than hard-coding a number.
const PARTITIONS = 2;

let workspaceId = '';
let clusterName = '';
let topicName = '';
let clusterId = '';

/** Wait for the live cluster to pass /test (Redpanda may take ~15s post-boot). */
async function createAndVerifyCluster(
  ctx: APIRequestContext,
  base: string,
  wsId: string,
  name: string,
): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/brokers/clusters`, {
    data: {
      name,
      bootstrap_servers: BOOTSTRAP,
      security_protocol: 'plaintext',
      schema_registry_url: SR_URL,
      metrics_url: METRICS_URL,
      environment: 'dev', // dev so PRODUCE/writes are allowed (prod is guarded)
      read_only: false,
    },
  });
  if (!r.ok()) throw new Error(`create cluster → ${r.status()} ${await r.text()}`);
  const cluster = (await r.json()) as { id: string };
  const id = cluster.id;

  // Retry /test for ~20s — the broker can be slow to accept the first metadata call.
  let lastErr = '';
  for (let i = 0; i < 20; i++) {
    const tr = await ctx.post(`${base}/api/v1/brokers/clusters/${id}/test`, { data: {} });
    if (tr.ok()) {
      const body = (await tr.json()) as { ok?: boolean; broker_count?: number; message?: string };
      if (body.ok && (body.broker_count ?? 0) >= 1) return id;
      lastErr = body.message ?? JSON.stringify(body);
    } else {
      lastErr = `${tr.status()} ${await tr.text()}`;
    }
    await new Promise((res) => setTimeout(res, 1000));
  }
  throw new Error(`live cluster never became healthy: ${lastErr}`);
}

/** Create a topic (PARTITIONS partitions) + produce several JSON messages so the
 *  topic list shows real stats and the message viewer overflows. */
async function seedTopic(
  ctx: APIRequestContext,
  base: string,
  clusterId: string,
  topic: string,
): Promise<string[]> {
  const ct = await ctx.post(`${base}/api/v1/brokers/clusters/${clusterId}/topics`, {
    data: { name: topic, partitions: PARTITIONS, replication_factor: 1 },
  });
  if (!ct.ok()) throw new Error(`create topic → ${ct.status()} ${await ct.text()}`);

  // Produce enough rows to make the message list overflow a phone viewport.
  const keys: string[] = [];
  for (let i = 0; i < 24; i++) {
    const key = `seed-${i}`;
    const pr = await ctx.post(
      `${base}/api/v1/brokers/clusters/${clusterId}/topics/${encodeURIComponent(topic)}/produce`,
      { data: { key, value: `{"n":${i},"label":"seed message number ${i}"}` } },
    );
    if (!pr.ok()) throw new Error(`produce ${i} → ${pr.status()} ${await pr.text()}`);
    keys.push(key);
  }
  return keys;
}

test.beforeAll(async ({}, testInfo) => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  // Unique names per worker so parallel projects don't fight over the shared broker.
  const tag = `${testInfo.parallelIndex}-${Date.now().toString(36)}`;
  clusterName = `e2e-redpanda-${tag}`;
  topicName = `e2e-orders-${tag}`;
  clusterId = await createAndVerifyCluster(ctx, base, workspaceId, clusterName);
  await seedTopic(ctx, base, clusterId, topicName);
  await ctx.dispose();
});

// Delete the seeded topic so this suite doesn't accumulate partitions on the
// shared single-node Redpanda across runs (which eventually trips its
// partition-count "hardware constraints" cap and breaks topic creation).
test.afterAll(async () => {
  try {
    const { ctx, base } = await apiCtx();
    if (clusterId && topicName) {
      await ctx.delete(
        `${base}/api/v1/brokers/clusters/${clusterId}/topics/${encodeURIComponent(topicName)}?confirm=true`,
      );
    }
    await ctx.dispose();
  } catch {
    // best-effort cleanup
  }
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// ── shared assertions ───────────────────────────────────────────────────────

/** The PAGE must not scroll horizontally (allow 1px rounding). Internal
 *  horizontal scrollers (tab strip, wide tables) are exempt — this measures only
 *  the document. */
async function assertPageFitsWidth(page: Page): Promise<void> {
  const r = await page.evaluate(() => {
    const de = document.documentElement;
    return { docScrollW: de.scrollWidth, docClientW: de.clientWidth, vw: window.innerWidth };
  });
  expect(r.docScrollW, 'document horizontal overflow').toBeLessThanOrEqual(r.docClientW + 1);
  expect(r.docClientW).toBeLessThanOrEqual(r.vw + 1);
}

/** No visible element may extend more than `slack` px past the viewport's right
 *  edge. Catches a single child that juts off-screen even if the document itself
 *  was clamped by overflow:hidden. Elements inside an x-scroll container are
 *  exempt (they're allowed to overflow their scroller). */
async function assertNothingJutsPast(page: Page, slack = 2): Promise<string[]> {
  return page.evaluate((slackPx) => {
    const vw = window.innerWidth;
    const offenders: string[] = [];
    const inScroller = (el: Element): boolean => {
      let p = el.parentElement;
      while (p) {
        const oc = getComputedStyle(p);
        if (
          (oc.overflowX === 'auto' || oc.overflowX === 'scroll') &&
          p.scrollWidth > p.clientWidth + 1
        )
          return true;
        p = p.parentElement;
      }
      return false;
    };
    const all = document.querySelectorAll('.brokers-page *');
    for (const el of all) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      if (r.right > vw + slackPx && !inScroller(el)) {
        const c = (el.className && typeof el.className === 'string' ? el.className : '').slice(0, 40);
        offenders.push(`${el.tagName.toLowerCase()}.${c} right=${Math.round(r.right)} vw=${vw}`);
      }
    }
    return offenders;
  }, slack);
}

/** Whether the page is rendered in the ≤640px phone layout (stacked, collapsible
 *  sections) vs the desktop two-column layout. */
async function isPhoneLayout(page: Page): Promise<boolean> {
  return (await page.evaluate(() => window.innerWidth)) <= 640;
}

/** Select the seeded cluster in the sidebar. On the phone layout the list
 *  auto-collapses after selection, so re-expand it first if needed. */
async function selectCluster(page: Page): Promise<void> {
  await page.goto('/#/brokers');
  await expect(page.locator('.brokers-page')).toBeVisible({ timeout: 30_000 });
  // The cluster list may be collapsed (phone) — open it via the section toggle.
  const listVisible = await page.locator('.cluster-list').isVisible().catch(() => false);
  if (!listVisible) await page.locator('.sec-toggle').first().click().catch(() => {});
  const row = page.locator('.cluster .cn', { hasText: clusterName }).first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();
  await expect(page.locator('.cluster-head .name', { hasText: clusterName })).toBeVisible({
    timeout: 15_000,
  });
}

/** On the phone layout the content section can be collapsed; ensure it's open so
 *  tabs/body are interactable. */
async function ensureContentOpen(page: Page): Promise<void> {
  const tabsVisible = await page.locator('.tabs').isVisible().catch(() => false);
  if (!tabsVisible) {
    const toggle = page.locator('.content-toggle').first();
    if (await toggle.isVisible().catch(() => false)) await toggle.click();
  }
  await expect(page.locator('.tabs')).toBeVisible({ timeout: 10_000 });
}

async function clickTab(page: Page, label: string): Promise<void> {
  await ensureContentOpen(page);
  await page.locator('.tabs button', { hasText: label }).first().click();
}

// ── 1. Overview shows REAL cluster/broker data ──────────────────────────────

test('overview: live cluster + broker render with real numbers', async ({ page }) => {
  await selectCluster(page);
  await ensureContentOpen(page);
  // Overview tab is the default after selecting.
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
  // Real cluster: at least one broker card with a count > 0, not the
  // "Connecting…" placeholder or an error toast.
  const cards = page.locator('.overview .cards .card');
  await expect(cards.first()).toBeVisible({ timeout: 20_000 });
  // "Brokers" card should report >=1.
  const brokersCard = page
    .locator('.overview .card', { has: page.locator('.k', { hasText: 'Brokers' }) })
    .first();
  await expect(brokersCard).toBeVisible();
  const brokerCount = await brokersCard.locator('.v').innerText();
  expect(Number(brokerCount.trim()), `broker count "${brokerCount}"`).toBeGreaterThanOrEqual(1);
  // A per-broker tile should render under the Brokers section.
  await expect(page.locator('.overview .broker').first()).toBeVisible({ timeout: 20_000 });

  await assertPageFitsWidth(page);
  expect(await assertNothingJutsPast(page), 'overview offenders').toEqual([]);
});

// ── 2. Topics list shows the seeded topic + its stats ───────────────────────

test('topics: seeded topic is listed with partition/count stats', async ({ page }) => {
  await selectCluster(page);
  await clickTab(page, 'Topics');
  await expect(page.locator('.topics')).toBeVisible({ timeout: 15_000 });

  // The seeded topic row appears (search to be robust against pagination).
  await page.locator('.topics .search').fill(topicName);
  const cell = page.locator('table.grid td.tname', { hasText: topicName }).first();
  await expect(cell).toBeVisible({ timeout: 15_000 });

  // Its partition count is visible (we created PARTITIONS). Columns: Topic,
  // Partitions, RF, Count, Msg/s, Size — td.num[0] is partitions, td.num[2] count.
  const tr = page.locator('table.grid tbody tr', {
    has: page.locator('td.tname', { hasText: topicName }),
  });
  await expect(tr.locator('td.num').first()).toHaveText(String(PARTITIONS), { timeout: 10_000 });
  // Count column eventually shows a real number (>=24) — poll the cell text.
  await expect
    .poll(
      async () => {
        const txt = await tr.locator('td.num').nth(2).innerText();
        const n = Number(txt.replace(/[^\d]/g, ''));
        return Number.isFinite(n) ? n : 0;
      },
      { timeout: 20_000, message: 'topic message count' },
    )
    .toBeGreaterThanOrEqual(24);

  await assertPageFitsWidth(page);
  expect(await assertNothingJutsPast(page), 'topics offenders').toEqual([]);
});

// ── 3. Topic detail: PEEK real messages ─────────────────────────────────────

test('topic detail: peek shows real message rows', async ({ page }) => {
  await selectCluster(page);
  await clickTab(page, 'Topics');
  await page.locator('.topics .search').fill(topicName);
  await page.locator('table.grid td.tname', { hasText: topicName }).first().click();

  // TopicDetail mounts with the Messages subtab.
  await expect(page.locator('.td')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.consume-bar')).toBeVisible();

  // Peek from the beginning so the seeded messages come back.
  await page.locator('.consume-bar select').first().selectOption('beginning');
  await page.locator('.consume-bar button', { hasText: 'Peek' }).click();

  // >=1 real message row in the viewer.
  const rows = page.locator('.msg-list tbody tr');
  await expect(rows.first()).toBeVisible({ timeout: 20_000 });
  expect(await rows.count(), 'peeked message rows').toBeGreaterThanOrEqual(1);

  // Clicking a row populates the detail pane (key/value visible).
  await rows.first().click();
  await expect(page.locator('.msg-detail .payload').first()).toBeVisible({ timeout: 10_000 });

  await assertPageFitsWidth(page);
  expect(await assertNothingJutsPast(page), 'topic-detail offenders').toEqual([]);

  // The message list scrolls vertically when rows overflow its container.
  const scrollable = await page.evaluate(() => {
    const el = document.querySelector('.msg-list') as HTMLElement | null;
    if (!el) return null;
    return { scrollH: el.scrollHeight, clientH: el.clientHeight };
  });
  expect(scrollable, '.msg-list present').not.toBeNull();
  // With 24 seeded rows in a phone-height pane the list overflows; assert it can
  // scroll (scrollHeight > clientHeight) OR all rows fit (small content is OK).
  if (scrollable!.scrollH > scrollable!.clientH + 1) {
    const moved = await page.evaluate(() => {
      const el = document.querySelector('.msg-list') as HTMLElement;
      el.scrollTop = el.scrollHeight;
      return el.scrollTop > 0;
    });
    expect(moved, '.msg-list scrolled vertically').toBe(true);
  }
});

// ── 4. PRODUCE through the UI, then re-peek to see it ────────────────────────

test('produce via UI: message round-trips into the topic', async ({ page }, testInfo) => {
  await selectCluster(page);
  await clickTab(page, 'Topics');
  await page.locator('.topics .search').fill(topicName);
  await page.locator('table.grid td.tname', { hasText: topicName }).first().click();
  await expect(page.locator('.td')).toBeVisible({ timeout: 15_000 });

  // Unique marker per project so re-peeks don't false-match the seed data.
  const marker = `ui-produced-${testInfo.project.name}-${Date.now().toString(36)}`;

  // Switch to the Produce subtab and send a message.
  await page.locator('.subtabs button', { hasText: 'Produce' }).click();
  await expect(page.locator('.produce')).toBeVisible({ timeout: 10_000 });
  await page.locator('.produce .field input').first().fill(marker); // key
  await page.locator('.produce textarea').fill(`{"marker":"${marker}"}`);
  await page.locator('.produce button', { hasText: 'Produce message' }).click();

  // Success toast confirms the broker accepted it.
  await expect(page.getByText(/Produced to partition/i).first()).toBeVisible({ timeout: 20_000 });

  // Re-peek from the beginning and assert the produced key appears.
  await page.locator('.subtabs button', { hasText: 'Messages' }).click();
  await page.locator('.consume-bar select').first().selectOption('beginning');
  // Raise the limit so the new message (latest offset) is included.
  await page.locator('.consume-bar input[type="number"]').first().fill('500');
  await page.locator('.consume-bar button', { hasText: 'Peek' }).click();

  await expect(page.locator('.msg-list tbody tr').first()).toBeVisible({ timeout: 20_000 });
  await expect
    .poll(
      async () => page.locator('.msg-list td.key', { hasText: marker }).count(),
      { timeout: 20_000, message: 'produced message visible on re-peek' },
    )
    .toBeGreaterThanOrEqual(1);

  await assertPageFitsWidth(page);
  expect(await assertNothingJutsPast(page), 'produce offenders').toEqual([]);
});

// ── 5. Consumer groups tab renders (real or empty-but-correct) ──────────────

test('consumer groups tab renders without error', async ({ page }) => {
  await selectCluster(page);
  await clickTab(page, 'Consumer Groups');
  await expect(page.locator('.groups')).toBeVisible({ timeout: 15_000 });
  // Either a group list, the empty state, or the ACL-denied banner — NOT an
  // error toast and NOT a perpetual loading spinner.
  await expect(page.locator('.groups .list')).toBeVisible({ timeout: 15_000 });
  await expect
    .poll(async () => (await page.locator('.groups .muted', { hasText: 'Loading' }).count()), {
      timeout: 15_000,
      message: 'groups finished loading',
    })
    .toBe(0);
  await expect(page.getByText('Failed to load groups')).toHaveCount(0);

  await assertPageFitsWidth(page);
  expect(await assertNothingJutsPast(page), 'groups offenders').toEqual([]);
});

// ── 6. Schema registry tab renders (empty-but-correct against fresh Redpanda) ─

test('schema registry tab renders without error', async ({ page }) => {
  await selectCluster(page);
  await clickTab(page, 'Schema Registry');
  await expect(page.locator('.schema')).toBeVisible({ timeout: 15_000 });
  // Fresh Redpanda registry → no subjects, but the panel must render its
  // empty/list state (not a thrown error / not stuck loading).
  await expect
    .poll(async () => (await page.locator('.schema .muted', { hasText: 'Loading' }).count()), {
      timeout: 15_000,
      message: 'schema finished loading',
    })
    .toBe(0);
  // Either the subject list (with "No subjects registered.") or an error empty
  // state; assert the list container exists OR the empty pane renders.
  const hasList = await page.locator('.schema .list').count();
  const hasEmpty = await page.locator('.schema .empty').count();
  expect(hasList + hasEmpty, 'schema renders list or empty pane').toBeGreaterThan(0);

  await assertPageFitsWidth(page);
  expect(await assertNothingJutsPast(page), 'schema offenders').toEqual([]);
});

// ── 7. Tabstrip / wide tables scroll INTERNALLY, page never breaks ──────────

test('wide content scrolls internally; page never overflows across all tabs', async ({ page }) => {
  await selectCluster(page);
  const tabs = ['Overview', 'Topics', 'Consumer Groups', 'Schema Registry', 'Replay', 'Lag Alerts'];
  for (const t of tabs) {
    await clickTab(page, t);
    await expect(page.locator('.tab-body')).toBeVisible({ timeout: 15_000 });
    // Give the panel a beat to fetch/render.
    await page.waitForTimeout(300);
    await assertPageFitsWidth(page);
    const offenders = await assertNothingJutsPast(page);
    expect(offenders, `offenders on ${t}`).toEqual([]);
  }

  // The tab strip itself is allowed to scroll horizontally on phones; verify it
  // is a bounded x-scroller rather than pushing the page wide.
  if (await isPhoneLayout(page)) {
    const tabScroll = await page.evaluate(() => {
      const el = document.querySelector('.tabs') as HTMLElement | null;
      if (!el) return null;
      const c = getComputedStyle(el);
      return { overflowX: c.overflowX, scrollW: el.scrollWidth, clientW: el.clientWidth };
    });
    expect(tabScroll, '.tabs present').not.toBeNull();
    expect(['auto', 'scroll']).toContain(tabScroll!.overflowX);
  }
});

// ── 8. Topics list scrolls vertically (the grid-wrap is a bounded scroller) ──

test('topics list is a bounded vertical scroller', async ({ page }) => {
  await selectCluster(page);
  await clickTab(page, 'Topics');
  await expect(page.locator('.topics')).toBeVisible({ timeout: 15_000 });
  await page.locator('.topics .search').fill(''); // show all topics
  await expect(page.locator('table.grid tbody tr').first()).toBeVisible({ timeout: 15_000 });

  const wrap = await page.evaluate(() => {
    const el = document.querySelector('.topics .grid-wrap') as HTMLElement | null;
    if (!el) return null;
    const c = getComputedStyle(el);
    return { overflowY: c.overflowY, scrollH: el.scrollHeight, clientH: el.clientHeight };
  });
  expect(wrap, '.grid-wrap present').not.toBeNull();
  expect(['auto', 'scroll']).toContain(wrap!.overflowY);
});
