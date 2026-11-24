import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';
import { setup, shoot } from './threadsReviewNotesFixture';

// ─────────────────────────────────────────────────────────────────────────────
// Review notes on the Changes diff (runtime/reviewNotes.ts, after Orca's diff
// comments): a line or ⇧-range gets a note, notes show under their lines and
// in one "Review · N notes" bar, survive a reload (per device), go to the
// focused thread as ONE message at once (Send to agent; "Add to message"
// puts them in the draft instead), read Sent only once the send was taken,
// render in the timeline as a compact "Review · N notes" block, can be
// resolved, and turn Outdated when the noted lines change.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

test('desktop: note a range, it survives a reload, goes to the agent as one message, then Sent → Resolved; Outdated when the lines change', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  const { sent, diff } = await setup(page, { inspector: true });
  await page.goto('/#/threads/notes-alpha');
  const tab = page.getByTestId('inspector-changes-tab');
  await expect(tab.getByRole('button', { name: 'Note on src/a.ts new line 9', exact: true })).toBeVisible({ timeout: 30_000 });
  // A range: a line, then ⇧-click another in the same hunk.
  await tab.getByRole('button', { name: 'Note on src/a.ts old line 9', exact: true }).click();
  await tab.getByRole('button', { name: 'Note on src/a.ts new line 10', exact: true }).click({ modifiers: ['Shift'] });
  const editor = tab.getByTestId('diff-note-editor');
  await expect(editor).toContainText('src/a.ts · new 9–10 · old 9');
  await expect(tab.getByTestId('diff-note-input')).toBeFocused();
  await tab.getByTestId('diff-note-input').fill('Keep save() for the offline case.');
  await tab.getByTestId('diff-note-input').press('ControlOrMeta+Enter');
  const note = tab.getByTestId('diff-note');
  await expect(note).toHaveCount(1);
  await expect(note.getByTestId('diff-note-state')).toHaveText('Draft');
  const bar = tab.getByTestId('diff-review-bar');
  await expect(bar).toContainText('Review');
  await expect(bar).toContainText('1 note');
  await shoot(page, 'review-notes-desktop');
  // Per thread, per device: a reload keeps it.
  await page.reload();
  await expect(tab.getByTestId('diff-note')).toHaveCount(1, { timeout: 30_000 });
  // "Add to message": one context item in the thread's draft, the notes still Draft.
  const thread = page.getByTestId('runtime-thread-notes-alpha');
  await bar.getByTestId('diff-review-draft').click();
  await expect(thread.getByTestId('runtime-context-chips')).toContainText('Review · 1 note');
  await expect(note.getByTestId('diff-note-state')).toHaveText('Draft');
  expect(sent.length).toBe(0);
  // "Send to agent": one message now; Sent once the daemon took it.
  await bar.getByTestId('diff-review-send').click();
  await expect.poll(() => sent.length).toBe(1);
  await expect(note.getByTestId('diff-note-state')).toHaveText('Sent');
  for (const t of ['Review notes on the diff', '1. src/a.ts (new 9–10 · old 9)', '> -save(note);', '> +queue.push(note);', 'Keep save() for the offline case.']) expect(sent[0]).toContain(t);
  // The agent's next change rewrites the noted lines: Outdated, not lost.
  diff.lines.splice(1, 3, { origin: 'add', content: 'queue.push(note, { offline: true });', old_line: null, new_line: 9 });
  await tab.getByRole('button', { name: 'Refresh changes' }).click();
  await page.getByTestId('threads-tab-changes').click().catch(() => {});
  await page.reload();
  await expect(tab.getByTestId('diff-note-state').or(bar)).toBeVisible({ timeout: 30_000 });
  await bar.getByRole('button', { name: /Review/ }).click();
  await expect(bar.getByTestId('diff-note-state')).toHaveText('Outdated');
  await bar.getByTestId('diff-note-resolve').click();
  await expect(bar.getByTestId('diff-note-state')).toHaveText('Resolved');
  await bar.getByTestId('diff-review-clear').click();
  await expect(bar).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});

