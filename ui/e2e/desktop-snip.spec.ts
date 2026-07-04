import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx } from './seed';

// Snipping tool (screenshot → annotate → clipboard) — desktop-browser only.
//
// The isolated daemon runs with OTTO_E2E=1, so clipboard writes skip the real
// macOS pasteboard and land ONLY in `<dataDir>/snips/clipboard-last.png` — the
// observable sink these tests assert against (byte-exact for the auto-copy on
// create, magic+IHDR for the flattened annotated export). Capture itself
// (interactive `screencapture -i`) can't run headless; the upload endpoint is
// the seed path and the capture plumbing is covered by Rust tests
// (crates/otto-server/tests/snips.rs).
//
// Serial: the tests walk one snip through draw → undo/redo → select/delete →
// copy/delete, and the clipboard sink is a single global file.

test.describe.configure({ mode: 'serial' });

/** 400×300 light-gray PNG (base64) — big enough for comfortable canvas drags. */
const PNG_B64 =
  'iVBORw0KGgoAAAANSUhEUgAAAZAAAAEsCAYAAADtt+XCAAAEO0lEQVR4nO3VoQ0AIADAMP5/FcMHcAMzhKSifm5jzrUB4NZ4HQDAnwwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDASAxEAASAwEgMRAAEgMBIDEQABIDu+ZX0oRyzxWAAAAAElFTkSuQmCC';

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';

function dataDir(): string {
  const meta = JSON.parse(
    readFileSync(join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json'), 'utf8'),
  ) as { dataDir: string };
  return meta.dataDir;
}

function clipboardSink(): Buffer {
  return readFileSync(join(dataDir(), 'snips', 'clipboard-last.png'));
}

function pngDims(buf: Buffer): { w: number; h: number } {
  expect(buf.subarray(0, 8)).toEqual(Buffer.from('\x89PNG\r\n\x1a\n', 'latin1'));
  return { w: buf.readUInt32BE(16), h: buf.readUInt32BE(20) };
}

let snipId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  if (!snipId) {
    const { ctx, base } = await apiCtx();
    const r = await ctx.post(`${base}/api/v1/snips`, {
      data: { data_b64: PNG_B64, filename: 'e2e.png' },
    });
    expect(r.ok()).toBeTruthy();
    snipId = (await r.json()).id as string;
    await ctx.dispose();
  }
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '1'));
});

test('upload auto-copies the original to the clipboard sink', async () => {
  const sink = clipboardSink();
  expect(sink.equals(Buffer.from(PNG_B64, 'base64'))).toBeTruthy();
  expect(pngDims(sink)).toEqual({ w: 400, h: 300 });
});

test('editor draws box + arrow + text and auto-copies the annotated PNG', async ({ page }) => {
  const original = Buffer.from(PNG_B64, 'base64');
  await page.goto(`/#/snip/${snipId}`);
  const canvas = page.locator('.snip-canvas');
  await expect(canvas).toBeVisible({ timeout: 15_000 });
  const editor = page.locator('.snip-editor');

  // Rectangle (default tool) — drag on the canvas.
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + 40, box.y + 40);
  await page.mouse.down();
  await page.mouse.move(box.x + 160, box.y + 120, { steps: 4 });
  await page.mouse.up();
  await expect(editor).toHaveAttribute('data-count', '1');

  // Arrow.
  await page.locator('[data-tool="arrow"]').click();
  await page.mouse.move(box.x + 200, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(box.x + 300, box.y + 120, { steps: 4 });
  await page.mouse.up();
  await expect(editor).toHaveAttribute('data-count', '2');

  // Text: click, type into the floating textarea, ⌘↩ commits.
  await page.locator('[data-tool="text"]').click();
  await canvas.click({ position: { x: 60, y: 200 } });
  const entry = page.locator('.snip-textentry');
  await expect(entry).toBeVisible();
  await entry.fill('hello from e2e');
  await page.keyboard.press('Meta+Enter');
  await expect(editor).toHaveAttribute('data-count', '3');

  // Auto-copy: debounced flatten → POST → sink updated with the ANNOTATED png.
  await expect(page.locator('.snip-copied')).toHaveText('Copied ✓', { timeout: 15_000 });
  const sink = clipboardSink();
  expect(sink.equals(original)).toBeFalsy();
  expect(pngDims(sink)).toEqual({ w: 400, h: 300 }); // natural resolution kept
});

test('undo/redo, color choice, select + delete', async ({ page }) => {
  await page.goto(`/#/snip/${snipId}`);
  const canvas = page.locator('.snip-canvas');
  await expect(canvas).toBeVisible({ timeout: 15_000 });
  const editor = page.locator('.snip-editor');
  await expect(editor).toHaveAttribute('data-count', '0'); // annotations are per-editing-session

  // Draw an ellipse in blue, M stroke.
  await page.locator('[data-color="#0090ff"]').click();
  await page.locator('[data-tool="ellipse"]').click();
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + 80, box.y + 80);
  await page.mouse.down();
  await page.mouse.move(box.x + 220, box.y + 180, { steps: 4 });
  await page.mouse.up();
  await expect(editor).toHaveAttribute('data-count', '1');

  // Undo → gone; redo → back.
  await page.locator('[data-act="undo"]').click();
  await expect(editor).toHaveAttribute('data-count', '0');
  await page.locator('[data-act="redo"]').click();
  await expect(editor).toHaveAttribute('data-count', '1');

  // Select it (click inside its bbox) and delete with the keyboard.
  await page.locator('[data-tool="select"]').click();
  await canvas.click({ position: { x: 150, y: 130 } });
  await page.keyboard.press('Delete');
  await expect(editor).toHaveAttribute('data-count', '0');
});

test('copy endpoint prefers annotated; list + delete round-trip', async () => {
  const { ctx, base } = await apiCtx();
  try {
    const copy = await ctx.post(`${base}/api/v1/snips/${snipId}/copy`, { data: {} });
    expect(copy.ok()).toBeTruthy();
    expect((await copy.json()).copied).toBe(true);

    const list = await ctx.get(`${base}/api/v1/snips`);
    expect(list.ok()).toBeTruthy();
    const snips = (await list.json()) as { id: string; has_annotated: boolean }[];
    const mine = snips.find((s) => s.id === snipId);
    expect(mine).toBeTruthy();
    expect(mine!.has_annotated).toBe(true); // the editor test saved an annotated export

    const del = await ctx.delete(`${base}/api/v1/snips/${snipId}`);
    expect(del.status()).toBe(204);
    const gone = await ctx.get(`${base}/api/v1/snips/${snipId}/image`);
    expect(gone.status()).toBe(404);
  } finally {
    await ctx.dispose();
  }
});

test('deleted snip renders the missing state in the editor', async ({ page }) => {
  await page.goto(`/#/snip/${snipId}`);
  await expect(page.locator('.snip-missing')).toBeVisible({ timeout: 15_000 });
});

test('sidebar: Connections is a plain row without an open-connections list', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole('button', { name: 'Connections' })).toBeVisible();
  await expect(page.getByText('No open connections')).toHaveCount(0);
});
