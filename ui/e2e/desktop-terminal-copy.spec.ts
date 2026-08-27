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

  // The subtler half of the same bug. The selection mirror puts the text INSIDE
  // xterm's hidden textarea and calls `select()` — and Chromium does not expose
  // a textarea's internal selection through `document.getSelection()`. So any
  // "is there something to copy?" check written against the document Selection
  // API reads EMPTY exactly when the mirror is doing its job, and sends ⌘C down
  // the permissioned path the browser is refusing. Forcing the selection into
  // the textarea reproduces that state directly.
  test('⌘C copies when the selection lives only inside the helper textarea', async ({
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

    const forced = await page.evaluate(() => {
      (window as unknown as Record<string, unknown>).__ottoCopied = '';
      document.addEventListener('copy', (e: ClipboardEvent) => {
        (window as unknown as Record<string, unknown>).__ottoCopied =
          e.clipboardData?.getData('text/plain') ?? '';
      });
      // Collapse the document selection into the textarea, the state the mirror
      // actually produces.
      const ta = document.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement | null;
      ta?.focus();
      ta?.select();
      return {
        mirror: (ta?.value ?? '').slice(0, 200),
        domSel: String(document.getSelection() ?? '').slice(0, 60),
      };
    });
    // Not asserted as empty: Chromium's behaviour here is version-dependent,
    // which is exactly why the copy path must not be gated on it. Recorded so a
    // future failure shows which selection actually existed.
    console.log('[forced-selection]', JSON.stringify(forced));
    expect(forced.mirror, 'the mirror holds the terminal selection').toContain('OTTOCOPY');

    await page.keyboard.press('ControlOrMeta+c');
    await expect
      .poll(
        async () =>
          page.evaluate(() => (window as unknown as Record<string, string>).__ottoCopied),
        { timeout: 5_000 },
      )
      .toContain('OTTOCOPY');
  });

  // The bug that made copy look permanently broken. A TUI that turns on mouse
  // reporting (claude, codex, vim, htop — `CSI ?1000h`) owns the mouse: xterm
  // hands the drag to the app and cancels it, so NO selection is created. The
  // only escape is `shouldForceSelection`, which on macOS is
  // `altKey && macOptionClickForcesSelection` — and that option defaults to
  // FALSE, so there was no way to select at all. Every copy path is gated on
  // `hasSelection()`, so all of them silently did nothing and Edit ▸ Copy came
  // up greyed. `Terminal.svelte` now enables the option; ⌥-drag is the same
  // gesture iTerm2 and Terminal.app use.
  test('⌥-drag selects even while the app has mouse reporting on', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    await ctx.post(`${base}/api/v1/sessions/${sessionId}/input`, {
      // Enable mouse reporting, then print fresh output to select.
      data: { text: 'printf "\\033[?1000h"; for i in $(seq 1 20); do echo "OTTOMOUSE-$i"; done', submit: true },
    });
    await ctx.dispose();

    await page.goto(`/#/agents/${sessionId}`);
    await expect(page.locator('.xterm-rows')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByText('OTTOMOUSE-5', { exact: false }).first()).toBeVisible({
      timeout: 20_000,
    });

    const box = await page.locator('.xterm-screen').boundingBox();
    if (!box) throw new Error('no .xterm-screen box');
    const readMirror = () =>
      page.evaluate(
        () => (document.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement)?.value ?? '',
      );
    const drag = async (alt: boolean) => {
      await page.evaluate(() => {
        const ta = document.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement | null;
        if (ta) ta.value = '';
      });
      if (alt) await page.keyboard.down('Alt');
      await page.mouse.move(box.x + 8, box.y + 20);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width - 40, box.y + 120, { steps: 25 });
      await page.mouse.up();
      if (alt) await page.keyboard.up('Alt');
      await page.waitForTimeout(400);
      return readMirror();
    };

    // Plain drag belongs to the app while mouse reporting is on — that is
    // correct terminal behaviour and must NOT be "fixed" by stealing the mouse.
    await page.evaluate(() => {
      (window as unknown as Record<string, unknown>).__md = [];
      document.querySelector('.xterm-screen')?.addEventListener(
        'mousedown',
        (e) => {
          ((window as unknown as Record<string, unknown>).__md as unknown[]).push({
            alt: (e as MouseEvent).altKey,
            button: (e as MouseEvent).button,
          });
        },
        true,
      );
    });
    const plain = await drag(false);
    // ⌥-drag must select regardless. This is the assertion that fails without
    // `macOptionClickForcesSelection`.
    const withAlt = await drag(true);
    const md = await page.evaluate(
      () => (window as unknown as Record<string, unknown>).__md,
    );
    console.log(
      '[mouse-report]',
      JSON.stringify({ plain: plain.slice(0, 30), withAlt: withAlt.slice(0, 30), mousedowns: md }),
    );
    // The invariant is about WHETHER a selection happens, not which lines the
    // drag happened to cover — the viewport scrolls as output arrives.
    expect(plain, 'plain drag belongs to the app while mouse reporting is on').toBe('');
    expect(
      withAlt.length,
      '⌥-drag forces a selection despite mouse reporting',
    ).toBeGreaterThan(0);
    // It must be the WHOLE dragged region, not one line — the drag spans rows.
    expect(
      withAlt.split('\n').length,
      '⌥-drag selects every row it covers, not a single line',
    ).toBeGreaterThan(1);

    // End-to-end: ⌥-drag then plain ⌘C (⌥ is for the DRAG only) must put the
    // full multi-line selection on the clipboard. This is the path a user
    // actually takes; right-click ▸ Copy is the fallback, and it only preserves
    // a drag when the click lands INSIDE the selection (xterm's
    // `rightClickSelect` → `_isClickInSelection`), otherwise it re-selects the
    // word under the pointer.
    await page.evaluate(() => {
      (window as unknown as Record<string, unknown>).__ottoCopied = '';
      document.addEventListener('copy', (e: ClipboardEvent) => {
        (window as unknown as Record<string, unknown>).__ottoCopied =
          e.clipboardData?.getData('text/plain') ?? '';
      });
    });
    await page.keyboard.press('ControlOrMeta+c');
    await page.waitForTimeout(500);
    const copied = await page.evaluate(
      () => (window as unknown as Record<string, string>).__ottoCopied,
    );
    console.log('[alt-drag-copy]', JSON.stringify(copied.slice(0, 60)));
    expect(copied.split('\n').length, '⌘C copies every selected row').toBeGreaterThan(1);
  });
});
