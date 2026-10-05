import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';

test.use({serviceWorkers: 'block', viewport: {width: 1440, height: 1000}});
for (const format of ['mermaid', 'd2', 'excalidraw'] as const) {
  test(`Canvas ${format} keeps a failed or empty assistant request for retry and clears accepted input`, async ({page}) => {
    const drawing = {type: 'excalidraw', version: 2, elements: [], appState: {viewBackgroundColor: '#ffffff', gridSize: 20}, files: {}};
    const source = format === 'mermaid' ? 'flowchart LR\n A[Before] --> B[Before]'
      : format === 'd2' ? 'before -> canvas' : JSON.stringify(drawing);
    const {ctx, base} = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/canvas/scenes`, {
      data: {title: `Recovery ${format}`, doc: {type: 'otto-canvas', version: 1, format, source}},
    });
    expect(response.ok()).toBe(true);
    const scene = await response.json();
    await ctx.dispose();
    await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
    const sent: string[] = [];
    await page.route(`**/api/v1/canvas/scenes/${scene.id}/assist`, route => {
      sent.push(route.request().postDataJSON().prompt);
      if (sent.length === 1) return route.fulfill({status: 503, json: {code: 'unavailable', message: 'Synthetic provider failure'}});
      if (sent.length === 2) return route.fulfill({json: {note: 'No diagram was produced'}});
      const result = format === 'mermaid' ? {mermaid: 'flowchart LR\n A[Recovered] --> B[Canvas]'}
        : format === 'd2' ? {d2: 'recovered -> canvas'} : {excalidraw: {...drawing, appState: {...drawing.appState, gridSize: 40}}};
      return route.fulfill({json: {...result, note: 'Accepted request'}});
    });
    await page.goto('/#/canvas');
    await page.locator('.scene-list .row', {hasText: `Recovery ${format}`}).getByRole('button').first().click();
    await page.locator('.ai-fab').click();
    const prompt = page.getByRole('textbox', {name: 'Ask the canvas assistant'});
    await prompt.fill('Keep this exact request until accepted');
    for (let attempt = 1; attempt <= 2; attempt++) {
      await prompt.press('Enter');
      await expect.poll(() => sent.length).toBe(attempt);
      await expect(prompt).toBeEnabled();
      await expect(prompt).toHaveValue('Keep this exact request until accepted');
    }
    await page.locator('.assistant').getByRole('button', {name: 'Send', exact: true}).click();
    await expect.poll(() => sent.length).toBe(3);
    await expect(prompt).toHaveValue('');
    expect(sent).toEqual(Array(3).fill('Keep this exact request until accepted'));
    await expect.poll(() => page.evaluate(async () => {
      const path = '/src/lib/stores/canvas.svelte.ts';
      return (await import(path)).canvas.source;
    })).toContain(format === 'excalidraw' ? '40' : format === 'mermaid' ? 'Recovered' : 'recovered');
  });
}
