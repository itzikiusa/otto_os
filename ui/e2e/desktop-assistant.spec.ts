import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow, expectAccessible } from './helpers';
import { assistantState, mockAssistant, type AssistantMock } from './assistant-fixture';

// ─────────────────────────────────────────────────────────────────────────────
// Otto Assistant (#/assistant) — desktop-browser.
//
// The isolated daemon serves auth/workspaces/WS; the assistant API itself is
// served by a stateful page.route fixture (assistant-fixture.ts) so the whole
// UI can be driven before (and independently of) the backend: list/detail
// opening on the latest thread, the chat's action cards (memory Undo,
// reminder, browser take-over/hand-back, delegation, the approval shape with
// scoped Always allow), the composer's @codex hint + model pin, the Tasks /
// Memory / Permissions tabs, Settings → Assistant routing, error states and
// the phone push-navigation.
// ─────────────────────────────────────────────────────────────────────────────

// The UI registers a fetch-proxying service worker that would bypass the mock.
test.use({ serviceWorkers: 'block' });

async function open(page: Page, route = 'assistant', s?: AssistantMock): Promise<AssistantMock> {
  const state = await mockAssistant(page, s ?? assistantState());
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  return state;
}

function lastCall(s: AssistantMock, method: string, path: string | RegExp) {
  return [...s.calls].reverse().find((c) => c.method === method && (typeof path === 'string' ? c.path === path : path.test(c.path)));
}

test('opens on the latest thread with Spaces, Recent and the needs-you badge', async ({ page }) => {
  await open(page);
  const header = page.locator('[data-testid="page-header"]');
  await expect(header.locator('h1')).toHaveText('Lisbon weekend');
  await expect(page.locator('[data-testid="assistant-needs-badge"]')).toHaveText('2');

  const list = page.getByRole('navigation', { name: 'Assistant threads' });
  await expect(list.getByRole('button', { name: /01\s*Lisbon weekend/ })).toHaveAttribute('aria-current', 'page');
  await expect(list.getByRole('button', { name: /04\s*Empty space/ })).toBeVisible();
  await expect(list.getByRole('button', { name: /Receipts cleanup/ })).toBeVisible();

  // The chat is the Conversation view, not a terminal.
  const thread = page.locator('[data-testid="assistant-thread"]');
  await expect(thread.locator('.xterm')).toHaveCount(0);
  await expect(thread.getByText('Find me a hotel in Lisbon')).toBeVisible();
  await expect(thread.locator('[data-testid="provider-badge"]').first()).toHaveText('Claude · Sonnet');
  await expect(thread.locator('table')).toContainText('Casa do Largo');
  await expect(thread.locator('[data-testid="line-delegation"]')).toContainText('Asked Daily Recap');
  // The rail (wide window) lists what needs you.
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.locator('[data-testid="assistant-rail"]')).toContainText('Send the shortlist to Dana');
  await expectNoHorizontalOverflow(page);
});

test('memory chip Undo forgets the memory and can restore it', async ({ page }) => {
  const s = await open(page);
  const chip = page.locator('[data-testid="memory-new"]');
  await expect(chip).toContainText('city-break budget €180/night');
  await chip.getByRole('button', { name: /Undo/ }).click();
  await expect(page.locator('[data-testid="memory-undone"]')).toContainText('Forgot: city-break budget');
  expect(lastCall(s, 'DELETE', '/assistant/memory/mem-budget')).toBeTruthy();
  await page.locator('[data-testid="memory-undone"]').getByRole('button', { name: 'Restore' }).click();
  await expect(page.locator('[data-testid="memory-new"]')).toBeVisible();
  expect(lastCall(s, 'POST', '/assistant/memory/undo')?.body).toEqual({ undo_token: 'undo-mem-budget' });
});

test('reminder and browser cards: take over, then hand back', async ({ page }) => {
  const s = await open(page);
  const reminder = page.locator('[data-testid="card-reminder"]');
  await expect(reminder).toContainText('Book the Sintra train');
  await expect(reminder).toContainText('Scheduled');
  await expect(reminder).toContainText('Tomorrow');

  const card = page.locator('[data-testid="card-browser"]');
  await expect(card).toContainText('Filtering: Alfama');
  await expect(card.getByRole('button', { name: 'Watch live' })).toBeDisabled();
  await card.getByRole('button', { name: 'Take over' }).click();
  await expect(card).toContainText('You have control');
  expect(lastCall(s, 'POST', '/assistant/tasks/task-hotels/takeover')).toBeTruthy();
  await card.getByRole('button', { name: 'Hand back' }).click();
  await expect(card.getByRole('button', { name: 'Take over' })).toBeVisible();
  expect(lastCall(s, 'POST', '/assistant/tasks/task-hotels/handback')).toBeTruthy();
});

test('the approval card shows where / who / why / what and approves with a scoped always-allow', async ({ page }) => {
  const s = await open(page);
  const card = page.locator('[data-testid="card-needs-approval"]');
  await expect(card).toContainText('Approval needed');
  await expect(card).toContainText('Telegram → Dana (via your Otto bot)');
  await expect(card).toContainText('Dana only');
  await expect(card).toContainText('she usually picks the hotel');
  await expect(card.getByLabel('Exactly what is sent')).toContainText('Which one?');

  // Deny asks for an optional reason in a sheet; cancel keeps it pending.
  await card.getByRole('button', { name: 'Deny…' }).click();
  const sheet = page.getByRole('dialog', { name: 'Deny request' });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('button', { name: 'Cancel' }).click();
  await expect(sheet).toHaveCount(0);

  await card.getByLabel(/Always allow for Dana on Telegram/).check();
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toContainText('Approved by you');
  expect(lastCall(s, 'POST', '/assistant/tasks/task-appr/approve')?.body).toEqual({ always_allow: true });
  await expect(page.locator('[data-testid="assistant-needs-badge"]')).toHaveText('1');
});

test('purchases never offer "Always allow"', async ({ page }) => {
  const s = assistantState();
  const t = s.tasks.find((x) => x.id === 'task-appr')!;
  t.needs_you!.approval = { ...t.needs_you!.approval!, category: 'purchase', always_allow_allowed: true, where: 'booking-site.example checkout' };
  await open(page, 'assistant', s);
  const card = page.locator('[data-testid="card-needs-approval"]');
  await expect(card.getByRole('button', { name: 'Buy', exact: true })).toBeVisible();
  await expect(card.getByRole('checkbox')).toHaveCount(0);
});

test('composer: @codex routes one turn, the model chip pins the thread, the mic is honest', async ({ page }) => {
  const s = await open(page);
  const box = page.getByRole('textbox', { name: 'Message Otto' });
  await box.fill('@codex fix this script');
  await expect(page.locator('[data-testid="route-hint"]')).toContainText('This message goes to Codex');
  await box.press('Enter');
  await expect(page.locator('[data-testid="assistant-thread"]').getByText('fix this script', { exact: true })).toBeVisible();
  expect(lastCall(s, 'POST', '/assistant/threads/th-personal/turns')?.body).toMatchObject({ text: '@codex fix this script', origin: 'app' });
  await expect(box).toHaveValue('');

  const mic = page.getByRole('button', { name: /Dictate/ });
  await expect(mic).toBeDisabled();
  await expect(mic).toHaveAttribute('title', 'Voice arrives in a later phase');

  await page.locator('[data-testid="model-chip"]').click();
  const sheet = page.getByRole('dialog', { name: 'Model for this thread' });
  await sheet.getByRole('button', { name: 'Codex' }).click();
  await sheet.getByRole('button', { name: 'Pin to thread' }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.locator('[data-testid="model-chip"]')).toContainText('Codex');
  expect(lastCall(s, 'POST', '/assistant/threads/th-personal/route')?.body).toEqual({ provider: 'codex', model: null });
});

test('Tasks tab decides a limit: continue on Codex', async ({ page }) => {
  const s = await open(page, 'assistant/tasks');
  const tasks = page.locator('[data-testid="assistant-tasks"]');
  await expect(tasks.getByRole('heading', { name: /Needs you/ })).toContainText('2');
  const limit = tasks.locator('[data-testid="card-needs-limit"]');
  await expect(limit).toContainText('Claude limit reached until');
  await expect(limit).toContainText('continue on Codex?');
  await limit.getByRole('button', { name: /Continue on Codex/ }).click();
  await expect(limit).toContainText('Answered');
  expect(lastCall(s, 'POST', '/assistant/tasks/task-limit/approve')?.body).toEqual({ provider: 'codex' });
  await expect(tasks.getByRole('heading', { name: /Running/ })).toBeVisible();
  await expect(tasks.getByRole('heading', { name: /Done/ })).toBeVisible();
});

test('Memory tab: profile save, forget with Undo, review queue, Hermes only queues', async ({ page }) => {
  const s = await open(page, 'assistant/memory');
  const mem = page.locator('[data-testid="assistant-memory"]');
  const profile = mem.getByLabel('Profile (markdown)');
  await expect(profile).toHaveValue(/Travels with Dana/);
  const save = mem.getByRole('button', { name: 'Save' });
  await expect(save).toBeDisabled();
  await profile.fill('- Prefers aisle seats');
  await save.click();
  await expect(mem.getByText('Saved', { exact: true })).toBeVisible();
  expect(lastCall(s, 'PUT', '/assistant/memory')?.body).toEqual({ profile: { content: '- Prefers aisle seats', version: 'v1' } });

  await mem.getByRole('button', { name: 'Forget “Prefers quiet rooms, high floor”' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Forget' }).click();
  await expect(mem.getByText('Forgot “Prefers quiet rooms, high floor”')).toBeVisible();
  await mem.getByRole('button', { name: 'Undo' }).click();
  expect(lastCall(s, 'POST', '/assistant/memory/undo')?.body).toEqual({ undo_token: 'undo-mem-quiet' });

  const review = mem.locator('[data-testid="memory-review"]');
  await expect(review).toContainText('Zendesk macros');
  await review.getByRole('button', { name: 'Keep' }).click();
  await expect(review).toHaveCount(0);
  expect(lastCall(s, 'POST', '/assistant/memory/mem-h1/accept')).toBeTruthy();

  const hermes = mem.locator('[data-testid="hermes-import"]');
  await hermes.getByRole('button', { name: 'Review 3…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Queue for review' }).click();
  await expect(mem.locator('[data-testid="memory-review"]')).toContainText('Prefers terse replies in Slack');
  // Nothing was accepted on its own.
  expect(s.calls.filter((c) => /\/accept$/.test(c.path))).toHaveLength(1);
});

test('Permissions is an honest read-only preview', async ({ page }) => {
  await open(page, 'assistant/permissions');
  const perm = page.locator('[data-testid="assistant-permissions"]');
  await expect(perm).toContainText('Read-only for now');
  await expect(perm.locator('input, select, textarea')).toHaveCount(0);
  await expect(perm).toContainText('Never “Always allow”');
});

test('Settings → Assistant saves routing rules and limit behaviour', async ({ page }) => {
  const s = await open(page, 'settings/assistant');
  const form = page.locator('[data-testid="assistant-routing"]');
  await expect(form.locator('[data-testid="sub-claude"]')).toContainText('Limit until');
  await expect(form.locator('[data-testid="sub-claude"]')).toContainText('69% of this week’s load');
  const save = page.locator('[data-testid="page-header"]').getByRole('button', { name: 'Save' });
  await expect(save).toBeDisabled();
  await expect(form.getByRole('radio', { name: /Ask before switching provider/ })).toBeChecked();

  await form.locator('[data-testid="rule-voice"]').getByRole('button', { name: 'Codex' }).click();
  await form.getByLabel('Extra words that mean “code”').fill('terraform, jq');
  await form.getByRole('radio', { name: /Switch automatically/ }).check();
  await save.click();
  await expect(form.getByText('Saved. New turns use these rules.')).toBeVisible();
  const body = lastCall(s, 'PUT', '/assistant/routing')?.body as Record<string, unknown>;
  expect(body.auto_failover).toBe(true);
  expect(body.extra_keywords).toEqual({ code: ['terraform', 'jq'], hard: [] });
  expect((body.targets as Record<string, { provider: string }>).voice.provider).toBe('codex');
  await expect(save).toBeDisabled();
});

test('a failed thread list shows inline with Retry; a missing API says so', async ({ page }) => {
  const s = assistantState();
  s.fail['/assistant/threads'] = 500;
  await open(page, 'assistant', s);
  await expect(page.getByText('Couldn’t load your threads.')).toBeVisible();
  delete s.fail['/assistant/threads'];
  await page.getByRole('button', { name: 'Retry' }).click();
  await expect(page.locator('[data-testid="page-header"] h1')).toHaveText('Lisbon weekend');

  // An older daemon without the assistant routes: say so, don't spin or blame.
  s.fail['/assistant'] = 404;
  await page.reload();
  await expect(page.getByText('The assistant isn’t available yet')).toBeVisible();
});

test('an empty assistant invites the first conversation', async ({ page }) => {
  const s = assistantState();
  s.threads = [];
  await open(page, 'assistant', s);
  await expect(page.getByText('Meet Otto, your assistant')).toBeVisible();
  await page.getByRole('button', { name: 'Start a conversation' }).click();
  await expect(page.locator('[data-testid="page-header"] h1')).toHaveText('Personal');
  expect(lastCall(s, 'POST', '/assistant/threads')?.body).toEqual({ space_slot: 1, title: 'Personal' });
});

test('phone: the list is the page, a thread pushes in with a back button', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page);
  const list = page.getByRole('navigation', { name: 'Assistant threads' });
  await expect(list).toBeVisible();
  await list.getByRole('button', { name: /Lisbon weekend/ }).click();
  await expect(page.locator('[data-testid="assistant-thread"]')).toContainText('Find me a hotel');
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Back to threads' }).click();
  await expect(list).toBeVisible();
});

test('accessibility: no critical axe violations and no serious contrast issues', async ({ page }) => {
  await open(page);
  await expect(page.locator('[data-testid="card-needs-approval"]')).toBeVisible();
  const v = await expectAccessible(page);
  expect(v.filter((x) => x.id === 'color-contrast' && x.impact === 'serious').map((x) => x.nodes.map((n) => n.target))).toEqual([]);
});
