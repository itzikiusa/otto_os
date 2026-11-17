import { test } from 'node:test';
import assert from 'node:assert/strict';
import { nestParentName, nestSiblings } from '../src/modules/ide/history/ideTreeNesting.ts';
import type { IdeFsEntry } from '../src/modules/ide/store/ideTypes.ts';

const f = (dir: string, name: string, kind: IdeFsEntry['kind'] = 'file'): IdeFsEntry => ({ name, path: `${dir}/${name}`, kind });
const nestedNames = (entries: IdeFsEntry[], enabled = true): Record<string, string[]> =>
  Object.fromEntries([...nestSiblings(entries, enabled).childrenOf].map(([p, kids]) => [p.split('/').pop()!, kids.map((k) => k.name)]));

test('the common lockfile / companion pairs nest under their parent', () => {
  const names = ['go.mod', 'go.sum', 'go.work', 'go.work.sum', 'package.json', 'package-lock.json', 'yarn.lock', 'pnpm-lock.yaml',
    'npm-shrinkwrap.json', 'Cargo.toml', 'Cargo.lock', 'pyproject.toml', 'poetry.lock', 'uv.lock', 'pdm.lock', 'Gemfile', 'Gemfile.lock',
    'composer.json', 'composer.lock', 'tsconfig.json', 'tsconfig.app.json', 'app.ts', 'app.js', 'app.js.map', 'app.d.ts', '.env', '.env.local', 'main.go'];
  assert.deepEqual(nestedNames(names.map((n) => f('/r', n))), {
    'go.mod': ['go.sum'],
    'go.work': ['go.work.sum'],
    'package.json': ['package-lock.json', 'yarn.lock', 'pnpm-lock.yaml', 'npm-shrinkwrap.json'],
    'Cargo.toml': ['Cargo.lock'],
    'pyproject.toml': ['poetry.lock', 'uv.lock', 'pdm.lock'],
    Gemfile: ['Gemfile.lock'],
    'composer.json': ['composer.lock'],
    'tsconfig.json': ['tsconfig.app.json'],
    'app.ts': ['app.js', 'app.js.map', 'app.d.ts'],
    '.env': ['.env.local'],
  });
  const n = nestSiblings(names.map((x) => f('/r', x)));
  assert.equal(n.parentOf.get('/r/go.sum'), '/r/go.mod');
  assert.equal(n.parentOf.has('/r/main.go'), false);
});

test('a child without its parent stays at the top level', () => {
  assert.deepEqual(nestedNames([f('/r', 'go.sum'), f('/r', 'yarn.lock'), f('/r', 'util.js'), f('/r', '.env.example')]), {});
});

test('nothing nests across folders, and folders never nest', () => {
  // go.mod lives in /r, go.sum in /r/sub: two listings, two separate nest passes.
  assert.deepEqual(nestedNames([f('/r', 'go.mod'), f('/r', 'sub', 'dir')]), {});
  assert.deepEqual(nestedNames([f('/r/sub', 'go.sum')]), {});
  // A folder named like a parent or child is not nested either.
  assert.deepEqual(nestedNames([f('/r', 'go.mod', 'dir'), f('/r', 'go.sum')]), {});
});

test('the setting off nests nothing', () => {
  const n = nestSiblings([f('/r', 'go.mod'), f('/r', 'go.sum')], false);
  assert.equal(n.childrenOf.size, 0);
  assert.equal(n.parentOf.size, 0);
});

test('rule edges: no self-nesting, only the clear TypeScript cases', () => {
  assert.equal(nestParentName('tsconfig.json'), null);
  assert.equal(nestParentName('app.ts'), null);
  assert.equal(nestParentName('.env'), null);
  assert.equal(nestParentName('README.md'), null);
  assert.equal(nestParentName('x.d.ts'), 'x.ts');
});
