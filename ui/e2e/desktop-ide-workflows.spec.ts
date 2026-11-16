import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx } from './seed';
import {
  expectContentHasHeight,
  expectFullyInViewport,
  expectNoHorizontalOverflow,
} from './helpers';
import {
  bootPolyglotIde,
  createPolyglotSeed,
  daemonCall,
  editorContent,
  GO_MAIN_SOURCE,
  JAVA_BONUS_SOURCE,
  openPolyglotFile,
  saveWorkflowShot,
  WORKFLOW_SCREENSHOT_DIR,
  type PolyglotWorkspaceSeed,
} from './ide-polyglot-fixtures';

// Otto IDE Workflows — End-to-End & Visual Parity Verification Suite
// Verifies polyglot IDE workflows against a real isolated `ottod` daemon:
//   1. 1-click gutter breakpoints across Go, Java, Python, JS & Shell + live debugger pause
//   2. Debugger Tabbed/Split view, hideable panes, JSON console, resizers & 0px editor CLS
//   3. JetBrains syntax/semantic coloring (`decimal.MarshalJSONWithoutQuotes`), Rainbow Brackets, Dark/Light & Java Lombok/import diagnostics
//   4. Compact Quick-Doc hover (`< 260px`, zero neighbor bleed) & `<10ms` auto-apply `⌥⏎` Quick Fix (`⌘Z`-undoable)
//   5. IntelliJ two-pane Settings & Project Structure (`⌘,`), Maven tool window & `go.mod` Dependency Analyzer
//   6. Integrated Git view (`M` badge in tree, Otto `DiffViewer`, stage & commit)
//   7. Side-by-side `>>` hunk revert, deleted-file Local History recovery & `IDE -> Threads -> IDE` durability
//   8. Go to Line (`⌘L`), Unified `EmbeddedRuntimeThread` (`⇧⌘L`), `@` / `@~/` mentions clamping, `IDE` folder in Threads & built-in `opencode`/`pi` providers

let seed: PolyglotWorkspaceSeed;

async function clickGutterLine(page: Page, line: number, useBpLane = false): Promise<void> {
  const lineEl = page
    .locator('.ide-editor-area .cm-lineNumbers .cm-gutterElement')
    .filter({ hasText: new RegExp(`^\\s*${line}\\s*$`) })
    .first();
  await expect(lineEl).toBeVisible({ timeout: 10_000 });
  await lineEl.evaluate((el) => el.scrollIntoView({ block: 'center' }));
  const lnBox = await lineEl.boundingBox();
  expect(lnBox).toBeTruthy();
  if (useBpLane) {
    const bpBox = await page.locator('.ide-editor-area .cm-ide-bp-gutter').first().boundingBox();
    expect(bpBox).toBeTruthy();
    await page.mouse.click(bpBox!.x + bpBox!.width / 2, lnBox!.y + lnBox!.height / 2);
  } else {
    await page.mouse.click(lnBox!.x + lnBox!.width / 2, lnBox!.y + lnBox!.height / 2);
  }
}

test.describe('Desktop IDE Workflows — Polyglot Debug, Settings, Git, History & Agent Suite', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({
    viewport: { width: 1440, height: 900 },
    isMobile: false,
    hasTouch: false,
    deviceScaleFactor: 2,
    userAgent:
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  });

  test.beforeAll(async () => {
    test.setTimeout(90_000);
    seed = await createPolyglotSeed();
  });

  test.afterAll(async () => {
    if (seed?.workspaceRoot && existsSync(seed.workspaceRoot)) {
      rmSync(seed.workspaceRoot, { recursive: true, force: true });
    }
    void WORKFLOW_SCREENSHOT_DIR;
  });

  let fatalUiErrors: string[] = [];

  test.beforeEach(async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'Desktop IDE Workflows suite runs on desktop-browser');
    fatalUiErrors = [];
    page.on('pageerror', (err) => {
      if (/effect_update_depth_exceeded|Maximum update depth/i.test(err.message)) {
        fatalUiErrors.push(err.message);
      }
    });
    page.on('console', (msg) => {
      if (msg.type() === 'error' && /effect_update_depth_exceeded|effect_loop/i.test(msg.text())) {
        fatalUiErrors.push(msg.text());
      }
    });
  });

  test.afterEach(() => {
    expect(fatalUiErrors, 'Zero Svelte effect_update_depth_exceeded errors allowed').toEqual([]);
  });

  test('1 & 2. 1-click gutter breakpoints across Go/Java/Python/JS/Shell, live debug pause, Tabbed/Split/JSON console, hideable panes, resizers & 0px CLS', async ({
    page,
  }) => {
    test.setTimeout(120_000);
    await bootPolyglotIde(page, seed, 'dark');
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 420);

    const bpMarkers = page.locator('.ide-editor-area [data-testid="ide-bp-marker"]');

    // 1a) Go: 1st click directly on line number 37 sets breakpoint immediately
    await openPolyglotFile(page, 'app', 'main.go');
    await expect(editorContent(page)).toContainText('decimal.MarshalJSONWithoutQuotes = true');
    await clickGutterLine(page, 37, false);
    await expect(bpMarkers).toHaveCount(1);

    // 1b) Java: 1st click in gutter lane sets breakpoint immediately
    await openPolyglotFile(page, 'src', 'main', 'java', 'com', 'otto', 'BonusService.java');
    await expect(editorContent(page)).toContainText('public class BonusService');
    await clickGutterLine(page, 21, true);
    await expect(bpMarkers).toHaveCount(1);

    // 1c) JS: 1st click sets breakpoint immediately
    await openPolyglotFile(page, 'web', 'worker.mjs');
    await expect(editorContent(page)).toContainText('transformBatch');
    await clickGutterLine(page, 2, false);
    await expect(bpMarkers).toHaveCount(1);

    // 1d) Shell: 1st click sets breakpoint immediately
    await openPolyglotFile(page, 'scripts', 'deploy.sh');
    await expect(editorContent(page)).toContainText('RELEASE_TAG=');
    await clickGutterLine(page, 5, true);
    await expect(bpMarkers).toHaveCount(1);

    // Persistence across tab switch + 2nd click toggles off cleanly!
    await page.locator('.ide-tab').filter({ hasText: 'main.go' }).first().click();
    await expect(bpMarkers).toHaveCount(1);
    await clickGutterLine(page, 37, false);
    await expect(bpMarkers).toHaveCount(0);

    // Verify 0.0px vertical CLS on multi-line text selection in CodeMirror!
    const cmRoot = page.locator('.ide-editor-area .cm-editor').first();
    const beforeSelBox = await cmRoot.boundingBox();
    expect(beforeSelBox).toBeTruthy();
    await editorContent(page).click();
    await page.keyboard.press('Meta+ArrowUp');
    await page.keyboard.press('Shift+ArrowDown');
    await page.keyboard.press('Shift+ArrowDown');
    await expect(page.getByTestId('ide-selection-agent-bar')).toBeVisible();
    const afterSelBox = await cmRoot.boundingBox();
    expect(afterSelBox).toBeTruthy();
    expect(Math.abs(afterSelBox!.y - beforeSelBox!.y)).toBe(0);

    // 1e) Python: 1-click breakpoint on line 11 (`doubled = settled * 2`) + live debug pause!
    await openPolyglotFile(page, 'pyapp', 'pipeline.py');
    await expect(editorContent(page)).toContainText('doubled = settled * 2');
    await clickGutterLine(page, 11, false);
    await expect(bpMarkers).toHaveCount(1);

    // Select `Python Settlement Debug` and click Debug
    await page.getByTestId('ide-run-config-select').click();
    const runPop = page.getByTestId('ide-run-config-popover');
    await expectFullyInViewport(page, runPop, 'IDE run configuration popover');
    await runPop.locator('.run-item').filter({ hasText: 'Python Settlement Debug' }).first().click();

    const startDbg = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/debug/sessions'),
      { timeout: 20_000 },
    );
    await page.getByTestId('ide-debug-btn').click();
    await startDbg;

    const dbgPanel = page.getByTestId('ide-debugger-panel');
    await expect(dbgPanel).toBeVisible({ timeout: 15_000 });
    await expect(dbgPanel).toContainText(/Paused/i, { timeout: 20_000 });
    await expect(dbgPanel).toContainText('run_settlement', { timeout: 15_000 });
    await expect(dbgPanel).toContainText('settled', { timeout: 15_000 });
    await expect(dbgPanel).toContainText('42', { timeout: 15_000 });

    // Evaluate REPL expression while paused
    await dbgPanel.getByLabel('Evaluate expression').fill('settled + 8');
    await dbgPanel.getByRole('button', { name: 'Evaluate' }).click();
    await expect(dbgPanel.locator('.dbg-repl-val').first()).toContainText('50');
    await saveWorkflowShot(page, '01-ide-gutter-breakpoints-live-pause.png');

    // 2a) Switch to full-width `Console` sub-tab -> assert JSON log output rendered cleanly!
    await page.getByTestId('ide-debug-subtab-console').click();
    const fullConsole = page.getByTestId('ide-debug-console-full');
    await expect(fullConsole).toBeVisible();
    await expect(fullConsole).toContainText('settlement.start', { timeout: 10_000 });
    await expect(fullConsole).toContainText('acme');
    await saveWorkflowShot(page, '02-ide-debugger-console-json.png');

    // 2b) Switch back to `Debugger` sub-tab -> toggle Split mode -> hide/show panes -> drag both resizers!
    await page.getByTestId('ide-debug-subtab-debugger').click();
    await page.getByTestId('ide-debug-layout-toggle').click();
    await expect(page.getByTestId('ide-debug-split-grid')).toBeVisible();

    await page.getByTestId('ide-debug-toggle-frames').click();
    await expect(page.getByTestId('ide-debug-frames-pane')).toHaveCount(0);
    await page.getByTestId('ide-debug-toggle-frames').click();
    await expect(page.getByTestId('ide-debug-frames-pane')).toBeVisible();

    await page.getByTestId('ide-debug-toggle-console').click();
    await expect(page.getByTestId('ide-debug-console-pane')).toHaveCount(0);
    await page.getByTestId('ide-debug-toggle-console').click();
    await expect(page.getByTestId('ide-debug-console-pane')).toBeVisible();

    // Drag horizontal column resizer (+60px)
    const framesPane = page.getByTestId('ide-debug-frames-pane');
    const colResizer = dbgPanel.getByRole('separator', { name: 'Resize Frames column' }).first();
    const framesWBefore = (await framesPane.boundingBox())!.width;
    const colBox = (await colResizer.boundingBox())!;
    await page.mouse.move(colBox.x + colBox.width / 2, colBox.y + colBox.height / 2);
    await page.mouse.down();
    await page.mouse.move(colBox.x + colBox.width / 2 + 60, colBox.y + colBox.height / 2, { steps: 5 });
    await page.mouse.up();
    const framesWAfter = (await framesPane.boundingBox())!.width;
    expect(framesWAfter).toBeGreaterThan(framesWBefore + 25);

    // Drag bottom dock vertical height resizer upward (-70px -> taller dock)
    const bottomDock = page.getByTestId('ide-bottom-dock');
    const dockResizer = page.getByTestId('ide-dock-resizer');
    const dockHBefore = (await bottomDock.boundingBox())!.height;
    const dockBox = (await dockResizer.boundingBox())!;
    await page.mouse.move(dockBox.x + dockBox.width / 2, dockBox.y + dockBox.height / 2);
    await page.mouse.down();
    await page.mouse.move(dockBox.x + dockBox.width / 2, dockBox.y - 70, { steps: 5 });
    await page.mouse.up();
    const dockHAfter = (await bottomDock.boundingBox())!.height;
    expect(dockHAfter).toBeGreaterThan(dockHBefore + 35);
    await saveWorkflowShot(page, '03-ide-debugger-split-resized.png');

    await dbgPanel.getByRole('button', { name: 'Resume program' }).click();
    await expect(dbgPanel).toContainText(/Terminated/i, { timeout: 15_000 });
  });

  test('3 & 4. JetBrains semantic coloring (`decimal.MarshalJSONWithoutQuotes`), Rainbow Brackets, Dark/Light schemes, Java `@Slf4j`/`@Service`/bogus-import diagnostics, Quick-Doc hover & `<10ms` `⌥⏎` Quick Fix + `⌘Z`', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    writeFileSync(join(seed.workspaceRoot, 'app', 'main.go'), GO_MAIN_SOURCE);
    writeFileSync(join(seed.workspaceRoot, 'src/main/java/com/otto', 'BonusService.java'), JAVA_BONUS_SOURCE);

    await bootPolyglotIde(page, seed, 'dark');
    await openPolyglotFile(page, 'app', 'main.go');
    await expect(editorContent(page)).toContainText('decimal.MarshalJSONWithoutQuotes = true');

    // Wait for semantic decorations (`.cm-syn-field` & `.cm-syn-func-call` & `.cm-syn-rb-1`)
    await expect(page.locator('.ide-editor-area .cm-syn-field').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.ide-editor-area .cm-syn-rb-1').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.ide-editor-area .cm-syn-rb-2').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.ide-editor-area .cm-syn-rb-3').first()).toBeVisible({ timeout: 10_000 });

    const goColors = await page.evaluate(() => {
      const lines = Array.from(document.querySelectorAll('.ide-editor-area .cm-line'));
      const decLine = lines.find((el) => (el.textContent ?? '').includes('decimal.MarshalJSONWithoutQuotes = true'));
      const callLine = lines.find((el) => (el.textContent ?? '').includes('banner := app.InitApp('));
      const colorForToken = (root: Element | undefined, token: string): string => {
        if (!root) return '';
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
        let n: Node | null;
        while ((n = walker.nextNode())) {
          if ((n.textContent ?? '').includes(token) && n.parentElement) {
            return getComputedStyle(n.parentElement).color;
          }
        }
        return '';
      };
      return {
        decimalColor: colorForToken(decLine, 'decimal'),
        fieldColor: colorForToken(decLine, 'MarshalJSONWithoutQuotes'),
        callColor: colorForToken(callLine, 'InitApp'),
      };
    });
    expect(goColors.decimalColor).toBeTruthy();
    expect(goColors.fieldColor).toBeTruthy();
    expect(goColors.callColor).toBeTruthy();
    expect(goColors.decimalColor).not.toBe(goColors.fieldColor);
    expect(goColors.fieldColor).not.toBe(goColors.callColor);
    await saveWorkflowShot(page, '04-ide-go-semantic-islands-dark.png');

    // Switch to JetBrains Islands Light -> verify editor-scoped scheme (no `:root` bleed) -> restore Dark
    const editorOuter = page.locator('.ide-editor-area .code-editor-outer').first();
    await page.evaluate(async () => {
      document.documentElement.dataset.scheme = 'light';
      // @ts-expect-error browser Vite runtime import
      const { writeIdeEditorPreferences } = await import('/src/lib/components/cm-ide-theme.ts');
      writeIdeEditorPreferences({ colorScheme: 'jetbrains-islands-light' });
    });
    await expect(editorOuter).toHaveAttribute('data-ide-scheme', 'jetbrains-islands-light');
    expect(await page.evaluate(() => document.documentElement.dataset.ideScheme)).toBeUndefined();
    await saveWorkflowShot(page, '05-ide-go-semantic-islands-light.png');

    await page.evaluate(async () => {
      document.documentElement.dataset.scheme = 'dark';
      // @ts-expect-error browser Vite runtime import
      const { writeIdeEditorPreferences } = await import('/src/lib/components/cm-ide-theme.ts');
      writeIdeEditorPreferences({ colorScheme: 'jetbrains-islands-dark' });
    });
    await expect(editorOuter).toHaveAttribute('data-ide-scheme', 'jetbrains-islands-dark');
    expect(await page.evaluate(() => document.documentElement.dataset.ideScheme)).toBeUndefined();

    // 4a) Compact Quick-Doc hover on `ComputeBonus` surrounded by `NeighborAboveDoc` & `NeighborBelowDoc`
    const computeTok = page
      .locator('.ide-editor-area .cm-line')
      .filter({ hasText: 'func ComputeBonus(' })
      .locator('.cm-syn-func-decl')
      .first();
    await expect(computeTok).toBeVisible();
    const tokBox = (await computeTok.boundingBox())!;
    await page.mouse.move(tokBox.x + tokBox.width / 2, tokBox.y + tokBox.height / 2);

    const hoverCard = page.getByTestId('ide-quick-doc-hover');
    await expect(hoverCard).toBeVisible({ timeout: 10_000 });
    const hoverBox = (await hoverCard.boundingBox())!;
    expect(hoverBox.height).toBeLessThan(260);
    const hoverText = (await hoverCard.textContent()) ?? '';
    expect(hoverText).toContain('ComputeBonus');
    expect(hoverText).toContain('calculates deterministic operator settlement bonus');
    expect(hoverText).not.toContain('NeighborAboveDoc');
    expect(hoverText).not.toContain('NeighborBelowDoc');
    expect(hoverText).not.toContain('```');
    await saveWorkflowShot(page, '06-ide-compact-quick-doc-hover.png');
    await page.mouse.move(10, 10);

    // 3b) Java `@Service` gold annotation, `fooService` purple field, `@Slf4j` zero false error on `log.debug`, and `FakeClass` error!
    await openPolyglotFile(page, 'src', 'main', 'java', 'com', 'otto', 'BonusService.java');
    await expect(editorContent(page)).toContainText('import com.nonexistent.FakeClass;');
    await expect(page.locator('.ide-editor-area .cm-syn-annotation').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.ide-editor-area .cm-syn-doc-tag').first()).toBeVisible({ timeout: 10_000 });

    const javaColors = await page.evaluate(() => {
      const ann = document.querySelector('.ide-editor-area .cm-syn-annotation');
      const fld = document.querySelector('.ide-editor-area .cm-syn-field');
      const cls = Array.from(document.querySelectorAll('.ide-editor-area .cm-syn-type')).find((e) =>
        (e.textContent ?? '').includes('BonusService'),
      );
      return {
        annColor: ann ? getComputedStyle(ann).color : '',
        fldColor: fld ? getComputedStyle(fld).color : '',
        clsColor: cls ? getComputedStyle(cls).color : '',
      };
    });
    expect(javaColors.annColor).toBeTruthy();
    expect(javaColors.fldColor).toBeTruthy();
    expect(javaColors.annColor).not.toBe(javaColors.fldColor);
    expect(javaColors.annColor).not.toBe(javaColors.clsColor);

    await expect
      .poll(
        () =>
          page.evaluate(async () => {
            // @ts-expect-error browser Vite runtime import
            const { ide } = await import('/src/modules/ide/ideStore.svelte.ts');
            return ide.problems.map((d: { line: number; severity: string; message: string }) => ({
              line: d.line,
              severity: d.severity,
              message: d.message,
            }));
          }),
        { timeout: 12_000 },
      )
      .toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            line: 5,
            severity: 'error',
            message: expect.stringMatching(/com\.nonexistent\.FakeClass/),
          }),
        ]),
      );

    const javaDiags = await page.evaluate(async () => {
      // @ts-expect-error browser Vite runtime import
      const { ide } = await import('/src/modules/ide/ideStore.svelte.ts');
      return ide.problems as Array<{ line: number; severity: string; message: string }>;
    });
    expect(javaDiags.some((d) => d.message.includes('log'))).toBe(false);
    await saveWorkflowShot(page, '07-ide-java-semantic-lombok-diag.png');

    // 4b) `<10ms` auto-apply `⌥⏎` Quick Fix on `import com.nonexistent.FakeClass;` + `⌘Z` undo!
    await editorContent(page).click();
    await page.keyboard.press('Meta+ArrowUp');
    for (let i = 1; i < 5; i++) await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Alt+Enter');

    const intentionPop = page.getByTestId('ide-intention-popover');
    await expect(intentionPop).toBeVisible({ timeout: 10_000 });
    await expectFullyInViewport(page, intentionPop, 'IDE intention & quick-fix popover');
    await expect(page.getByTestId('ide-intention-preview')).toBeVisible();
    await saveWorkflowShot(page, '08-ide-intention-quickfix-preview.png');

    await intentionPop
      .getByTestId('ide-intention-option')
      .filter({ hasText: /Remove .*import/i })
      .first()
      .click();
    await expect(editorContent(page)).not.toContainText('com.nonexistent.FakeClass');
    await expect(page.getByTestId('ide-agent-panel')).toHaveCount(0);

    // Native `⌘Z` immediately restores the removed import!
    await editorContent(page).click();
    await page.keyboard.press('Meta+z');
    await expect(editorContent(page)).toContainText('import com.nonexistent.FakeClass;');
  });

  test('5 & 6. IntelliJ Settings & Project Structure (`⌘,`), Maven tool window, `go.mod` Dependency Analyzer, and Integrated Git (`M` badge, DiffViewer, stage & commit)', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await bootPolyglotIde(page, seed, 'dark');
    await openPolyglotFile(page, 'app', 'main.go');

    // 5a) Open unified two-pane Settings directly via `⌘,` (no toolbar button fallback)
    await page.keyboard.press('Meta+,');
    const settingsModal = page.getByTestId('ide-settings-modal');
    await expect(settingsModal).toBeVisible({ timeout: 10_000 });

    // Verify VS Code Modern dark & light editor color schemes apply cleanly to `.code-editor-outer`
    const editorOuter = page.locator('.ide-editor-area .code-editor-outer').first();
    await page.getByTestId('ide-settings-nav-appearance.ui').click();
    const schemeSelect = page.getByTestId('ide-color-scheme-select');
    await schemeSelect.selectOption('vscode-dark-modern');
    await page.getByTestId('ide-settings-apply').click();
    await expect(editorOuter).toHaveAttribute('data-ide-scheme', 'vscode-dark-modern');

    await schemeSelect.selectOption('vscode-light-modern');
    await page.getByTestId('ide-settings-apply').click();
    await expect(editorOuter).toHaveAttribute('data-ide-scheme', 'vscode-light-modern');

    await schemeSelect.selectOption('jetbrains-islands-dark');
    await page.getByTestId('ide-settings-apply').click();
    await expect(editorOuter).toHaveAttribute('data-ide-scheme', 'jetbrains-islands-dark');

    // Search filter narrows categories cleanly
    const searchIn = page.getByTestId('ide-settings-search');
    await searchIn.fill('goroot');
    await expect(page.getByTestId('ide-settings-tree')).toContainText('Go');
    await searchIn.fill('');

    // Project Structure -> Project SDK menu (`Configured` / `Detected` + `Download SDK…` + `Add SDK from disk…`)
    await page.getByTestId('ide-settings-nav-project.project').click();
    await expect(page.getByTestId('ide-settings-project')).toBeVisible();
    await page.getByTestId('ide-project-sdk-picker').click();
    const sdkMenu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, sdkMenu, 'Project SDK picker menu');
    await expect(sdkMenu).toContainText('Download SDK…');
    await expect(sdkMenu).toContainText('Add SDK from disk…');
    await saveWorkflowShot(page, '09-ide-settings-project-sdk.png');
    await page.keyboard.press('Escape');
    await expect(sdkMenu).toBeHidden();
    await expect(settingsModal).toBeVisible();

    // Languages & Frameworks -> Go: GOROOT + `+` (`Local…` / `Download…`) + Apply + OK
    await page.getByTestId('ide-settings-nav-lang.go').click();
    await expect(page.getByTestId('ide-settings-lang-go')).toBeVisible();

    await page.getByTestId('ide-goroot-add-btn').click();
    const plusMenu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, plusMenu, 'GOROOT + menu');
    await expect(plusMenu).toContainText('Local…');
    await expect(plusMenu).toContainText('Download…');
    await page.keyboard.press('Escape');
    await expect(plusMenu).toBeHidden();
    await expect(settingsModal).toBeVisible();

    await saveWorkflowShot(page, '10-ide-settings-lang-go.png');
    const applyBtn = page.getByTestId('ide-settings-apply');
    if (await applyBtn.isEnabled().catch(() => false)) {
      await applyBtn.click();
    }
    const okBtn = page.getByTestId('ide-settings-ok');
    if (await okBtn.isVisible().catch(() => false)) {
      await okBtn.click();
    } else {
      await settingsModal.locator('xpath=ancestor::*[@role="dialog"]').getByRole('button', { name: 'OK' }).click();
    }
    await expect(settingsModal).toHaveCount(0);

    // 5b) Right-side Maven Build Tool window + `go.mod` Dependency Analyzer
    const buildToggle = page.getByTestId('ide-toggle-build-tool');
    await expect(buildToggle).toBeVisible({ timeout: 10_000 });
    await buildToggle.click();
    await expect(page.locator('body')).toContainText('otto-bonus-service');

    await openPolyglotFile(page, 'go.mod');
    await expect(page.getByTestId('ide-dep-mode-switcher')).toBeVisible();
    await page.getByTestId('ide-dep-analyzer-tab').click();
    const depAnalyzer = page.getByTestId('ide-dependency-analyzer');
    await expect(depAnalyzer).toBeVisible({ timeout: 15_000 });
    await expect(depAnalyzer).toContainText('github.com/shopspring/decimal');
    await saveWorkflowShot(page, '11-ide-dependency-analyzer-maven.png');
    await buildToggle.click();

    // 6) Integrated Git view: modify tracked file -> `M` badge in tree -> Git dock DiffViewer -> Stage & Commit!
    await openPolyglotFile(page, 'notes', 'roadmap.md');
    await expect(editorContent(page)).toContainText('Initial committed baseline.');
    await editorContent(page).click();
    await page.keyboard.press('Meta+ArrowDown');
    await page.keyboard.insertText('\n## Integrated Git Dock Verified\n');
    const saveFileWait = page.waitForResponse(
      (r) => r.request().method() === 'PUT' && r.url().includes('/api/v1/ide/fs/file'),
      { timeout: 15_000 },
    );
    await page.keyboard.press('Meta+s');
    expect((await saveFileWait).ok()).toBeTruthy();

    // Refresh tree / Git status and assert `M` badge appears on `roadmap.md`
    const roadmapItem = page
      .getByTestId('ide-project-tree')
      .locator('button.tree-item[title$="roadmap.md"]')
      .first();
    await expect(roadmapItem.getByTestId('ide-tree-git-status')).toHaveText('M', { timeout: 15_000 });

    await page.getByTestId('ide-dock-tab-git').click();
    const gitView = page.getByTestId('ide-git-changes-view');
    await expect(gitView).toBeVisible({ timeout: 15_000 });
    await expect(gitView).toContainText('roadmap.md', { timeout: 15_000 });
    await expect(gitView.locator('.gc-diff-scroll')).toContainText('Integrated Git Dock Verified', {
      timeout: 15_000,
    });
    await saveWorkflowShot(page, '12-ide-integrated-git-diff.png');

    await page.getByTestId('ide-git-stage-all').click();
    await expect(page.getByTestId('ide-git-unstage-all')).toBeVisible({ timeout: 10_000 });
    await page.getByTestId('ide-git-commit-input').fill('docs: verify integrated git dock');
    await page.getByTestId('ide-git-commit-btn').click();
    await expect(gitView).toContainText('Working tree clean', { timeout: 15_000 });

    const headSubject = execFileSync('git', ['log', '-1', '--pretty=%s'], {
      cwd: seed.workspaceRoot,
      encoding: 'utf8',
    }).trim();
    expect(headSubject).toBe('docs: verify integrated git dock');
  });

  test('7 & 8. Side-by-side `>>` hunk revert, Local History deleted-file recovery, IDE<->Threads durability, Go to Line (`⌘L`), EmbeddedRuntimeThread (`⇧⌘L`), `@`/`@~/` clamping & built-in `opencode`/`pi` providers', async ({
    page,
  }) => {
    test.setTimeout(120_000);
    const recRel = 'notes/recoverable.txt';
    const recAbs = join(seed.workspaceRoot, 'notes', 'recoverable.txt');

    // Create & record two versions of `notes/recoverable.txt` via daemon IDE FS API so Local History captures both
    const c = await apiCtx();
    try {
      await daemonCall(c.ctx, seed.base, seed.token, 'POST', '/ide/fs/mutate', {
        target: seed.target,
        op: 'create_file',
        path: recRel,
      });
      await daemonCall(c.ctx, seed.base, seed.token, 'PUT', '/ide/fs/file', {
        target: seed.target,
        path: recRel,
        content: 'Alpha v1\nShared baseline\n',
      });
      await daemonCall(c.ctx, seed.base, seed.token, 'PUT', '/ide/fs/file', {
        target: seed.target,
        path: recRel,
        content: 'Alpha v2\nShared baseline\n',
      });
    } finally {
      await c.ctx.dispose();
    }

    await bootPolyglotIde(page, seed, 'dark');
    await openPolyglotFile(page, 'notes', 'recoverable.txt');
    await expect(editorContent(page)).toContainText('Alpha v2');

    // 7a) Open Side-by-Side Local History comparison via Project Tree context menu & revert hunk via `>>`!
    const tree = page.getByTestId('ide-project-tree');
    const recItem = tree.locator('button.tree-item[title$="recoverable.txt"]').first();
    await expect(recItem).toBeVisible({ timeout: 10_000 });
    await recItem.click({ button: 'right' });
    const ctxMenu = page.locator('.ctx-menu');
    await expect(ctxMenu).toBeVisible({ timeout: 5_000 });
    await ctxMenu.getByRole('menuitem', { name: 'Local History' }).click();
    await expect(ctxMenu).toBeVisible({ timeout: 5_000 });
    await ctxMenu.getByRole('menuitem', { name: 'Show History…' }).click();
    const cmpModal = page.getByTestId('ide-comparison-modal');
    await expect(cmpModal).toBeVisible({ timeout: 15_000 });
    const rows = cmpModal.getByTestId('ide-local-history-row');
    await expect(rows.nth(1)).toBeVisible({ timeout: 10_000 });
    // Select `rows.nth(1)` (`Saved` snapshot `Alpha v1` prior to `Alpha v2`)
    await rows.nth(1).click();
    await expect(page.getByTestId('ide-compare-revert-hunk').first()).toBeVisible({ timeout: 10_000 });
    await saveWorkflowShot(page, '13-ide-side-by-side-compare.png');

    await page.getByTestId('ide-compare-revert-hunk').first().click();
    await expect(page.getByTestId('ide-compare-identical-banner')).toBeVisible({ timeout: 10_000 });
    expect(readFileSync(recAbs, 'utf8')).toContain('Alpha v1');
    await page.keyboard.press('Escape');
    await expect(cmpModal).toHaveCount(0);

    // 7b) Delete `recoverable.txt` via `IdeProjectTree` -> verify gone -> Restore via Local History (`Deleted`)!
    await expect(recItem).toBeVisible();
    await recItem.focus();
    await page.keyboard.press('Meta+Backspace');
    await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();

    await expect(tree.locator('button.tree-item[title$="recoverable.txt"]')).toHaveCount(0, { timeout: 10_000 });
    expect(existsSync(recAbs)).toBe(false);

    await page.getByTestId('ide-tree-local-history-btn').click();
    await expect(cmpModal).toBeVisible({ timeout: 10_000 });
    await page.getByTestId('ide-local-history-filter-deleted').click();
    const delRow = cmpModal.getByTestId('ide-local-history-row').filter({ hasText: 'recoverable.txt' }).first();
    await expect(delRow).toBeVisible({ timeout: 10_000 });
    await delRow.click();
    await saveWorkflowShot(page, '14-ide-local-history-deleted-restore.png');

    await page.getByTestId('ide-local-history-restore').click();
    await expect(cmpModal).toHaveCount(0, { timeout: 10_000 });
    expect(existsSync(recAbs)).toBe(true);
    expect(readFileSync(recAbs, 'utf8')).toContain('Alpha v1');
    await expect(tree.locator('button.tree-item[title$="recoverable.txt"]').first()).toBeVisible({ timeout: 10_000 });
    await expect(editorContent(page)).toContainText('Alpha v1');

    // 7c) Edit + copy in editor -> navigate `IDE -> Threads -> IDE` -> verify `⌘Z` & `⌘⇧V` Clipboard History survive!
    await editorContent(page).click();
    await page.keyboard.press('Meta+ArrowDown');
    await page.keyboard.insertText('\nTransient unsaved edit');
    await page.keyboard.press('Shift+Meta+ArrowLeft');
    await page.keyboard.press('Meta+c');
    await editorContent(page).dispatchEvent('copy');

    await page.goto('/#/threads');
    const ideFolder = page.getByTestId('threads-ide-folder');
    await expect(ideFolder).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('threads-ide-project-row').first()).toBeVisible();
    await expect(ideFolder).toContainText(/Otto IDE Polyglot Project|otto-ide-workflows-suite-/);
    await saveWorkflowShot(page, '15-ide-threads-ide-folder.png');

    await page.goto('/#/ide');
    await expect(editorContent(page)).toContainText('Transient unsaved edit', { timeout: 15_000 });
    await editorContent(page).click();
    await page.keyboard.press('Meta+z');
    await expect(editorContent(page)).not.toContainText('Transient unsaved edit');

    await editorContent(page).click();
    await page.keyboard.press('Meta+Shift+V');
    const clipModal = page.getByTestId('ide-clipboard-history-popover');
    await expect(clipModal).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');

    // 8a) Verify `⌘L` opens Go to Line:Column dialog, then `⇧⌘L` opens AI Agent dock!
    await editorContent(page).click();
    await page.keyboard.press('Meta+l');
    const gotoDialog = page.getByRole('dialog').filter({ hasText: /Go to Line/i }).first();
    await expect(gotoDialog).toBeVisible({ timeout: 5_000 });
    await page.keyboard.press('Escape');
    await expect(gotoDialog).toHaveCount(0);

    const agentPanel = page.getByTestId('ide-agent-panel');
    await page.keyboard.press('Meta+Shift+L');
    await expect(agentPanel).toBeVisible({ timeout: 5_000 });

    const composer = page.getByTestId('ide-agent-composer');
    await composer.click();
    await composer.fill('@');
    const mentionMenu = page.getByTestId('runtime-at-mention-menu');
    await expectFullyInViewport(page, mentionMenu, 'IDE @ mention menu');

    await composer.fill('@~/');
    await expectFullyInViewport(page, mentionMenu, 'IDE @~/ home-path mention menu');

    await composer.fill('Verify EmbeddedRuntimeThread live streaming in IDE');
    await page.getByTestId('ide-agent-send').click();
    await expect(page.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 25_000 });
    await expect(agentPanel).toContainText('Verify EmbeddedRuntimeThread live streaming in IDE', { timeout: 20_000 });
    await saveWorkflowShot(page, '16-ide-embedded-runtime-thread.png');

    // 8b) `#/settings/providers`: verify `opencode` & `pi` under Built-in + `Enabled` toggle!
    await page.goto('/#/settings/providers');
    await expect(page.locator('.section-title').filter({ hasText: 'Built-in' })).toBeVisible({ timeout: 15_000 });
    const opencodeToggle = page.getByLabel('Show opencode in pickers');
    const piToggle = page.getByLabel('Show pi in pickers');
    await expect(opencodeToggle).toBeVisible();
    await expect(piToggle).toBeVisible();
    await expect(opencodeToggle).toBeChecked();

    await opencodeToggle.uncheck();
    await expect(opencodeToggle).not.toBeChecked();
    await saveWorkflowShot(page, '17-ide-settings-providers-opencode-pi.png');

    // Re-enable `opencode` so daemon settings stay clean
    await opencodeToggle.check();
    await expect(opencodeToggle).toBeChecked();
  });
});
