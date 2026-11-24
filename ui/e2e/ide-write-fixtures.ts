import { expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// The IDE's Makefile + Markdown specs' own project: the unit tokenizer fixture as
// `Makefile` (modelled on the owner's go_acceptance_tests Makefile), the `common.mk`
// it includes, a Go file to compare against, and a README.

export const MAKEFILE = readFileSync(join(process.cwd(), 'unit', 'fixtures', 'Makefile.sample'), 'utf8');
export const COMMON_MK = readFileSync(join(process.cwd(), 'unit', 'fixtures', 'common.mk'), 'utf8');

export const MAIN_GO = `package main

import (
\t"fmt"
\t"os"
)

// main prints the build's commit.
func main() {
\tcommit := os.Getenv("GIT_COMMIT")
\tfmt.Println("commit", commit)
}
`;

export const README = '# Games Management\n\nIntro paragraph.\n\n## Setup\n\nRun make.\n';

/** Config files (the config-language specs): a Java .properties like the owner's, a
 *  multi-document YAML, a Cargo.toml, a Dockerfile, a .env and an unknown key = value file. */
export const CONFIG_PLACEHOLDER = '$' + '{DB_HOST:localhost}';
export const CONFIG_FILES: Record<string, string> = {
  'application.properties': [
    '# Games service: database and Kafka',
    'spring.datasource.url=jdbc:mysql://' + CONFIG_PLACEHOLDER + ':3306/games?useSSL=false&serverTimezone=UTC',
    'spring.datasource.username=games',
    'spring.kafka.consumer.properties={"max.poll.records": 500, "enable.auto.commit": false, "topics": ["bets", "wins"]}',
    'server.port : 8080',
    'app.banner=Welcome to the games service, the one with the very long property values that run off the screen \\',
    '    and continue on the next line',
    '! legacy',
  ].join('\n') + '\n',
  'deploy.yaml': [
    '# Deployment',
    'apiVersion: apps/v1',
    'kind: Deployment',
    'metadata:',
    '  name: games',
    '  labels: &labels',
    '    app: games',
    'spec:',
    '  replicas: 3',
    '  selector:',
    '    matchLabels: *labels',
    '  script: |',
    '    echo starting',
    '---',
    'apiVersion: v1',
    'kind: Service',
  ].join('\n') + '\n',
  'Cargo.toml': '[package]\nname = "otto"\nversion = "0.5.0"\n# the edition\nedition = "2021"\n\n[dependencies]\nserde = { version = "1", features = ["derive"] }\ntokio = "1"\n',
  'Dockerfile': '# build stage\nFROM golang:1.23 AS build\nENV CGO_ENABLED=0\nRUN go build -o /out/app ./cmd/...\nEXPOSE 8080\n',
  'prod.env': '# local settings\nexport DB_URL=postgres://u@h/db\nAPI_KEY="abc123"\nRETRIES=3\n',
  'service.overrides': '# per-host overrides\nhost = db.internal\nport = 5432\n[cache]\nttl = 30\n',
};

export interface WriteSeed {
  workspaceId: string;
  root: string;
}

let seed: WriteSeed | null = null;

export async function getWriteSeed(): Promise<WriteSeed> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-write-')));
  mkdirSync(join(root, 'cmd'), { recursive: true });
  writeFileSync(join(root, 'Makefile'), MAKEFILE);
  writeFileSync(join(root, 'common.mk'), COMMON_MK);
  writeFileSync(join(root, 'cmd', 'main.go'), MAIN_GO);
  writeFileSync(join(root, 'go.mod'), 'module example.com/games\n\ngo 1.22\n');
  writeFileSync(join(root, 'README.md'), README);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE Write ${Date.now()}`, root_path: root });
    seed = { workspaceId: ws.id, root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

/** Boot #/ide on the project (autosave off, so edits stay in the buffer). */
export async function bootWriteIde(page: Page, scheme: 'dark' | 'light' = 'dark'): Promise<WriteSeed> {
  const s = await getWriteSeed();
  await page.addInitScript(
    ({ wsId, root, uiScheme }) => {
      if (sessionStorage.getItem('__ide_write_init')) return;
      sessionStorage.setItem('__ide_write_init', '1');
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', uiScheme);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.preferences.v1', JSON.stringify({ autoSave: false }));
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-write', name: 'Games', folders: [{ path: root, name: 'games' }], updatedAt: Date.now() }]));
    },
    { wsId: s.workspaceId, root: s.root, uiScheme: scheme },
  );
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-editor-area').or(page.getByTestId('ide-project-tree')).first()).toBeVisible({ timeout: 20_000 });
  return s;
}

/** Open a file from the project tree (desktop: double-click pins it). */
export async function openTreeFile(page: Page, rel: string): Promise<void> {
  const parts = rel.split('/');
  const item = (r: string) => page.getByTestId('ide-project-tree').locator(`button.tree-item[title="${r}"], button.tree-item[title$="/${r}"]`).first();
  for (let i = 0; i < parts.length - 1; i++) {
    const dir = item(parts.slice(0, i + 1).join('/'));
    await expect(dir).toBeVisible({ timeout: 15_000 });
    if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.click();
  }
  const f = item(rel);
  await expect(f).toBeVisible({ timeout: 15_000 });
  await f.dblclick();
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText(parts[parts.length - 1], { timeout: 15_000 });
  await expect(page.locator('.ide-editor-area .cm-editor .cm-line').first()).toBeVisible({ timeout: 15_000 });
}

/** Screenshot into IDE_SHOTS (skipped when unset). */
export async function writeShot(page: Page, name: string): Promise<void> {
  const dir = process.env.IDE_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
  await page.screenshot({ path: join(dir, `${name}.png`) });
}
