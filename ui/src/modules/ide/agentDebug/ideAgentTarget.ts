// The agent's run target (`ide_debug_start.target`) resolved like a gutter
// click: which gutter item of the file, and which test name the adapter gets —
// a table-test case (`TestX/case`) and a testify suite method (its runner +
// `/Method`, which `go test -run` matches as a subtest) included. Pure;
// the port (ideAgentPort.svelte.ts) turns the item into the launch through
// `launchGutterAction`, so build tags and cwd are exactly the gutter's.

import type { IdeGutterItem } from '../store/ideTypes.ts';

export class IdeTargetError extends Error {
  readonly code: 'not_found' | 'invalid_args';
  constructor(code: 'not_found' | 'invalid_args', message: string) {
    super(message);
    this.code = code;
  }
}

const isTest = (i: IdeGutterItem): boolean => i.kind === 'test' || i.kind === 'suite';
const names = (i: IdeGutterItem): string[] =>
  [i.label, i.run_spec?.test_name, i.run_spec?.debug_test_name, i.run_spec?.test_filter].filter((s): s is string => !!s);

function known(items: readonly IdeGutterItem[], pred: (i: IdeGutterItem) => boolean, what: string): string {
  const list = items.filter(pred).slice(0, 20).map((i) => i.label);
  return list.length ? ` ${what} in this file: ${list.join(', ')}` : ` This file has no ${what}.`;
}

const esc = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The testify runner (`func TestX(t *testing.T) { suite.Run(t, new(T)) }`) of suite type `T`. */
function suiteRunner(content: string, type: string): string | null {
  const re = new RegExp(
    `func\\s+(Test\\w*)\\s*\\(\\s*\\w+\\s+\\*testing\\.T\\s*\\)\\s*\\{[^}]*?suite\\.Run\\(\\s*\\w+\\s*,\\s*(?:new\\(\\s*${esc(type)}\\s*\\)|&${esc(type)}\\s*\\{)`,
  );
  return re.exec(content)?.[1] ?? null;
}

/** A test target: the gutter item to launch (null = launch the file in test
 *  mode with just the name, the adapter finds the package) and the test name
 *  the adapter runs. */
export function pickTestTarget(
  items: readonly IdeGutterItem[],
  content: string,
  language: string,
  name: string,
): { item: IdeGutterItem | null; testName: string } {
  const want = name.trim();
  if (!want) throw new IdeTargetError('invalid_args', '`target.name` is required for a test');
  const exact = items.find((i) => isTest(i) && names(i).includes(want));
  if (exact) return { item: exact, testName: exact.run_spec?.debug_test_name || exact.run_spec?.test_name || exact.label };
  const slash = want.indexOf('/');
  if (slash > 0) {
    // A table-test case or a suite method spelled `Runner/Method`.
    const parent = items.find((i) => isTest(i) && names(i).includes(want.slice(0, slash)));
    if (parent || language === 'go') return { item: parent ?? null, testName: want };
  }
  if (language === 'go') {
    const method = new RegExp(`func\\s*\\(\\s*\\w*\\s*\\*?\\s*(\\w+)\\s*\\)\\s*${esc(want)}\\s*\\(`).exec(content);
    if (method) {
      const runner = suiteRunner(content, method[1]);
      if (!runner) {
        throw new IdeTargetError(
          'not_found',
          `${want} is a method of ${method[1]}, whose suite.Run is not in this file. Pass name as "TestRunner/${want}" (the test function that calls suite.Run).`,
        );
      }
      const item = items.find((i) => isTest(i) && names(i).includes(runner)) ?? null;
      return { item, testName: `${runner}/${want}` };
    }
  }
  throw new IdeTargetError('not_found', `No test ${want}.${known(items, isTest, 'Tests')}`);
}

/** A `main` target: the file's main / script entry. */
export function pickMainTarget(items: readonly IdeGutterItem[]): IdeGutterItem {
  const main = items.find((i) => i.kind === 'main');
  if (!main) throw new IdeTargetError('not_found', `No main in this file.${known(items, isTest, 'Tests')}`);
  return main;
}

function rel(path: string, root: string): string {
  const r = root.replace(/\/+$/, '');
  if (path === r) return '.';
  return path.startsWith(`${r}/`) ? path.slice(r.length + 1) : path;
}

/** `^A$/^B$` — what `go test -run` gets for `A/B` (the adapter does the same). */
function goRunPattern(name: string): string {
  return name.split('/').map((p) => `^${p.replace(/^\^/, '').replace(/\$$/, '')}$`).join('/');
}

/** The command line the start confirm shows (a readable preview, not argv;
 *  paths relative to the project `root` — the confirm names the cwd apart). */
export function launchPreview(l: {
  language: string;
  program: string;
  root: string;
  test_name?: string;
  build_flags?: readonly string[];
  args?: readonly string[];
}): string {
  const flags = (l.build_flags ?? []).join(' ');
  const args = (l.args ?? []).join(' ');
  const parts: string[] = [];
  const prog = rel(l.program, l.root);
  if (l.language === 'go') {
    const pkg = prog.endsWith('.go') ? prog.slice(0, Math.max(0, prog.lastIndexOf('/'))) || '.' : prog;
    const where = pkg === '.' || pkg.startsWith('./') || pkg.startsWith('/') ? pkg : `./${pkg}`;
    if (l.test_name) parts.push('go test', flags, `-run ${goRunPattern(l.test_name)}`, where, args);
    else parts.push('go run', flags, where, args);
  } else if (l.language === 'zig') {
    if (l.test_name) parts.push('zig test', flags, `--test-filter "${l.test_name}"`, prog, args);
    else parts.push('zig build', flags, prog, args);
  } else if (l.language === 'cpp' || l.language === 'c') {
    if (l.test_name) parts.push('ctest', flags, `-R "${l.test_name}"`, prog, args);
    else parts.push('cmake --build', flags, prog, args);
  } else if (l.language === 'python') {
    parts.push(l.test_name ? `pytest ${l.test_name.includes('::') ? l.test_name : `${prog}::${l.test_name}`}` : `python ${prog}`, args);
  } else {
    parts.push(`debug ${l.language}`, prog, l.test_name ? `test ${l.test_name}` : '', flags, args);
  }
  return parts.filter(Boolean).join(' ');
}
