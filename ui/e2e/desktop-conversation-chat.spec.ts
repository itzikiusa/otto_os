import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { rmSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { isDesktopProject, watchFatalUiErrors } from './perf';
import { appendRecords, assistantText, toolUse, userPrompt, withUsage, writeChatTranscript } from './chat-fixture';

// Desktop (Chromium + WebKit): the chat, rev 6 "chat v2"
// (docs/design/conversation-view.md §5.2), on a realistic transcript
// (e2e/chat-fixture.ts) fed to a seeded session through the `OTTO_E2E=1`
// `meta.e2e_transcript_path` hook. Covers:
//   • two speakers: a blue bubble for you, a green attributed response, the
//     column using the pane's width;
//   • the settled "Worked for …" fold (first + final message stay), the tool
//     timeline inside it, a step's detail, the failure tail;
//   • thinking as its own marker and the per-turn token line;
//   • IDE code blocks: label, file, line numbers, Wrap, Copy, Open;
//   • links: file references → the side panel at the line, PR / issue chips,
//     external links, links in command output;
//   • the changed-files card, the side panel (markdown / sandboxed HTML / CSV /
//     diff, Esc closes, over the chat in a narrow pane);
//   • jump pill with unread count, ⌥↑ to your previous message, Edit & resend;
//   • composer ⏎ / ⇧⏎ / Stop, the waiting card, a 3×2 tiled grid.
// The chat is opened by its persisted per-session view (not the pane header's
// switch), so the spec doesn't depend on the header's controls.

test.setTimeout(120_000);

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const dirs: string[] = [];
let fatal: string[] = [];

async function seedChat(title: string): Promise<{ id: string; path: string; root: string }> {
  const { dir, path, root } = writeChatTranscript();
  dirs.push(dir);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: root, meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: path } },
  });
  if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  return { id: (await r.json()).id as string, path, root };
}

async function openChat(page: Page, ids: string[], first = ids[0]): Promise<void> {
  await withUsage(page);
  await page.addInitScript(
    ({ ws, ids }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_firstrun_dismissed', '1');
      localStorage.setItem('otto_nav_all_ws', '0');
      for (const id of ids) localStorage.setItem(`otto_session_view:${id}`, 'chat');
    },
    { ws: wsId, ids },
  );
  await page.goto(`/#/agents/${first}`);
  await expect(page.locator('.conv[data-loaded="true"]').first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.conv .turn[data-role="assistant"]').first()).toBeVisible({ timeout: 30_000 });
}

const sessionStatus = async (id: string): Promise<string> =>
  ((await (await ctx.get(`${base}/api/v1/sessions/${id}`)).json()) as { status: string }).status;

/** The fresh shell prints its prompt (→ "working") then goes quiet; the last
 *  response settles (fold + changed-files card) once it does. */
async function settled(page: Page, id: string): Promise<void> {
  await expect.poll(() => sessionStatus(id), { timeout: 30_000 }).not.toBe('working');
  await expect(page.locator('.conv').first().locator('[data-changed-files]')).toHaveCount(2, { timeout: 15_000 });
}

async function stubClipboard(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { __copied: string[] };
    w.__copied = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (t: string) => void w.__copied.push(t) },
    });
  });
}
const copied = (page: Page) => page.evaluate(() => (window as unknown as { __copied: string[] }).__copied);

test.beforeEach(async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  fatal = watchFatalUiErrors(page);
});

test.afterEach(async () => {
  await ctx?.dispose();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
  const seen = fatal;
  fatal = [];
  // A live append while scrolled up used to loop an effect (→ app reload).
  expect(seen, 'no fatal UI error (effect loop / reload) during the run').toEqual([]);
});

test('two speakers: a blue bubble for you, a green attributed response across the pane', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const s = await seedChat('Messages');
  await openChat(page, [s.id]);
  const conv = page.locator('.conv');
  const mine = conv.locator('.turn[data-role="user"]').first();
  await expect(mine.locator('.bubble')).toContainText('jitter_stays_in_range');
  const reply = conv.locator('.turn[data-role="assistant"]').first();
  await expect(reply.locator('.agent-head .avatar')).toBeVisible();
  await expect(reply.locator('.agent-head .agent-name')).toHaveText('Claude');
  await expect(reply.locator('.agent-head .agent-model')).toHaveText('claude-opus-5');
  await expect(reply.locator('.agent-head time')).toHaveAttribute('datetime', /2026-09-28T/);
  await expect(reply.locator('.resp')).toContainText("I'll start by reproducing the failure");
  // Colour identity: the bubble is accent-tinted, the response green-railed —
  // two different hues, neither the neutral surface.
  const tint = await page.evaluate(() => {
    const bubble = document.querySelector('.turn[data-role="user"] .bubble')!;
    const resp = document.querySelector('.turn[data-role="assistant"] .resp')!;
    const conv = document.querySelector('.conv')!;
    return {
      bubbleBg: getComputedStyle(bubble).backgroundColor,
      rail: getComputedStyle(resp).borderInlineStartColor,
      convBg: getComputedStyle(conv).backgroundColor,
    };
  });
  expect(tint.bubbleBg).not.toBe(tint.convBg);
  expect(tint.rail).not.toBe(tint.bubbleBg);
  // The bubble sits on the trailing side; the response spans the pane — no
  // narrow centred reading column any more.
  const b = (await mine.locator('.bubble').boundingBox())!;
  const r = (await reply.locator('.resp').boundingBox())!;
  const pane = (await conv.locator('.conv-list').boundingBox())!;
  expect(b.x + b.width).toBeGreaterThan(r.x + r.width - 40);
  expect(b.x).toBeGreaterThan(r.x);
  expect(r.width, 'the response uses the pane width').toBeGreaterThan(pane.width * 0.85);
  // A day divider precedes the first message.
  await expect(conv.locator('.day').first()).not.toBeEmpty();
  // Actions appear on hover.
  await reply.hover();
  await expect(reply.locator('.copy-btn')).toBeVisible();
});

test('a settled response keeps its first and final message and folds the work', async ({ page }) => {
  const s = await seedChat('Fold');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  const reply = page.locator('.conv .turn[data-role="assistant"]').first();
  const fold = reply.locator('.fold').first();
  const toggle = fold.locator('[data-fold-toggle]');
  await expect(toggle).toContainText('Worked for 2m 7s');
  await expect(toggle).toContainText('5 steps');
  await expect(fold.locator('.fold-fail')).toHaveText('1 failed');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  // First and final messages are on screen; the work between is not.
  await expect(reply.locator('.resp')).toContainText("I'll start by reproducing the failure");
  await expect(reply.locator('.resp h2')).toHaveText('Fixed');
  await expect(reply.getByText('Found it.')).toHaveCount(0);
  await expect(reply.locator('.steps')).toHaveCount(0);
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(reply.getByText('Found it.')).toBeVisible();
  await expect(reply.locator('.tasks .tasks-count')).toHaveText('1 of 4 done');
  // Thinking is its own marker (violet, italic), with the turn's thinking tokens.
  const think = reply.locator('.think').first();
  await expect(think).toContainText('Thought');
  await expect(think).toContainText('300 thinking tokens');
  await think.locator('.think-row').click();
  await expect(think.locator('.think-note')).toContainText('does not save the reasoning text');
  // Token line: in · thinking · out · cache, each with a tooltip.
  const usage = reply.locator('.usage');
  await expect(usage.locator('[data-part="input"]')).toContainText('1.5k in');
  await expect(usage.locator('[data-part="thinking"]')).toHaveAttribute('title', /Thinking tokens/);
  await expect(usage.locator('[data-part="output"]')).toContainText('out');
  await expect(usage.locator('[data-part="cache_read"]')).toContainText('cache read');
  // "Show all work" opens every fold.
  await page.locator('[data-conv-menu]').click();
  await page.getByRole('menuitemcheckbox', { name: /Show all work/ }).click();
  await expect(page.locator('.conv [data-fold-toggle][aria-expanded="false"]')).toHaveCount(0);
});

test('tool activity: the timeline, a step detail, the failure tail and an inline diff', async ({ page }) => {
  const s = await seedChat('Steps');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  const conv = page.locator('.conv');
  await conv.locator('[data-fold-toggle]').first().click();
  const group = conv.locator('.steps:not(.single)').first();
  const head = group.locator('.steps-head');
  await expect(head).toContainText('Ran a command, searched the code, read retry.rs');
  await expect(group.locator('.steps-fail')).toHaveText('1 failed');
  await head.click();
  const failed = group.locator('.step[data-status="err"]');
  await expect(failed.locator('.step-verb')).toHaveText('Ran');
  await expect(failed.locator('.step-target')).toHaveText('cargo test -p otto-net retry -- --test-threads=1');
  await expect(failed.locator('.step-tail.err')).toContainText('test result: FAILED. 13 passed; 1 failed');
  await failed.locator('.step-row').click();
  await expect(failed.locator('.cmd .cmd-text')).toHaveText('cargo test -p otto-net retry -- --test-threads=1');
  // Paths in command output are links into the side panel, at the line.
  const outRef = failed.locator('.out a.file-ref', { hasText: 'crates/otto-net/src/retry.rs:88:9' });
  await expect(outRef).toBeVisible();
  // A read shows source with the file's own line numbers.
  const read = group.locator('.step[data-tool="read"]');
  await expect(read.locator('.step-detail')).toHaveText('crates/otto-net/src', { useInnerText: true });
  await read.locator('.step-row').click();
  await expect(read.locator('.cv-row[data-ln="80"]')).toContainText('pub fn backoff');
  // The edit: +/− on its row, an inline diff when opened, Open diff → panel.
  const editGroup = conv.locator('.steps:not(.single)').nth(1);
  await editGroup.locator('.steps-head').click();
  const edit = editGroup.locator('.step[data-tool="edit"]');
  await expect(edit.locator('.step-stats .add')).toHaveText('+1');
  await edit.locator('.step-row').click();
  await expect(edit.locator('.idiff-row.add')).toContainText('(capped + jitter(capped / 10)).min(max)');
  await expect(edit.locator('.idiff-row.del')).toContainText('capped + jitter(capped / 10)');
  await edit.getByRole('button', { name: 'Open diff' }).click();
  await expect(page.locator('.pv .pdiff-row.add')).toContainText('.min(max)');
});

test('code blocks are IDE-like: language, file, numbered lines, Wrap, Copy, Open', async ({ page }) => {
  const s = await seedChat('Code');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  await stubClipboard(page);
  const block = page.locator('.conv .code-block').last();
  await block.scrollIntoViewIfNeeded();
  await expect(block.locator('.code-lang')).toHaveText('Rust');
  await expect(block.locator('.code-file')).toHaveText('retry.rs');
  await expect(block).toHaveClass(/\bnumbered\b/);
  await expect(block.locator('.cl')).toHaveCount(5);
  // Syntax colours from the whole-block highlight (a keyword span).
  await expect(block.locator('.cl').first().locator('.hljs-keyword').first()).toHaveText('pub');
  const wrap = block.getByRole('button', { name: 'Wrap' });
  await wrap.click();
  await expect(wrap).toHaveAttribute('aria-pressed', 'true');
  await expect(block).toHaveClass(/\bwrap\b/);
  await block.getByRole('button', { name: 'Copy' }).click();
  await expect(block.locator('[data-code-act="copy"]')).toHaveText('Copied');
  const text = (await copied(page))[0];
  expect(text.split('\n')).toHaveLength(5);
  expect(text.startsWith('pub fn backoff(attempt: u32')).toBe(true);
  expect(text).not.toMatch(/^\d/m); // line numbers are never copied
  await block.getByRole('button', { name: 'Open' }).click();
  await expect(page.locator('.pv[data-preview="code"] .cv-row')).toHaveCount(5);
});

test('links: file references open the panel at the line; PR and issue chips; external links', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const s = await seedChat('Links');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  const last = page.locator('.conv .turn[data-role="assistant"]').last();
  const pr = last.locator('a.ref-chip.pr');
  await expect(pr).toHaveText('itzikiusa/otto#7723');
  await expect(pr).toHaveAttribute('href', 'https://github.com/itzikiusa/otto/pull/7723');
  await expect(last.locator('a.ref-chip.issue')).toHaveText('#64');
  // Prose path with a line → Source at that line, beside the chat.
  await last.locator('a.file-ref', { hasText: 'crates/otto-net/src/retry.rs:7' }).click();
  const panel = page.locator('.pv');
  await expect(panel).toBeVisible();
  await expect(panel.locator('.pv-base')).toHaveText('retry.rs');
  await expect(panel.locator('.cv-row.target')).toContainText('let capped = exp.min(max);');
  const chat = (await page.locator('.conv-chat').boundingBox())!;
  const pv = (await panel.boundingBox())!;
  expect(pv.x, 'the panel sits beside the chat on a wide pane').toBeGreaterThanOrEqual(chat.x + chat.width - 4);
  // A code-span path (no line) → the rendered markdown preview.
  await last.locator('a.file-ref', { hasText: 'docs/retry-jitter.md' }).click();
  await expect(panel.locator('.pb-md h1')).toHaveText('Retry jitter');
  // Esc closes it.
  await panel.locator('.pv-seg button', { hasText: 'Source' }).focus();
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
  // An external link opens outside the app (the web build falls back to a new tab).
  await page.context().route('https://docs.rs/**', (r) => r.fulfill({ status: 200, body: 'ok' }));
  const popup = page.waitForEvent('popup');
  await last.locator('a', { hasText: 'fastrand docs' }).click();
  expect((await popup).url()).toContain('docs.rs/fastrand');
});

test('changed files: the card, Show files, Open diff and rendered previews (md, sandboxed HTML, CSV)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const s = await seedChat('Changed');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  const card = page.locator('.conv [data-changed-files]').last();
  await expect(card.locator('.cf-title')).toHaveText('3 changed files');
  await expect(card.locator('.cf-head .cf-stat')).toHaveText('+26 −0');
  await card.locator('.cf-toggle').click();
  await expect(card.locator('.cf-row')).toHaveCount(3);
  await expect(card.locator('.cf-path').first()).toHaveText('docs/retry-jitter.md');
  // Open diff → every file in the panel.
  await card.getByRole('button', { name: 'Open diff' }).click();
  const panel = page.locator('.pv');
  await expect(panel.locator('.pdiff-file')).toHaveCount(3);
  // A row → that file's diff, focused.
  await card.locator('.cf-file', { hasText: 'bench/jitter.csv' }).click();
  await expect(panel.locator('.pdiff-file.focus')).toHaveAttribute('data-diff-file', /bench\/jitter\.csv$/);
  // HTML renders in a sandboxed frame: no scripts ran.
  await card.getByRole('button', { name: /Preview jitter\.html/ }).click();
  const frame = panel.locator('iframe.pb-frame');
  await expect(frame).toHaveAttribute('sandbox', '');
  await expect(panel.frameLocator('iframe.pb-frame').locator('h1')).toHaveText('Jitter distribution — 10 000 runs');
  await expect(panel.frameLocator('iframe.pb-frame').locator('body')).not.toHaveAttribute('data-ran', 'yes');
  // Source / Diff tabs for the same file.
  await panel.locator('.pv-seg button', { hasText: 'Source' }).click();
  await expect(panel.locator('.cv-row').first()).toContainText('<!doctype html>');
  await panel.locator('.pv-seg button', { hasText: 'Diff' }).click();
  await expect(panel.locator('.pdiff-row.add').first()).toBeVisible();
  // CSV → a table.
  await card.getByRole('button', { name: /Preview jitter\.csv/ }).click();
  await expect(panel.locator('.pb-table th').nth(1)).toHaveText('attempt');
  await expect(panel.locator('.pb-table tbody tr')).toHaveCount(4);
  // Full view in a modal.
  await panel.getByRole('button', { name: 'Full view' }).click();
  await expect(page.getByRole('dialog').locator('.pb-table')).toBeVisible();
});

test('a narrow pane shows the panel over the chat', async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 860 });
  const s = await seedChat('Narrow');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  await page.locator('.conv a.file-ref', { hasText: 'docs/retry-jitter.md' }).last().click();
  const panel = page.locator('.pv');
  await expect(panel).toBeVisible();
  const chat = (await page.locator('.conv-main').boundingBox())!;
  const pv = (await panel.boundingBox())!;
  expect(Math.abs(pv.width - chat.width)).toBeLessThanOrEqual(2);
  await panel.getByRole('button', { name: 'Close preview' }).click();
  await expect(panel).toHaveCount(0);
});

test('your messages: ⌥↑ jumps to the previous one; Edit & resend fills the composer', async ({ page }) => {
  const s = await seedChat('Prompts');
  await openChat(page, [s.id]);
  await settled(page, s.id);
  const list = page.locator('.conv-list');
  await list.evaluate((el) => el.scrollTo({ top: el.scrollHeight }));
  await list.focus();
  await page.keyboard.press('Alt+ArrowUp');
  await expect(page.locator('.conv .turn[data-role="user"]').last()).toBeInViewport();
  const mine = page.locator('.conv .turn[data-role="user"]').nth(1);
  await mine.hover();
  await mine.getByRole('button', { name: 'Edit and resend' }).click();
  await expect(page.locator('.composer textarea')).toHaveValue('Great — add the property test, and tell me which callers assume the overshoot.');
  await expect(page.locator('.composer textarea')).toBeFocused();
});

test('scrolled up, new messages raise a jump pill with the unread count', async ({ page }) => {
  const s = await seedChat('Unread');
  await openChat(page, [s.id]);
  const list = page.locator('.conv-list');
  // Let the live tail arm (first GET on a live session), then read from the top.
  await page.waitForTimeout(1500);
  await list.evaluate((el) => el.scrollTo({ top: 0 }));
  await list.dispatchEvent('scroll');
  appendRecords(s.path, userPrompt('One more thing: bump the crate version.') + assistantText('4', 'Bumped otto-net to 0.4.1.'));
  const pill = page.locator('.jump-pill');
  await expect(pill).toContainText('2 new messages', { timeout: 15_000 });
  await expect(pill).toHaveAttribute('data-unread', '2');
  // Nothing moved under the reader while the messages arrived.
  expect(await list.evaluate((el) => el.scrollTop)).toBeLessThan(40);
  await pill.click();
  await expect(page.locator('.conv').getByText('Bumped otto-net to 0.4.1.')).toBeInViewport();
  await expect(pill).toHaveCount(0);
});

test('composer: ⏎ sends, ⇧⏎ adds a line; Stop interrupts a working agent', async ({ page }) => {
  const s = await seedChat('Composer');
  await openChat(page, [s.id]);
  const sent: { text: string; submit?: boolean }[] = [];
  await page.route(`**/sessions/${s.id}/input`, async (route) => {
    sent.push(route.request().postDataJSON());
    await route.fulfill({ status: 200, body: '' });
  });
  const ta = page.locator('.composer textarea');
  await ta.click();
  await ta.pressSequentially('first line');
  await ta.press('Shift+Enter');
  await ta.pressSequentially('second line');
  await expect(ta).toHaveValue('first line\nsecond line');
  expect(sent).toHaveLength(0);
  await ta.press('Enter');
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0]).toEqual({ text: 'first line\nsecond line', submit: true });
  await expect(ta).toHaveValue('');
  // Working: the live line names the current step and Stop sends one Esc.
  await page.unroute(`**/sessions/${s.id}/input`);
  await ctx.post(`${base}/api/v1/sessions/${s.id}/input`, { data: { text: 'while true; do echo tick; sleep 1; done', submit: true } });
  appendRecords(s.path, toolUse('9', 'toolu_live', 'Bash', { command: 'cargo test -p otto-net --release' }));
  const live = page.locator('[data-live-status="working"]');
  await expect(live).toContainText('Claude is working', { timeout: 15_000 });
  await expect(live).toContainText('Running cargo test -p otto-net --release', { timeout: 15_000 });
  // The response being worked on stays flat (no fold) so the work is visible.
  await expect(page.locator('.conv .turn[data-role="assistant"]').last().locator('[data-fold-toggle]')).toHaveCount(0);
  await page.route(`**/sessions/${s.id}/input`, async (route) => {
    sent.push(route.request().postDataJSON());
    await route.fulfill({ status: 200, body: '' });
  });
  await page.locator('.composer').getByRole('button', { name: 'Stop' }).click();
  await expect.poll(() => sent.length).toBe(2);
  expect(sent[1]).toEqual({ text: '\u001b', submit: false });
  // Stop the loop on the real PTY (Ctrl-C) so the session goes quiet again.
  await page.unroute(`**/sessions/${s.id}/input`);
  await ctx.post(`${base}/api/v1/sessions/${s.id}/input`, { data: { text: '\u0003', submit: false } });
});

test('a call that never got a result on a quiet session asks for you, and opens the terminal', async ({ page }) => {
  const s = await seedChat('Waiting');
  await openChat(page, [s.id]);
  // The fresh shell prints its prompt, then goes quiet (idle) — like an agent
  // sitting on a permission prompt.
  await expect.poll(() => sessionStatus(s.id), { timeout: 30_000 }).not.toBe('working');
  appendRecords(s.path, toolUse('7', 'toolu_perm', 'Bash', { command: 'rm -rf target/', description: 'Clean the build' }));
  const card = page.locator('[data-live-status="waiting"]');
  await expect(card).toContainText('Claude is waiting for you', { timeout: 15_000 });
  await expect(card).toContainText('Claude wants to run rm -rf target/');
  // The step itself reads as waiting on you — not running, not "interrupted".
  const step = page.locator('.step[data-status="pending"]').last();
  await expect(step.locator('.step-status.blocked')).toHaveAttribute('aria-label', 'Waiting for you');
  await expect(step.locator('.step-verb')).toHaveText('Wants to run');
  await card.getByRole('button', { name: 'Open terminal' }).click();
  await expect(page.locator('.conv')).toHaveCount(0);
});

test('six tiled panes: every chat fits its narrow pane', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  const seeded = [];
  for (const t of ['One', 'Two', 'Three', 'Four', 'Five', 'Six']) seeded.push(await seedChat(t));
  await openChat(page, seeded.map((x) => x.id));
  await page.locator('button[aria-label="Tiled view"]').first().click();
  await expect(page.locator('[data-tile-id]')).toHaveCount(6, { timeout: 20_000 });
  await expect(page.locator('.conv[data-loaded="true"]')).toHaveCount(6, { timeout: 30_000 });
  const convs = page.locator('.conv');
  for (let i = 0; i < 6; i++) {
    const c = convs.nth(i);
    const w = await c.evaluate((el) => el.getBoundingClientRect().width);
    expect(w, 'a 3-column tile is a narrow pane').toBeLessThan(480);
    // No sideways scroll anywhere in the chat column or the composer.
    expect(await c.locator('.conv-list').evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    expect(await c.locator('.composer').evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    // Chrome sheds by pane width: no header stats, no key hints, send reachable.
    await expect(c.locator('.conv-head .stats')).toBeHidden();
    await expect(c.locator('.composer .keys')).toBeHidden();
    const box = (await c.boundingBox())!;
    const send = (await c.locator('.composer .send').boundingBox())!;
    expect(send.x + send.width).toBeLessThanOrEqual(box.x + box.width + 1);
    // A user bubble stays inside the pane.
    const bubble = (await c.locator('.bubble').last().boundingBox())!;
    expect(bubble.x).toBeGreaterThanOrEqual(box.x - 1);
    expect(bubble.x + bubble.width).toBeLessThanOrEqual(box.x + box.width + 1);
  }
});
