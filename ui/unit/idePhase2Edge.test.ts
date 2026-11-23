import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  extractGutterHunks,
  findGutterHunkAtLine,
  formatHunkTitle,
  revertHunkInText,
} from '../src/modules/ide/git/ideGutterHunks.ts';
import {
  buildMultiAgentReviewRoute,
  findRegisteredGitRepo,
  normalizeGitRepoPath,
} from '../src/modules/ide/git/ideGitReviewRoute.ts';
import {
  inferTestDebugLanguage,
  planFailedTestDebug,
} from '../src/modules/ide/debug/ideFailedTestDebug.ts';
import type { IdeGutterItem } from '../src/modules/ide/store/ideTypes.ts';

describe('E-10a: ideGutterHunks extraction, unified diff, and revert', () => {
  it('returns empty for missing base or identical content', () => {
    assert.deepEqual(extractGutterHunks(null, 'a\nb\n'), []);
    assert.deepEqual(extractGutterHunks(undefined, 'a\nb\n'), []);
    assert.deepEqual(extractGutterHunks('a\nb\n', 'a\nb\n'), []);
    assert.deepEqual(extractGutterHunks('', ''), []);
  });

  it('extracts additions, modifications, and deletions with accurate unifiedDiff', () => {
    const base = ['line1', 'line2', 'line3', 'line4', 'line5', 'line6'].join('\n') + '\n';
    const current = [
      'line1',
      'line2-edited',
      'line3',
      'line4',
      'line4-added',
      'line5',
    ].join('\n') + '\n';

    const hunks = extractGutterHunks(base, current, 'src/app.ts');
    assert.equal(hunks.length, 3);

    // Hunk 0: modification at L2
    assert.equal(hunks[0].index, 0);
    assert.equal(hunks[0].kind, 'mod');
    assert.equal(hunks[0].startLine, 2);
    assert.equal(hunks[0].endLine, 2);
    assert.equal(hunks[0].baseStart, 2);
    assert.equal(hunks[0].baseEnd, 2);
    assert.deepEqual(hunks[0].removedLines, ['line2']);
    assert.deepEqual(hunks[0].addedLines, ['line2-edited']);
    assert.match(hunks[0].unifiedDiff, /^--- a\/src\/app\.ts\n\+\+\+ b\/src\/app\.ts\n@@ -2,1 \+2,1 @@\n-line2\n\+line2-edited$/);

    // Hunk 1: pure addition at L5
    assert.equal(hunks[1].index, 1);
    assert.equal(hunks[1].kind, 'add');
    assert.equal(hunks[1].startLine, 5);
    assert.equal(hunks[1].endLine, 5);
    assert.deepEqual(hunks[1].removedLines, []);
    assert.deepEqual(hunks[1].addedLines, ['line4-added']);
    assert.match(hunks[1].unifiedDiff, /@@ -4,0 \+5,1 @@\n\+line4-added$/);

    // Hunk 2: pure deletion of line6 after L6
    assert.equal(hunks[2].index, 2);
    assert.equal(hunks[2].kind, 'del');
    assert.deepEqual(hunks[2].removedLines, ['line6']);
    assert.deepEqual(hunks[2].addedLines, []);
    assert.equal(hunks[2].anchorLine, 6);
    assert.match(hunks[2].unifiedDiff, /@@ -6,1 \+6,0 @@\n-line6$/);

    // Line lookup hits each hunk and returns null on unchanged lines
    assert.equal(findGutterHunkAtLine(hunks, 1), null);
    assert.equal(findGutterHunkAtLine(hunks, 2)?.index, 0);
    assert.equal(findGutterHunkAtLine(hunks, 5)?.index, 1);
    assert.equal(findGutterHunkAtLine(hunks, 6)?.index, 2);
  });

  it('reverts a single hunk without touching sibling hunks and preserves trailing newline', () => {
    const base = 'alpha\nbeta\ngamma\ndelta\n';
    const current = 'ALPHA\nbeta\ngamma\ndelta\nepsilon\n';

    // Revert only hunk 0 (ALPHA -> alpha), keeping epsilon
    const afterFirst = revertHunkInText(base, current, 0);
    assert.equal(afterFirst, 'alpha\nbeta\ngamma\ndelta\nepsilon\n');

    // Revert remaining hunk (pure addition epsilon) -> matches base exactly
    const afterSecond = revertHunkInText(base, afterFirst, 0);
    assert.equal(afterSecond, base);

    // Out-of-range hunk index is a safe no-op
    assert.equal(revertHunkInText(base, current, -1), current);
    assert.equal(revertHunkInText(base, current, 99), current);
  });

  it('reverts a pure deletion back to base text (including full-file wipe)', () => {
    const base = 'a\nb\nc\n';
    assert.equal(revertHunkInText(base, 'a\nc\n', 0), base);
    assert.equal(revertHunkInText(base, '', 0), base);
  });

  it('formats singular and plural hunk headers', () => {
    assert.equal(
      formatHunkTitle({ kind: 'add', startLine: 3, endLine: 3, anchorLine: 3, addedLines: ['x'], removedLines: [] }),
      'Added 1 line at L3',
    );
    assert.equal(
      formatHunkTitle({ kind: 'add', startLine: 3, endLine: 5, anchorLine: 3, addedLines: ['a', 'b', 'c'], removedLines: [] }),
      'Added 3 lines at L3–5',
    );
    assert.equal(
      formatHunkTitle({ kind: 'del', startLine: 4, endLine: 3, anchorLine: 3, addedLines: [], removedLines: ['x', 'y'] }),
      'Deleted 2 lines near L3',
    );
    assert.equal(
      formatHunkTitle({ kind: 'mod', startLine: 10, endLine: 12, anchorLine: 10, addedLines: ['a'], removedLines: ['b'] }),
      'Modified L10–12',
    );
  });
});

describe('E-10b: ideGitReviewRoute repo lookup and route builder', () => {
  const repos = [
    { id: 'repo-root', path: '/Users/dev/monorepo', name: 'monorepo' },
    { id: 'repo-sub', path: '/Users/dev/monorepo/packages/web/', name: 'web' },
    { id: 'repo-tmp', path: '/private/var/folders/demo', name: 'demo' },
  ];

  it('normalizes trailing slashes, backslashes, and /private/var prefixes', () => {
    assert.equal(normalizeGitRepoPath('/Users/dev/app///'), '/Users/dev/app');
    assert.equal(normalizeGitRepoPath('/private/var/tmp/repo/'), '/var/tmp/repo');
    assert.equal(normalizeGitRepoPath('C:\\work\\repo\\'), 'C:/work/repo');
  });

  it('matches exact and deepest enclosing repository roots', () => {
    assert.equal(findRegisteredGitRepo(repos, '/Users/dev/monorepo')?.id, 'repo-root');
    assert.equal(findRegisteredGitRepo(repos, '/Users/dev/monorepo/packages/web/src/index.ts')?.id, 'repo-sub');
    assert.equal(findRegisteredGitRepo(repos, '/var/folders/demo/main.go')?.id, 'repo-tmp');
    assert.equal(findRegisteredGitRepo(repos, '/Users/dev/other'), null);
  });

  it('builds git/<repoId>/review when registered and falls back to git otherwise', () => {
    assert.deepEqual(buildMultiAgentReviewRoute(repos, '/Users/dev/monorepo/packages/web'), {
      route: 'git/repo-sub/review',
      repoId: 'repo-sub',
      registered: true,
    });
    assert.deepEqual(buildMultiAgentReviewRoute([{ id: 'org/repo 1', path: '/ws/r' }], '/ws/r'), {
      route: 'git/org%2Frepo%201/review',
      repoId: 'org/repo 1',
      registered: true,
    });
    assert.deepEqual(buildMultiAgentReviewRoute(repos, '/Users/dev/unregistered'), {
      route: 'git',
      repoId: null,
      registered: false,
    });
  });
});

describe('E-10c: ideFailedTestDebug launch mapping', () => {
  it('prefers matching gutter test item when available (exact line or Go subtest name)', () => {
    const gutterItem: IdeGutterItem = {
      line: 24,
      kind: 'test',
      label: 'TestCheckout',
      language: 'go',
      run_spec: { runner: 'go_test', target_path: '/ws/app/checkout_test.go', test_name: 'TestCheckout' },
    };

    const bySubtest = planFailedTestDebug({
      test: { name: 'TestCheckout/empty_cart', file: 'checkout_test.go', line: 31 },
      runner: 'go_test',
      targetRoot: '/ws/app',
      gutterByPath: { '/ws/app/checkout_test.go': [gutterItem] },
    });
    assert.equal(bySubtest?.kind, 'gutter');
    if (bySubtest?.kind === 'gutter') assert.equal(bySubtest.item.label, 'TestCheckout');

    const byLine = planFailedTestDebug({
      test: { name: 'renamed', file: '/ws/app/checkout_test.go', line: 24 },
      runner: 'go_test',
      targetRoot: '/ws/app',
      gutterByPath: { '/ws/app/checkout_test.go': [gutterItem] },
    });
    assert.equal(byLine?.kind, 'gutter');
  });

  it('builds direct debug launch spec when no gutter item matches', () => {
    const plan = planFailedTestDebug({
      test: { name: 'shouldFail', suite: 'com.acme.OrderTest', file: 'src/test/java/OrderTest.java', line: 42 },
      runner: 'maven',
      targetRoot: '/ws/svc',
    });
    assert.deepEqual(plan, {
      kind: 'direct',
      language: 'java',
      launch: {
        program: '/ws/svc/src/test/java/OrderTest.java',
        args: [],
        cwd: '/ws/svc',
        test_name: 'com.acme.OrderTest#shouldFail',
      },
    });
  });

  it('infers language from runner when file is absent and returns null when unlaunchable', () => {
    assert.equal(inferTestDebugLanguage(null, 'pytest'), 'python');
    assert.equal(inferTestDebugLanguage(null, 'cargo_test'), 'rust');
    assert.equal(inferTestDebugLanguage('foo.spec.ts', null), 'typescript');

    const nameOnly = planFailedTestDebug({
      test: { name: 'test_login', file: null, line: null },
      runner: 'pytest',
      targetRoot: '/ws/py',
    });
    assert.deepEqual(nameOnly, {
      kind: 'direct',
      language: 'python',
      launch: { program: '/ws/py', args: [], cwd: '/ws/py', test_name: 'test_login' },
    });

    assert.equal(planFailedTestDebug({ test: { name: '', file: null }, targetRoot: '/ws/py' }), null);
    assert.equal(planFailedTestDebug({ test: { name: 'TestA', file: 'a_test.go' }, targetRoot: '' }), null);
  });
});
