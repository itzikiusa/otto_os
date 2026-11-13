// lib/pathLinks/resolve.ts — a printed path is tried under each candidate
// root in order and the first one that exists wins; nothing is guessed.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  candidatePaths,
  cleanPathRef,
  joinRoot,
  looksLikeNonPath,
  MAX_CANDIDATES,
  notFoundDetail,
  orderRoots,
  resolvePathRef,
  type StatAnswer,
} from '../src/lib/pathLinks/resolve.ts';

/** A fake daemon: `files` / `dirs` exist, `blocked` cannot be answered. */
function fakeStat(files: string[], dirs: string[] = [], blocked: string[] = []) {
  const asked: string[] = [];
  const stat = async (p: string): Promise<StatAnswer> => {
    asked.push(p);
    if (blocked.includes(p)) return 'unknown';
    if (files.includes(p)) return { kind: 'file', path: p };
    if (dirs.includes(p)) return { kind: 'dir', path: p };
    return 'missing';
  };
  return { stat, asked };
}

test('candidate order decides: the first root where the path exists wins, not the first root', async () => {
  const { stat } = fakeStat(['/b/src/app.ts', '/c/src/app.ts']);
  const res = await resolvePathRef({ path: 'src/app.ts' }, ['/a', '/b', '/c'], stat);
  assert.deepEqual(res, { found: true, path: '/b/src/app.ts', dir: false, line: undefined, col: undefined, verified: true });
});

test('line and column survive resolution', async () => {
  const ref = cleanPathRef('a/b/c.rs:12:5')!;
  assert.deepEqual(ref, { path: 'a/b/c.rs', line: 12, col: 5 });
  const res = await resolvePathRef(ref, ['/x', '/y'], fakeStat(['/y/a/b/c.rs']).stat);
  assert.ok(res.found);
  assert.equal(res.path, '/y/a/b/c.rs');
  assert.equal(res.line, 12);
  assert.equal(res.col, 5);
  assert.deepEqual(cleanPathRef('src/x.ts#L8C2'), { path: 'src/x.ts', line: 8, col: 2 });
});

test('wrappers and sentence punctuation are not part of the path', () => {
  for (const raw of ['"src/x.ts"', '`src/x.ts`', '(src/x.ts)', '[src/x.ts]', '<src/x.ts>', 'src/x.ts.', '“src/x.ts”', '`src/x.ts`,']) {
    assert.equal(cleanPathRef(raw)?.path, 'src/x.ts', raw);
  }
  assert.deepEqual(cleanPathRef('`src/x.ts:4`'), { path: 'src/x.ts', line: 4, col: undefined });
});

test('a trailing slash means a folder: a file with that name does not answer it', async () => {
  assert.equal(cleanPathRef('marketing/videos/tour/')?.path, 'marketing/videos/tour/');
  assert.deepEqual(candidatePaths('marketing/videos/tour/', ['/w']), ['/w/marketing/videos/tour']);
  const dir = await resolvePathRef({ path: 'marketing/videos/tour/' }, ['/a', '/b'], fakeStat([], ['/b/marketing/videos/tour']).stat);
  assert.ok(dir.found && dir.dir);
  assert.equal(dir.path, '/b/marketing/videos/tour');
  const notDir = await resolvePathRef({ path: 'notes.md/' }, ['/a'], fakeStat(['/a/notes.md']).stat);
  assert.equal(notDir.found, false);
});

test('~ and absolute paths are verified as printed, never joined to a root', async () => {
  assert.deepEqual(candidatePaths('~/notes/a.md', ['/w']), ['~/notes/a.md']);
  assert.deepEqual(candidatePaths('/abs/a.md', ['/w']), ['/abs/a.md']);
  // The daemon canonicalizes `~`: its answer (the real path) is what opens.
  const home = await resolvePathRef({ path: '~/notes/a.md' }, ['/w'], async () => ({ kind: 'file', path: '/Users/me/notes/a.md' }));
  assert.ok(home.found);
  assert.equal(home.path, '/Users/me/notes/a.md');
  // An absolute path that does not exist is NOT opened (and roots are not tried).
  const { stat, asked } = fakeStat(['/w/abs/a.md']);
  const missing = await resolvePathRef({ path: '/abs/a.md' }, ['/w'], stat, async () => ['/other']);
  assert.deepEqual(missing, { found: false, path: '/abs/a.md', tried: ['/abs/a.md'] });
  assert.deepEqual(asked, ['/abs/a.md']);
});

test('.. and ./ keep their filesystem meaning (no lexical folding)', () => {
  assert.equal(joinRoot('/work/ui/', '../Cargo.toml'), '/work/ui/../Cargo.toml');
  assert.equal(joinRoot('/work', './src/a.ts'), '/work/src/a.ts');
  assert.equal(joinRoot('/work', './'), '/work');
  assert.deepEqual(candidatePaths('../x.rs', ['/a/b', '/a/b/', '/c']), ['/a/b/../x.rs', '/c/../x.rs']);
});

test('nothing found: the answer says where it looked, as a count', async () => {
  const res = await resolvePathRef({ path: 'gone/file.ts' }, ['/a', '/b'], fakeStat([]).stat, async () => ['/c']);
  assert.deepEqual(res, { found: false, path: 'gone/file.ts', tried: ['/a/gone/file.ts', '/b/gone/file.ts', '/c/gone/file.ts'] });
  assert.match(notFoundDetail(res as { path: string; tried: string[] }), /Looked in 3 places/);
  assert.match(notFoundDetail({ path: 'x', tried: ['/a/x'] }), /\/a\/x/);
  assert.match(notFoundDetail({ path: 'x', tried: [] }), /No folder is known/);
});

test('the on-demand roots (repos, workspace) are asked only when the session folders miss', async () => {
  let asked = 0;
  const more = async () => { asked++; return ['/repo']; };
  const hit = await resolvePathRef({ path: 'a.ts' }, ['/cwd'], fakeStat(['/cwd/a.ts']).stat, more);
  assert.ok(hit.found);
  assert.equal(asked, 0);
  const later = await resolvePathRef({ path: 'b.ts' }, ['/cwd'], fakeStat(['/repo/b.ts']).stat, more);
  assert.ok(later.found);
  assert.equal(later.path, '/repo/b.ts');
  assert.equal(asked, 1);
});

test('a candidate the daemon cannot answer (403 / busy) opens only when nothing was verified', async () => {
  const blocked = await resolvePathRef({ path: 'a.ts' }, ['/engine', '/w'], fakeStat([], [], ['/engine/a.ts']).stat);
  assert.deepEqual(blocked, { found: true, path: '/engine/a.ts', dir: false, line: undefined, col: undefined, verified: false });
  const verified = await resolvePathRef({ path: 'a.ts' }, ['/engine', '/w'], fakeStat(['/w/a.ts'], [], ['/engine/a.ts']).stat);
  assert.ok(verified.found && verified.verified);
  assert.equal(verified.path, '/w/a.ts');
  // A throwing stat is "no answer", never a crash.
  const thrown = await resolvePathRef({ path: 'a.ts' }, ['/w'], async () => { throw new Error('offline'); });
  assert.ok(thrown.found && !thrown.verified);
});

test('candidates are deduped, bounded, and skip empty / relative roots', () => {
  assert.deepEqual(orderRoots(['/a/', '', null], ['/a', '/b'], [undefined, '/c']), ['/a', '/b', '/c']);
  assert.deepEqual(candidatePaths('x.ts', ['', 'relative', '/a', '/a/']), ['/a/x.ts']);
  const many = Array.from({ length: 20 }, (_, i) => `/r${i}`);
  assert.equal(candidatePaths('x.ts', many).length, MAX_CANDIDATES);
});

test('version tags, numbers, fractions and schemeless URLs are not paths', () => {
  for (const s of ['v0.2.2', '1.2.3', '1.0.0-rc.1', 'V2.10.4', '1/2', '3/4/5', 'example.com/docs', 'www.github.io/a', 'mailto:me@x.io', 'javascript:alert(1)']) {
    assert.equal(looksLikeNonPath(s), true, s);
    assert.equal(cleanPathRef(s), null, s);
  }
  for (const s of ['src/v0.2.2/notes.md', 'releases/0.2.2/', 'Cargo.toml', './a.ts', '~/x']) {
    assert.equal(looksLikeNonPath(s), false, s);
  }
});
