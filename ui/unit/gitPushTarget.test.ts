import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { pushRefusal } from '../src/modules/git/push-errors.ts';

test('force retry carries the source and destination captured before the initial push', async () => {
  const target = { branch: 'feature-a', source_sha: 'a'.repeat(40), remote: 'upstream',
    destination_ref: 'refs/heads/review-a', destination_hash: 'b'.repeat(64), remote_sha: 'c'.repeat(40) };
  const requests: Record<string, unknown>[] = [];
  class ApiError extends Error { status = 409; }
  const ctx: Record<string, any> = {
    Error, ApiError, pushRefusal, runPull: async () => {},
    api: {
      get: async () => target,
      post: async (_url: string, body: Record<string, unknown>) => {
        requests.push(JSON.parse(JSON.stringify(body)));
        if (requests.length === 1) throw new ApiError('push rejected: non-fast-forward');
        return { branch: 'feature-b', upstream: 'origin/feature-b' };
      },
    },
    confirmer: { choose: async () => ({ value: 'force' }) },
    toasts: { error() {}, success() {} },
  };
  const source = readFileSync(new URL('../src/modules/git/pushFlow.ts', import.meta.url), 'utf8');
  const ast = ts.createSourceFile('flow.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const functions = ast.statements.filter(ts.isFunctionDeclaration).map(n => n.getText(ast).replace(/^export\s+/, '')).join('\n');
  runInNewContext(ts.transpileModule(functions, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  await ctx.runPush('repo', { branch: 'feature-a', upstream: 'upstream/review-a' }, () => {});
  assert.equal(requests.length, 2);
  assert.deepEqual(requests[1], { force_with_lease: true, expected_target: target });
});
