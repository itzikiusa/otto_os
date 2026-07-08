import { test, expect, type Page, type BrowserContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ── E2E: manual title + description edit on a Jira product story ────────────────
//
// The Product story Overview lets you edit the Jira **summary** (title) and
// **description** in place — not just status / assignee / custom fields. This spec
// drives both inline editors end-to-end and asserts the round-trip: the write
// endpoints (`PUT …/fields` for the summary, `PUT …/description` for the body)
// receive the new content, the UI reflects it optimistically, and a reload
// re-renders the persisted values.
//
// The throwaway daemon can't reach a real Jira, so every `/issue/*` call — and the
// story-detail GET that flags the story as Jira-backed — is served from a small
// in-memory fixture via context.route. The PUT handlers MUTATE that fixture, so the
// detail GET after a reload returns exactly what the editors sent: that mutation is
// the durable proof the write happened with the right payload.
//
// context.route (not page.route) is required: the UI's fetch to the 127.0.0.1
// daemon is cross-origin, which page.route does not intercept. serviceWorkers:
// 'block' is required too — the UI registers a fetch-proxying service worker that
// would otherwise bypass interception. Desktop viewport so the inline editors render
// in their stable side-by-side layout.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000, serviceWorkers: 'block' });

let workspaceId = '';
let storyId = '';
let createdBy = '';
// Product stories are GLOBAL across workspaces and the suite runs in parallel
// against ONE shared daemon, so we always select our story BY a unique title.
const ROW_TITLE = `E2E Edit ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

const ACCOUNT_ID = 'acct-e2e';
const ISSUE_KEY = 'GS-99001';
const INITIAL_TITLE = `Jira Title ${Math.random().toString(36).slice(2, 8)}`;
const INITIAL_DESC = 'Original description line one.\n\nOriginal description line two.';
const NEW_TITLE = `Edited Title ${Math.random().toString(36).slice(2, 8)}`;
const NEW_DESC = `Rewritten description ${Math.random().toString(36).slice(2, 8)}.\n\nWith a second paragraph.`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  // A real draft gives us a valid story id + a row in the list. Its detail GET is
  // then stubbed to a Jira-backed shape (below); the list row keeps ROW_TITLE.
  const draft = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, {
    data: { title: ROW_TITLE },
  });
  if (!draft.ok()) throw new Error(`create draft → ${draft.status()} ${await draft.text()}`);
  const detail = await draft.json();
  storyId = detail.story.id;
  createdBy = detail.story.created_by;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// A mutable Jira fixture shared by all route handlers within a test run. The PUT
// handlers mutate `summary`/`description_md`; the detail + `/full` GETs read them.
interface Fixture {
  summary: string;
  description_md: string;
  fieldsPuts: Array<Record<string, unknown>>;
  descPuts: string[];
}

function issueFull(fx: Fixture): Record<string, unknown> {
  return {
    key: ISSUE_KEY,
    id: '990010',
    summary: fx.summary,
    status: 'To Do',
    issue_type: 'Story',
    url: `https://example.atlassian.net/browse/${ISSUE_KEY}`,
    description_md: fx.description_md,
    assignee: null,
    reporter: { account_id: 'rep-1', display_name: 'Reporter One' },
    priority: 'Major',
    labels: ['platform'],
    fields: [],
    comments: [],
    history: [],
    attachments: [],
    links: [],
    estimate: null,
  };
}

function storyDetail(fx: Fixture): Record<string, unknown> {
  const now = new Date(0).toISOString();
  return {
    story: {
      id: storyId,
      workspace_id: workspaceId,
      source_kind: 'jira',
      account_id: ACCOUNT_ID,
      source_key: ISSUE_KEY,
      title: fx.summary,
      url: `https://example.atlassian.net/browse/${ISSUE_KEY}`,
      issue_type: 'Story',
      stage: 'Backlog',
      cwd: null,
      watch_enabled: false,
      watch_cadence_min: 60,
      watch_cursor: null,
      confluence_tests_page_id: null,
      confluence_tests_url: null,
      tags: '',
      created_by: createdBy,
      created_at: now,
      updated_at: now,
    },
    source: {
      id: 'ver-1',
      story_id: storyId,
      version_no: 1,
      kind: 'source',
      title: fx.summary,
      body_md: fx.description_md,
      raw_json: null,
      change_notes: null,
      created_by: createdBy,
      created_at: now,
    },
    counts: { versions: 1, analyses: 0, open_questions: 0, notes: 0, testcases: 0 },
    swarm_link: null,
  };
}

function json(route: import('@playwright/test').Route, body: unknown, status = 200): Promise<void> {
  return route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
}

async function installStubs(context: BrowserContext, fx: Fixture): Promise<void> {
  // Story-detail GET → Jira-backed shape (exact id; sub-routes like /versions have
  // extra path and are NOT matched). Regex, not glob: globs mis-handle path tails.
  await context.route(new RegExp(`/api/v1/product/stories/${storyId}(\\?|$)`), (route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    return json(route, storyDetail(fx));
  });
  // PUT …/fields → capture, apply a summary edit, echo the updated issue.
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/fields(\?|$)/, (route) => {
    if (route.request().method() !== 'PUT') return route.fallback();
    const body = route.request().postDataJSON() as { fields?: Record<string, unknown> };
    fx.fieldsPuts.push(body?.fields ?? {});
    if (typeof body?.fields?.summary === 'string') fx.summary = body.fields.summary as string;
    return json(route, issueFull(fx));
  });
  // PUT …/description → capture the markdown, apply it, echo the updated issue.
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/description(\?|$)/, (route) => {
    if (route.request().method() !== 'PUT') return route.fallback();
    const body = route.request().postDataJSON() as { body_md?: string };
    fx.descPuts.push(body?.body_md ?? '');
    fx.description_md = body?.body_md ?? '';
    return json(route, issueFull(fx));
  });
  // Read-only Jira endpoints the Overview loads eagerly / lazily.
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/full(\?|$)/, (route) =>
    json(route, issueFull(fx)),
  );
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/editmeta(\?|$)/, (route) => json(route, []));
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/transitions(\?|$)/, (route) =>
    json(route, []),
  );
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/assignable(\?|$)/, (route) =>
    json(route, []),
  );
  await context.route(/\/api\/v1\/issue\/[^/]+\/[^/]+\/devstatus(\?|$|\?)/, (route) =>
    json(route, { branches: [], commits: [], pull_requests: [] }),
  );
}

async function openStoryOverview(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  const row = page.locator('.story-row', { hasText: ROW_TITLE }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
}

test('product: manually edit Jira story title and description', async ({
  page,
  context,
}: {
  page: Page;
  context: BrowserContext;
}) => {
  const fx: Fixture = {
    summary: INITIAL_TITLE,
    description_md: INITIAL_DESC,
    fieldsPuts: [],
    descPuts: [],
  };
  await installStubs(context, fx);

  await openStoryOverview(page);

  // ── Baseline: the Jira layout renders with the stubbed title + description ──
  await expect(page.locator('h1.story-title')).toHaveText(INITIAL_TITLE, { timeout: 20_000 });
  await expect(page.locator('.body-wrap .md-body')).toContainText('Original description line one', {
    timeout: 20_000,
  });

  // ── Layout: comments / activity live in the MAIN column below the description
  //    (Jira-style), NOT in the right sidebar. The right column keeps the details.
  await expect(page.locator('.col-left .jira-activity')).toBeVisible({ timeout: 10_000 });
  await expect(
    page.locator('.col-left .jira-activity').getByText('Comments', { exact: true }),
  ).toBeVisible();
  await expect(page.locator('.col-left .comment-textarea')).toBeVisible();
  await expect(page.locator('.col-right')).toContainText('Status');
  await expect(page.locator('.col-right .comment-textarea')).toHaveCount(0);


  // ── 1. Edit the TITLE ──────────────────────────────────────────────────────
  await page.locator('.title-row').hover();
  await page.locator('.title-edit-btn').click();
  const titleInput = page.locator('.title-input');
  await expect(titleInput).toBeVisible({ timeout: 10_000 });
  await expect(titleInput).toHaveValue(INITIAL_TITLE);
  await titleInput.fill(NEW_TITLE);
  await page.locator('.title-edit .field-save-btn').click();

  // Optimistic update: the header reflects the new title without a reload.
  await expect(page.locator('h1.story-title')).toHaveText(NEW_TITLE, { timeout: 10_000 });
  // The write endpoint received `fields.summary = NEW_TITLE`.
  await expect.poll(() => fx.fieldsPuts.length, { timeout: 10_000 }).toBeGreaterThan(0);
  expect(fx.fieldsPuts.at(-1)).toMatchObject({ summary: NEW_TITLE });

  // ── 2. Edit the DESCRIPTION ────────────────────────────────────────────────
  await page.locator('.desc-edit-btn').click();
  const descArea = page.locator('.desc-textarea');
  await expect(descArea).toBeVisible({ timeout: 10_000 });
  await expect(descArea).toHaveValue(INITIAL_DESC);
  await descArea.fill(NEW_DESC);
  await page.locator('.desc-editor .field-save-btn').click();

  // Optimistic update: the rendered body shows the new text; editor closes.
  await expect(descArea).toHaveCount(0, { timeout: 10_000 });
  await expect(page.locator('.body-wrap .md-body')).toContainText('Rewritten description', {
    timeout: 10_000,
  });
  // The description endpoint received the exact markdown we typed.
  await expect.poll(() => fx.descPuts.length, { timeout: 10_000 }).toBeGreaterThan(0);
  expect(fx.descPuts.at(-1)).toBe(NEW_DESC);

  // ── 3. Reload → persisted values re-render (detail GET reads the mutated fx) ─
  await page.reload();
  await openStoryOverview(page);
  await expect(page.locator('h1.story-title')).toHaveText(NEW_TITLE, { timeout: 20_000 });
  await expect(page.locator('.body-wrap .md-body')).toContainText('Rewritten description', {
    timeout: 20_000,
  });

  // ── 4. Cancel path: opening + cancelling an edit leaves content unchanged ───
  await page.locator('.desc-edit-btn').click();
  await expect(page.locator('.desc-textarea')).toBeVisible({ timeout: 10_000 });
  await page.locator('.desc-editor .field-cancel-btn').click();
  await expect(page.locator('.desc-textarea')).toHaveCount(0, { timeout: 10_000 });
  await expect(page.locator('.body-wrap .md-body')).toContainText('Rewritten description');
  // No extra description PUT fired on cancel.
  expect(fx.descPuts.length).toBe(1);
});
