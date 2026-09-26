import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 900 } });
let workspaceId = '', vaultId = 0;
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  vaultId = (await seedVaultDir(ctx, base, workspaceId)).vaultId;
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ w, v }) => {
    localStorage.setItem('otto_workspace', w);
    localStorage.setItem('otto_vault_last', String(v));
  }, { w: workspaceId, v: vaultId });
});
async function artifact(page: Page) {
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/design/artifacts`, { data: {
    workspace_id: workspaceId, title: 'R4 synthetic release checklist', format: 'html', studio: 'frames',
    content: '<body style="font:16px system-ui;padding:24px"><h1>Release checklist</h1><p>Review each change before publication.</p><button>Review draft</button></body>',
  } });
  expect(r.ok()).toBeTruthy();
  const id = (await r.json()).artifact.id;
  await ctx.dispose();
  await page.goto(`/#/design/a/${id}`);
  await expect(page.getByTestId('design-source-toggle')).toBeVisible();
}
for (const [theme, scheme, width, rtl] of [
  ['native', 'light', 1440, false], ['native', 'dark', 375, false],
  ['warm', 'light', 834, true], ['warm', 'dark', 375, false], ['pro-dark', 'dark', 1440, true],
] as const) {
  test(`D2 real renderer and source recovery ${theme} ${scheme}`, async ({ page }, testInfo) => {
    test.setTimeout(90_000);
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(({ theme, scheme, rtl }) => {
      localStorage.setItem('otto_theme', theme); localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_direction', rtl ? 'rtl' : 'ltr');
    }, { theme, scheme, rtl });
    const { ctx, base } = await apiCtx();
    const title = `D2 ${theme} ${scheme} ${Date.now()}`;
    const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`, { data: {
      title, doc: { type: 'otto-canvas', version: 1, format: 'd2', source: 'direction: right\nReview -> Publish' },
    } });
    expect(r.ok()).toBeTruthy(); const id = (await r.json()).id;
    await openPage(page, 'canvas');
    await page.locator('.scene-list .row', { hasText: title }).getByRole('button').first().click();
    await expect(page.locator('.board .content > svg')).toBeVisible({ timeout: 60_000 });
    if (width >= 640) {
    await page.getByTitle('Edit the D2 source', { exact: true }).click();
    await page.screenshot({ path: `/tmp/otto-ux-r4-content-d2-${theme}-${scheme}-${testInfo.project.name}.png` });
    expect((await page.locator('.board .code-pane').boundingBox())!.width).toBeGreaterThan(280);
    await page.locator('.cm-content').fill('direction: right\nReview -> Approved');
    await expect(page.locator('.board .content')).toContainText('Approved', { timeout: 30_000 });
    await page.locator('.cm-content').fill('broken: {');
    await expect(page.locator('.board .err')).toContainText('Diagram error', { timeout: 30_000 });
    await expect(page.locator('.board .content')).toContainText('Approved');
    await page.locator('.cm-content').fill('direction: right\nReview -> Recovered');
    await expect(page.locator('.board .err')).toHaveCount(0);
    await expect.poll(async () => JSON.parse((await (await ctx.get(`${base}/api/v1/canvas/scenes/${id}`)).json()).doc_json).source).toContain('Recovered');
    await page.getByTitle('Edit the D2 source', { exact: true }).click();
    }
    await ctx.dispose(); await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-ux-r4-content-d2-preview-${theme}-${scheme}-${testInfo.project.name}.png` });
    const ai = (await page.locator('.ai-bar').boundingBox())!, zoom = (await page.locator('.zoombar').boundingBox())!;
    expect(ai.x + ai.width <= zoom.x || zoom.x + zoom.width <= ai.x || ai.y + ai.height <= zoom.y || zoom.y + zoom.height <= ai.y, 'Ask AI must not obscure diagram controls').toBe(true);

  });
  test(`Design Hall responsive editing ${theme} ${scheme}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(({ theme, scheme, rtl }) => {
      if (window.top !== window) return;
      localStorage.setItem('otto_theme', theme); localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_direction', rtl ? 'rtl' : 'ltr');
    }, { theme, scheme, rtl });
    await artifact(page);
    if (width < 640) await page.getByRole('group', { name: 'Device frame' }).getByRole('button', { name: 'iPhone', exact: true }).click();
    await page.screenshot({ path: `/tmp/otto-ux-r4-content-design-${theme}-${scheme}-${testInfo.project.name}.png` });
    if (width < 900) {
      expect((await page.locator('.stage-host').boundingBox())!.height).toBeGreaterThan(450);
      await page.getByRole('button', { name: 'Show design details', exact: true }).click();
      await expect(page.getByRole('tablist', { name: 'Details panel' })).toBeVisible();
      await page.getByTestId('design-tab-links').click();
      await page.getByRole('button', { name: 'Back to design', exact: true }).click();
    }
    if (width < 640) {
      const notch = (await page.locator('.device.iphone .notch').boundingBox())!;
      const screen = (await page.locator('.device.iphone iframe').boundingBox())!;
      expect(notch.y + notch.height, 'Decorative notch must not cover preview content').toBeLessThanOrEqual(screen.y);
    }
    await page.getByTestId('design-source-toggle').click();
    await page.locator('.cm-content').fill('<h1>R4 saved responsive draft</h1>');
    await page.getByTestId('design-save').click();
    await expect(page.getByTestId('design-version-chip')).toHaveCount(2);
    await page.getByTestId('design-source-toggle').click();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-ux-r4-content-design-${theme}-${scheme}-${testInfo.project.name}.png` });
  });
}
test('Snip creates edits selects moves and removes annotations using only the keyboard', async ({ page }) => {
  await page.goto('/');
  const png = await page.evaluate(() => {
    const c = document.createElement('canvas'); c.width = 640; c.height = 480;
    return c.toDataURL().split(',')[1];
  });
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/snips`, { data: { data_b64: png, filename: 'r4-keyboard.png' } });
  const id = (await r.json()).id; await ctx.dispose();
  const copies: string[] = [];
  await page.context().route(`**/snips/${id}/annotated`, route => {
    copies.push(route.request().postDataJSON().data_b64); return route.fulfill({ json: { copied: true } });
  });
  await page.goto(`/#/snip/${id}`);
  const canvas = page.locator('.snip-canvas'); await expect(canvas).toBeVisible();
  await page.getByRole('button', { name: 'Copy', exact: true }).focus();
  await page.keyboard.press('Tab'); await expect(canvas).toBeFocused();
  await page.keyboard.press('t'); await page.keyboard.press('Enter');
  await page.getByRole('textbox', { name: 'Annotation text' }).fill('Keyboard annotation');
  await page.keyboard.press('Control+Enter'); await expect(canvas).toBeFocused();
  await expect(page.locator('.snip-editor')).toHaveAttribute('data-count', '1');
  await page.keyboard.press('r'); await page.keyboard.press('Enter');
  await expect(page.locator('.snip-editor')).toHaveAttribute('data-count', '2');
  await expect.poll(() => copies.length).toBeGreaterThan(0); const before = copies.at(-1);
  await page.keyboard.press('Shift+ArrowRight');
  await expect.poll(() => copies.at(-1)).not.toBe(before);
  await page.keyboard.press('['); await page.keyboard.press('Enter');
  await expect(page.getByRole('textbox', { name: 'Annotation text' })).toHaveValue('Keyboard annotation');
  await page.keyboard.press('Escape'); await expect(canvas).toBeFocused();
  await page.keyboard.press('Delete'); await expect(page.locator('.snip-editor')).toHaveAttribute('data-count', '1');
  await page.keyboard.press('Control+z'); await expect(page.locator('.snip-editor')).toHaveAttribute('data-count', '2');
  await page.screenshot({ path: '/tmp/otto-ux-r4-content-snip-keyboard.png' });
});

test('Compact Design Hall gives the preview the available height', async ({ page }) => {
  await page.setViewportSize({ width: 834, height: 900 });
  await artifact(page);
  await page.screenshot({ path: '/tmp/otto-ux-r4-content-design-before.png' });
  expect((await page.locator('.stage-host').boundingBox())!.height).toBeGreaterThan(450);
});
test('Product rejected autosave retains both artifacts through switching and resize', async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on('pageerror', error => {
    if (error.name === 'SecurityError' && /sandbox|web-inspector/.test(error.stack ?? error.message)) return;
    errors.push(error.stack ?? error.message);
  });
  const { ctx, base } = await apiCtx();
  const title = `R4 two artifacts ${Date.now()}`;
  const draft = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: { title } });
  const sid = (await draft.json()).story.id;
  const ids: string[] = [];
  for (const filename of ['alpha.html', 'beta.html']) {
    const r = await ctx.post(`${base}/api/v1/product/stories/${sid}/attachments`, { data: {
      filename, mime: 'text/html', kind: 'mockup', data_b64: Buffer.from(`<h1>${filename}</h1>`).toString('base64'),
    } });
    expect(r.ok()).toBeTruthy(); ids.push((await r.json()).id);
  }
  let fail = true;
  await page.context().route(`**/product/attachments/${ids[0]}/content`, route => fail
    ? route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic autosave unavailable' } }) : route.continue());
  await openPage(page, 'product');
  await page.locator('.story-row', { hasText: title }).click();
  await page.getByRole('tab', { name: 'Story', exact: true }).click();
  await page.locator('.sub-tab-strip .st', { hasText: 'Design' }).click();
  const select = async (name: string) => {
    const panes = page.getByRole('tablist', { name: 'Design panes' });
    if (await panes.isVisible()) await panes.getByRole('tab', { name: 'Assets', exact: true }).click();
    await page.locator('.mockup-row', { hasText: name }).click();
    await page.locator('.stage-toolbar .tb-btn', { hasText: 'Source' }).click();
    await expect(page.locator('.code-view')).toBeVisible();
  };
  await select('alpha.html');
  await page.locator('.code-view').fill('<h1>Retained Alpha draft</h1>');
  await expect(page.locator('.stage-status')).toContainText('save failed');
  expect(errors, 'Rejected autosave must reach its handled UI error path').toEqual([]);
  await page.evaluate(async ({ id, sid }) => {
    const path = '/src/lib/stores/mockup-assist.svelte.ts';
    const { mockupAssist } = await import(path);
    mockupAssist.lastUpdate = { attachmentId: id, storyId: sid, format: 'html', content: null, tick: Date.now() };
  }, { id: ids[0], sid });
  await expect(page.getByRole('dialog')).toContainText('Newer version available');
  await page.keyboard.press('Escape');
  await expect(page.locator('.code-view')).toHaveValue('<h1>Retained Alpha draft</h1>');
  await select('beta.html'); await page.locator('.code-view').fill('<h1>Saved Beta draft</h1>');
  await page.setViewportSize({ width: 375, height: 900 });
  await select('alpha.html');
  await expect(page.locator('.code-view')).toHaveValue('<h1>Retained Alpha draft</h1>');
  fail = false;
  await page.locator('.code-view').fill('<h1>Recovered Alpha draft</h1>');
  await expect(page.locator('.stage-status')).toContainText('saved');
  await expect.poll(async () => (await ctx.get(`${base}/api/v1/product/attachments/${ids[0]}`)).text()).toContain('Recovered Alpha');
  await expect.poll(async () => (await ctx.get(`${base}/api/v1/product/attachments/${ids[1]}`)).text()).toContain('Saved Beta');
  await ctx.dispose(); await page.reload();
  expect(errors).toEqual([]);
});

test('Vault long phone tree leaves useful space for reading and editing', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 900 });
  const { ctx, base } = await apiCtx();
  for (let i = 0; i < 16; i++) {
    const r = await ctx.put(`${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/note`, { data: {
      path: `long-tree/note-${String(i).padStart(2, '0')}.md`, if_hash: '',
      content: `# Note ${i}\n\n${'Readable long documentation with a clear next step.\n\n'.repeat(15)}`,
    } }); expect(r.ok()).toBeTruthy();
  }
  await ctx.dispose(); await openPage(page, 'vault');
  await page.locator('.tree').getByText('long-tree', { exact: true }).click();
  await page.locator('.tree').getByText('note-15', { exact: true }).click();
  await expect(page.locator('.note-view')).toContainText('Note 15');
  await page.screenshot({ path: '/tmp/otto-ux-r4-content-vault-long.png' });
  expect((await page.locator('.vault-page .left').boundingBox())!.height).toBeLessThanOrEqual(200);
  await page.getByRole('button', { name: 'Edit', exact: true }).click();
  await page.locator('.cm-content').fill('# Long tree edited note\n\nThis draft has room to breathe.');
  await expect.poll(async () => {
    const { ctx: c, base: b } = await apiCtx();
    const r = await c.get(`${b}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/note?path=long-tree%2Fnote-15.md`);
    const note = await r.json(); await c.dispose(); return note.raw;
  }).toContain('room to breathe');
  await expectNoHorizontalOverflow(page);
});

test('Canvas same-scene reopen keeps the newer draft when an older save resolves', async ({ page }) => {
  const { ctx, base } = await apiCtx(); const scenes: { id: string; title: string }[] = [];
  for (const name of ['Alpha', 'Beta']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`, { data: {
      title: `R4 ${name} ${Date.now()}`, doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: `flowchart LR\n A[${name}] --> B[Start]` },
    } }); scenes.push(await r.json());
  }
  let release!: () => void; const held = new Promise<void>(r => release = r); let puts = 0;
  await page.context().route(`**/canvas/scenes/${scenes[0].id}`, async route => {
    if (route.request().method() !== 'PUT') return route.continue();
    if (++puts === 1) await held;
    return route.continue();
  });
  await openPage(page, 'canvas');
  const select = async (i: number) => page.locator('.scene-list .row', { hasText: scenes[i].title }).getByRole('button').first().click();
  await select(0); await page.getByTitle('Edit the Mermaid source', { exact: true }).click();
  await page.locator('.cm-content').fill('flowchart LR\n A[Old in-flight] --> B[Save]');
  await expect.poll(() => puts).toBe(1);
  await select(1); await select(0);
  await page.getByTitle('Edit the Mermaid source', { exact: true }).click();
  await page.locator('.cm-content').fill('flowchart LR\n A[Reopened latest] --> B[Save]');
  release();
  await expect.poll(async () => JSON.parse((await (await ctx.get(`${base}/api/v1/canvas/scenes/${scenes[0].id}`)).json()).doc_json).source).toContain('Reopened latest');
  await expect(page.locator('.cm-content')).toContainText('Reopened latest');
  await ctx.dispose();
});

// Real MessageChannel/iframe timeout with a bounded protocol fixture. The first
// runtime never answers compile; already-queued callers must load a fresh one.
test('D2 timed-out transport is disposed and the next queued render recovers', async ({ page }) => {
  await page.route('**/node_modules/@terrastruct/d2/dist/browser/index.js*', route => route.fulfill({
    contentType: 'text/javascript', body: `
      let loads = 0;
      export class D2 {
        ready = Promise.reject(new Error('Maximum call stack size exceeded'));
        worker = { terminate() {} };
      }
      export async function ottoD2Assets() {
        const fail = ++loads === 1;
        return { wasm: new ArrayBuffer(0), source: \`
          port.onmessage = ({ data: { type, data } }) => {
            if (type === 'init') port.postMessage({ type: 'ready' });
            if (type === 'compile' && !\${fail}) port.postMessage({ type: 'result', data: { diagram: data.fs.index, renderOptions: {} } });
            if (type === 'render') port.postMessage({ type: 'result', data: '<svg><text>' + data.diagram + '</text></svg>' });
          };
        \` };
      }
    `,
  }));
  await page.route('**/src/modules/canvas/d2-frame.ts*', async route => {
    const response = await route.fetch();
    const body = await response.text();
    expect(body).toContain('3e4');
    await route.fulfill({ response, body: body.replace('3e4', '100') });
  });
  await page.goto('/');
  const result = await page.evaluate(async () => {
    const path = '/src/modules/canvas/d2.ts';
    const { renderD2, parseD2 } = await import(path);
    const renders = await Promise.all([renderD2('first', 'Stalled'), renderD2('next', 'Recovered')]);
    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }));
    return { renders, valid: await parseD2('Still ready'), frames: document.querySelectorAll('iframe[title="Diagram renderer"]').length };
  });
  expect(result.renders[0].error).toContain('timed out');
  expect(result.renders[1].svg).toContain('Recovered');
  expect(result.valid).toBe(true);
  expect(result.frames).toBe(1);
});

test('tablet Canvas gives the editor full width and toggles scenes without losing its draft', async ({ page }, info) => {
  await page.setViewportSize({ width: 834, height: 1112 });
  await page.addInitScript(() => {
    localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_direction', 'rtl');
  });
  const { ctx, base } = await apiCtx();
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`, { data: {
    title: `Tablet draft ${Date.now()}`, doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n A[Draft] --> B[Review]' },
  } });
  expect(created.ok()).toBeTruthy(); const scene = await created.json(); await ctx.dispose();
  await openPage(page, 'canvas');
  await page.locator('.scene-list .row', { hasText: scene.title }).getByRole('button').first().click();
  expect((await page.locator('.canvas-page .main').boundingBox())!.width).toBeGreaterThan(580);
  await page.getByTitle('Edit the Mermaid source', { exact: true }).click();
  expect((await page.locator('.board .code-pane').boundingBox())!.width).toBeGreaterThan(580);
  await page.locator('.cm-content').fill('flowchart LR\n A[Unsaved tablet draft] --> B[Review]');
  await page.getByRole('button', { name: 'Hide scenes', exact: true }).click();
  await expect(page.locator('.canvas-page .scenes')).toBeHidden();
  await expect(page.locator('.cm-content')).toContainText('Unsaved tablet draft');
  const showScenes = page.getByRole('button', { name: 'Show scenes', exact: true });
  await showScenes.focus(); await showScenes.press('Enter');
  await expect(page.locator('.canvas-page .scenes')).toBeVisible();
  await expect(page.locator('.cm-content')).toContainText('Unsaved tablet draft');
  await expect(page.getByRole('button', { name: 'Hide scenes', exact: true })).toBeFocused();
  await expect(page.locator('.board .content')).toContainText('Unsaved tablet draft');
  await expect.poll(async () => {
    const diagram = (await page.locator('.board .content > svg').boundingBox())!;
    const surface = (await page.locator('.board .surface').boundingBox())!;
    return diagram.x >= surface.x - 1 && diagram.y >= surface.y - 1 && diagram.x + diagram.width <= surface.x + surface.width + 1 && diagram.y + diagram.height <= surface.y + surface.height + 1;
  }).toBe(true);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('tablet-canvas.png') });
});
