import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Design Hall — review comments (desktop-browser), against the REAL
// /design/artifacts/{id}/comments routes (needs an ottod built with them:
// OTTO_E2E_BIN=<repo>/target/debug/ottod):
//
//   • an empty design says how to start ("Press C or use Comment …");
//   • Comment mode (the stage button or C) drops a pin by click; the
//     composer posts it; the numbered pin draws over the stage and the
//     thread lists it in the Comments tab;
//   • the pin stays on the same relative spot when the device frame changes
//     and when the window resizes;
//   • selecting a thread highlights its pin and a pin selects its thread;
//   • reply, resolve (moves to the collapsed Resolved group), reopen;
//   • a comment made on an older version shows "made on v1", which opens v1;
//   • a comment without a pin, from the tab's composer;
//   • with comment mode OFF the layer lets clicks through to the design.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
let wsId = '';
const stamp = `${Date.now().toString(36)}${process.pid.toString(36)}`;

test.describe.configure({ timeout: 150_000 });

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}

/** An HTML design with v1 and v2 (head). */
async function twoVersions(title: string): Promise<{ id: string; v1: string; v2: string }> {
  const { ctx, base } = await apiCtx();
  const a = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId, format: 'html', studio: 'frames', title,
    content: '<!doctype html><body style="margin:0"><button style="margin:40px">Join</button></body>',
  });
  const v2 = await postJson(ctx, `${base}${V1}/design/artifacts/${a.artifact.id}/versions`, {
    content: '<!doctype html><body style="margin:0"><button style="margin:40px;font-size:20px">Join now</button></body>',
    base_version: a.version.id, message: 'Bigger CTA',
  });
  await ctx.dispose();
  return { id: a.artifact.id, v1: a.version.id, v2: v2.version.id };
}

async function openDesign(page: Page, id: string): Promise<void> {
  await page.goto(`/#/design/a/${id}`);
  await expect(page.getByTestId('design-more')).toBeVisible({ timeout: 90_000 });
  await expect(page.getByTestId('design-pin-layer')).toBeAttached({ timeout: 30_000 });
}

/** A pin's centre as fractions of the pin layer (the stage's content box). */
async function pinFraction(page: Page, n = 0): Promise<{ x: number; y: number }> {
  const layer = (await page.getByTestId('design-pin-layer').boundingBox())!;
  const pin = (await page.getByTestId('design-pin').nth(n).boundingBox())!;
  return { x: (pin.x + pin.width / 2 - layer.x) / layer.width, y: (pin.y + pin.height / 2 - layer.y) / layer.height };
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

test('pin a comment on the design; it stays put across device frames and resizes', async ({ page }) => {
  const { id } = await twoVersions(`Pins ${stamp}`);
  await openDesign(page, id);
  await page.getByTestId('design-tab-comments').click();
  await expect(page.getByTestId('design-comments')).toContainText('No comments yet');
  await expect(page.getByTestId('design-comments')).toContainText('Press C or use Comment to pin one on the design.');

  // Comment mode off: the layer is click-through.
  await expect(page.getByTestId('design-pin-drop')).toHaveCount(0);
  await page.getByTestId('design-comment-mode').click();
  await expect(page.getByTestId('design-comment-mode')).toHaveAttribute('aria-pressed', 'true');
  const layer = (await page.getByTestId('design-pin-layer').boundingBox())!;
  await page.getByTestId('design-pin-drop').click({ position: { x: layer.width * 0.25, y: layer.height * 0.4 } });
  const composer = page.getByTestId('design-pin-composer');
  await expect(composer).toBeVisible();
  await composer.getByRole('textbox', { name: 'Comment text' }).fill('The CTA is too small');
  await composer.getByRole('button', { name: 'Post', exact: true }).click();
  await expect(composer).toHaveCount(0);

  const pin = page.getByTestId('design-pin');
  await expect(pin).toHaveCount(1);
  await expect(pin).toHaveText('1');
  const at = await pinFraction(page);
  expect(at.x).toBeCloseTo(0.25, 1);
  expect(at.y).toBeCloseTo(0.4, 1);
  const thread = page.getByTestId('design-comment');
  await expect(thread).toContainText('The CTA is too small');
  await expect(page.getByTestId('design-tab-comments')).toContainText('1');

  // Leave comment mode (Esc), switch the device frame, resize the window.
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('design-pin-drop')).toHaveCount(0);
  await page.getByRole('group', { name: 'Device frame' }).getByRole('button').nth(1).click();
  await expect.poll(async () => (await pinFraction(page)).x).toBeCloseTo(0.25, 1);
  await page.setViewportSize({ width: 1100, height: 760 });
  await expect.poll(async () => (await pinFraction(page)).y).toBeCloseTo(0.4, 1);
  await expectNoHorizontalOverflow(page);

  // A comment just posted is selected (its pin is pressed). At this width the
  // Details panel is folded away: a click on the pin opens its thread, and the
  // thread's own button clears the selection.
  await expect(pin).toHaveAttribute('aria-pressed', 'true');
  await pin.click();
  await expect(pin).toHaveAttribute('aria-pressed', 'true');
  const unselect = thread.getByRole('button', { name: /Unselect comment 1/ });
  await expect(unselect).toBeVisible();
  await unselect.click();
  // The folded Details view covers the stage here, so read the thread's state.
  await expect(thread.getByRole('button', { name: /^Select comment 1/ })).toBeVisible();
});

test('reply, resolve and reopen; an older-version comment jumps to its version', async ({ page }) => {
  const { id, v1 } = await twoVersions(`Threads ${stamp}`);
  const { ctx, base } = await apiCtx();
  await postJson(ctx, `${base}${V1}/design/artifacts/${id}/comments`, { body: 'Was clearer on v1', version_id: v1, pin: { x: 0.5, y: 0.5 } });
  await ctx.dispose();
  await openDesign(page, id);
  await page.getByTestId('design-tab-comments').click();
  const thread = page.getByTestId('design-comment').first();
  await expect(thread).toContainText('Was clearer on v1');
  // Made on another version: listed with its chip, its pin drawn muted.
  await expect(page.getByTestId('design-pin')).toHaveClass(/muted/);
  await thread.getByTestId('design-comment-version').click();
  await expect(page.getByTestId('design-viewing')).toContainText('v1');

  // A comment about the whole design (no pin) from the tab's composer.
  await page.getByRole('textbox', { name: 'New comment' }).fill('Overall: lighter background');
  await page.getByTestId('design-comments').getByRole('button', { name: 'Comment', exact: true }).click();
  await expect(page.getByTestId('design-comment')).toHaveCount(2);

  // Reply, then resolve: the thread folds into "Resolved (1)".
  await thread.getByRole('button', { name: 'Reply', exact: true }).click();
  await thread.getByRole('textbox', { name: 'Reply' }).fill('Agreed, restoring the spacing');
  await thread.getByRole('button', { name: 'Reply', exact: true }).click();
  await expect(thread).toContainText('Agreed, restoring the spacing');
  await thread.getByTestId('design-comment-resolve').click();
  const folded = page.getByTestId('design-comments-resolved');
  await expect(folded).toHaveText(/Resolved \(1\)/);
  await expect(page.getByTestId('design-pin')).toHaveCount(0);
  await folded.click();
  const resolved = page.getByRole('list', { name: 'Resolved comments' }).getByTestId('design-comment');
  await expect(resolved).toContainText('Resolved by');
  await resolved.getByTestId('design-comment-resolve').click();
  await expect(page.getByTestId('design-comments-resolved')).toHaveCount(0);
  await expect(page.getByTestId('design-pin')).toHaveCount(1);
});

test('C toggles comment mode from the keyboard, never while typing', async ({ page }) => {
  const { id } = await twoVersions(`Keys ${stamp}`);
  await openDesign(page, id);
  await page.getByTestId('design-tab-comments').click();
  await page.getByRole('textbox', { name: 'New comment' }).fill('c');
  await expect(page.getByTestId('design-comment-mode')).toHaveAttribute('aria-pressed', 'false');
  await page.getByRole('textbox', { name: 'New comment' }).fill('');
  await page.getByTestId('design-more').focus();
  await page.keyboard.press('c');
  await expect(page.getByTestId('design-comment-mode')).toHaveAttribute('aria-pressed', 'true');
  // Keyboard drop: Enter pins the middle, arrows move it, the composer posts.
  await page.getByTestId('design-pin-drop').focus();
  await page.keyboard.press('Enter');
  await page.keyboard.press('ArrowRight');
  await expect(page.getByTestId('design-pin-composer')).toHaveAttribute('aria-label', 'New comment at 51% across, 50% down');
  await page.getByRole('textbox', { name: 'Comment text' }).fill('Centre it');
  await page.keyboard.press('Meta+Enter');
  await expect(page.getByTestId('design-pin')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('design-comment-mode')).toHaveAttribute('aria-pressed', 'false');
});
