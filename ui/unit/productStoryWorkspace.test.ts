// api-contracts-4-01: the story agent routes (analyze, rewrite, testcases /
// plan generate, plan, inject-session) 404 for any `{ws}` but the story's
// creating workspace, so the store builds them from `story.workspace_id`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const src = readFileSync(new URL('../src/lib/stores/product.svelte.ts', import.meta.url), 'utf8');
const file = ts.createSourceFile('p.ts', src, ts.ScriptTarget.Latest, true);
const fn = file.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === 'storyWorkspaceId')!;
const ctx: Record<string, any> = {};
runInNewContext(ts.transpileModule(fn.getText(file).replace(/^export\s+/, ''), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
const storyWorkspaceId = ctx.storyWorkspaceId as (id: string, s: any[], d: any, f: string) => string;

test('the story’s own workspace wins over the current one', () => {
  const stories = [{ id: 's1', workspace_id: 'A' }, { id: 's2', workspace_id: 'B' }];
  assert.equal(storyWorkspaceId('s1', stories, null, 'B'), 'A');
  assert.equal(storyWorkspaceId('s2', stories, { id: 's2', workspace_id: 'B' }, 'A'), 'B');
  // The loaded detail answers even when the list doesn't hold the story.
  assert.equal(storyWorkspaceId('s3', stories, { id: 's3', workspace_id: 'C' }, 'A'), 'C');
  // Unknown story: fall back to the current workspace.
  assert.equal(storyWorkspaceId('s9', stories, null, 'A'), 'A');
});

test('every story agent route uses the story scope, not the current workspace', () => {
  for (const route of ['/analyze`', '/rewrite`', '/plan/generate`', '/plan`', '/testcases/generate`', '/inject-session`']) {
    const i = src.indexOf(`product/stories/\${id}${route}`);
    assert.ok(i > 0, route);
    const fnStart = src.lastIndexOf('async ', i);
    assert.match(src.slice(fnStart, i), /this\.storyScope\(\)/, route);
  }
});
