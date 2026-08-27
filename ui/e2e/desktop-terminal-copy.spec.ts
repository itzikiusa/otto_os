import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// Terminal copy: drag-select in the terminal, then ⌘C/Ctrl+C must put THAT text
// on the clipboard.
//
// This exists because copy was reported broken for weeks and every theory about
// WHY was wrong — clipboard permissions, secure contexts, a native Edit ▸ Copy
// menu. The thing nobody could see from the outside is which of two independent
// steps had failed: does xterm register the drag as a SELECTION at all, and does
// the selection then reach the CLIPBOARD. The assertions below separate them, so
// a future regression names its own cause instead of surfacing as "copy is
// broken" again.
//
// Run with --workers=1 (shared seeded session).

let workspaceId = '';
let sessionId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  sessionId = await seedShellSession(ctx, base, workspaceId);
  // Distinctive, easily-asserted output to select.
  await ctx.post(`${base}/api/v1/sessions/${sessionId}/input`, {
    data: { text: 'for i in $(seq 1 30); do echo "OTTOCOPY-$i"; done', submit: true },
  });
  await new Promise((r) => setTimeout(r, 1200));
  await ctx.dispose();
});

test.beforeEach(async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
  }, workspaceId);
});

test.describe('desktop terminal copy', () => {
  test('drag-select registers an xterm selection and ⌘C copies it', async ({ page }) => {
    await page.goto(`/#/agents/${sessionId}`);
    const rows = page.locator('.xterm-rows');
    await expect(rows).toBeVisible({ timeout: 20_000 });
    await expect(page.getByText('OTTOCOPY-5', { exact: false }).first()).toBeVisible({
      timeout: 20_000,
    });

    // Drag across several rows, the way a user selects output.
    const box = await page.locator('.xterm-screen').boundingBox();
    if (!box) throw new Error('no .xterm-screen box');
    await page.mouse.move(box.x + 8, box.y + 20);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width - 40, box.y + 120, { steps: 25 });
    await page.mouse.up();

    // STEP 1 — did xterm register a selection? `Terminal.svelte` mirrors it into
    // xterm's hidden textarea on every selection change, so a populated mirror
    // is proof the selection exists as far as xterm is concerned. An empty
    // mirror here means the drag never became an xterm selection, and no amount
    // of clipboard work downstream can help.
    const state = await page.evaluate(() => ({
      mirror: (document.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement)?.value ?? '',
      domSel: String(document.getSelection() ?? ''),
      renderer: document.querySelector('.xterm-screen canvas') ? 'canvas/webgl' : 'DOM',
    }));
    console.log('[copy-probe]', JSON.stringify(state).slice(0, 300));
    expect(state.mirror, 'xterm selection mirrored into the helper textarea').toContain('OTTOCOPY');

    // STEP 2 — does the selection reach the clipboard?
    await page.keyboard.press('ControlOrMeta+c');
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()), { timeout: 5_000 })
      .toContain('OTTOCOPY');
  });

  // The regression that actually bit: on an origin the browser de-privileges
  // (Otto's self-signed `0.0.0.0` listener), `navigator.clipboard` is refused.
  // Copy MUST still work, because the DOM renderer gives the document a real
  // selection and the native copy command needs no permission at all. This
  // failed for weeks because the ⌘C handler called preventDefault and routed
  // through the async API, REPLACING the working native path with a blocked one.
  //
  // `clearPermissions` leaves clipboard-write ungranted, which is the closest
  // Playwright gets to a de-privileged origin.
  test('⌘C still copies when the async clipboard API is not permitted', async ({
    page,
    context,
  }) => {
    await context.clearPermissions();
    await page.goto(`/#/agents/${sessionId}`);
    await expect(page.locator('.xterm-rows')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByText('OTTOCOPY-5', { exact: false }).first()).toBeVisible({
      timeout: 20_000,
    });

    const box = await page.locator('.xterm-screen').boundingBox();
    if (!box) throw new Error('no .xterm-screen box');
    await page.mouse.move(box.x + 8, box.y + 20);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width - 40, box.y + 120, { steps: 25 });
    await page.mouse.up();

    // Arm a listener, then press the REAL chord. This is the assertion that
    // matters: if the keydown handler preventDefaults and routes through the
    // async API, the browser never runs its copy command, no `copy` event fires
    // at all, and `__ottoCopied` stays empty — which is exactly how this broke.
    // Reading navigator.clipboard instead would need the permission we withheld.
    await page.evaluate(() => {
      (window as unknown as Record<string, unknown>).__ottoCopied = '';
      document.addEventListener('copy', (e: ClipboardEvent) => {
        (window as unknown as Record<string, unknown>).__ottoCopied =
          e.clipboardData?.getData('text/plain') ?? '';
      });
    });
    await page.keyboard.press('ControlOrMeta+c');
    await expect
      .poll(
        async () =>
          page.evaluate(() => (window as unknown as Record<string, string>).__ottoCopied),
        { timeout: 5_000 },
      )
      .toContain('OTTOCOPY');
  });
});
