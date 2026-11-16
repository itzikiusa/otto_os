import { expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall, seedPolyglotFiles } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// The IDE preview specs' own project (desktop-ide-preview / ide-preview-mobile):
// the polyglot files (so app/main.go is there as the "no preview" case) plus a
// README with every Markdown feature the preview renders, a linked doc, an HTML
// page that probes its sandbox, JSON (valid + broken), an SVG and a PNG.
// Own workspace — createPolyglotSeed posts a run config with a FIXED id.

export const README = [
  '# Otto Preview Fixture',
  '',
  'Intro with a [guide link](docs/guide.md) and an [external link](https://example.com/otto).',
  '',
  '| Name | Value |',
  '| --- | --- |',
  '| alpha | 1 |',
  '| beta | 2 |',
  '',
  '- [x] done task',
  '- [ ] open task',
  '',
  '```go',
  'func main() { fmt.Println("hi") }',
  '```',
  '',
  '```mermaid',
  'flowchart LR',
  '  A[Start] --> B[Finish]',
  '```',
  '',
  '![one pixel](./pixel.png)',
  '',
  '## Usage',
  '',
  'Run it.',
  '',
].join('\n');

/** Probes, from INSIDE the frame, whether it can reach Otto: with scripts on
 *  and no same-origin it must read "parent:blocked storage:blocked". */
export const INDEX_HTML = [
  '<!doctype html><html><head><title>Probe</title></head><body>',
  '<h1>Sandboxed Page</h1><p id="probe">not run</p>',
  '<script>',
  'var out = [];',
  "try { out.push('parent:' + (window.parent.document.title ? 'READ' : 'empty')); } catch (e) { out.push('parent:blocked'); }",
  "try { out.push('storage:' + (window.localStorage.getItem('otto_token') !== undefined ? 'READ' : 'none')); } catch (e) { out.push('storage:blocked'); }",
  "document.getElementById('probe').textContent = out.join(' ');",
  '</script></body></html>',
].join('\n');

// 1×1 transparent PNG.
const PIXEL_PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=';

export interface PreviewSeed {
  workspaceId: string;
  workspaceRoot: string;
}

let seed: PreviewSeed | null = null;

export async function getPreviewSeed(): Promise<PreviewSeed> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-preview-')));
  seedPolyglotFiles(root);
  mkdirSync(join(root, 'docs'), { recursive: true });
  writeFileSync(join(root, 'README.md'), README);
  writeFileSync(join(root, 'docs', 'guide.md'), '# Guide Page\n\nBack to the [readme](../README.md).\n');
  writeFileSync(join(root, 'index.html'), INDEX_HTML);
  writeFileSync(join(root, 'data.json'), JSON.stringify({ name: 'otto', items: [1, 2, 3], nested: { ok: true } }, null, 2));
  writeFileSync(join(root, 'broken.json'), '{ "a": 1,, }\n');
  writeFileSync(join(root, 'logo.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="teal"/></svg>\n');
  writeFileSync(join(root, 'pixel.png'), Buffer.from(PIXEL_PNG, 'base64'));
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', {
      name: `IDE Preview ${Date.now()}`,
      root_path: root,
    });
    seed = { workspaceId: ws.id, workspaceRoot: root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

/** Boot #/ide on the preview project. Autosave off, so "typing does not save"
 *  is observable; the init script runs once per page (a reload keeps state). */
export async function bootPreviewIde(page: Page, scheme: 'dark' | 'light' = 'dark'): Promise<PreviewSeed> {
  const s = await getPreviewSeed();
  await page.addInitScript(
    ({ wsId, root, uiScheme }) => {
      if (sessionStorage.getItem('__ide_preview_init')) return;
      sessionStorage.setItem('__ide_preview_init', '1');
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', uiScheme);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.setItem('otto.ide.preferences.v1', JSON.stringify({ autoSave: false }));
      localStorage.setItem(
        'otto.ide.namedProjects.v1',
        JSON.stringify([{ id: 'proj-preview', name: 'Otto IDE Preview Project', folders: [{ path: root, name: 'otto-preview' }], updatedAt: Date.now() }]),
      );
    },
    { wsId: s.workspaceId, root: s.workspaceRoot, uiScheme: scheme },
  );
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-editor-area').or(page.getByTestId('ide-project-tree')).first()).toBeVisible({ timeout: 20_000 });
  return s;
}

/** A project-tree file button by its path relative to the root. */
export function treeFile(page: Page, rel: string) {
  return page
    .getByTestId('ide-project-tree')
    .locator(`button.tree-item[title="${rel}"], button.tree-item[title$="/${rel}"]`)
    .first();
}

/** Screenshot into OTTO_E2E_SHOTS (skipped when unset — never a fixed path). */
export async function previewShot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  const out = join(dir, 'ide-preview');
  mkdirSync(out, { recursive: true });
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
  await page.screenshot({ path: join(out, `${name}.png`) });
}
