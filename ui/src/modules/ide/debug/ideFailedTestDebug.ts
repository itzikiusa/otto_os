// Pure failed-test debug launch mapper for `IdeRunPanel.svelte` (`E-10c`).
// Resolves a failed `IdeTestResult` to either a matching gutter test item
// (`ide.runFromGutter(item, 'debug')`) or a direct `ide.startDebug` launch spec.

import type { IdeGutterItem, IdeTestResult } from '../store/ideTypes.ts';

export interface FailedTestDebugInput {
  test: Pick<IdeTestResult, 'name' | 'suite' | 'file' | 'line'>;
  runner?: string | null;
  targetRoot: string;
  activePath?: string | null;
  activeLanguage?: string | null;
  gutterByPath?: Readonly<Record<string, readonly IdeGutterItem[]>>;
  resolvePath?: (p: string) => string;
}

export interface DirectTestDebugLaunch {
  program: string;
  args: string[];
  cwd: string;
  test_name?: string;
}

export type FailedTestDebugPlan =
  | { kind: 'gutter'; item: IdeGutterItem }
  | { kind: 'direct'; language: string; launch: DirectTestDebugLaunch };

function joinIfRelative(root: string, filePath: string): string {
  const cleanFile = filePath.trim();
  if (!cleanFile) return '';
  if (cleanFile.startsWith('/') || /^[A-Za-z]:[\\/]/.test(cleanFile)) return cleanFile;
  const cleanRoot = root.replace(/\/+$/, '');
  return cleanRoot ? `${cleanRoot}/${cleanFile.replace(/^\.\//, '')}` : cleanFile;
}

export function inferTestDebugLanguage(
  filePath: string | null | undefined,
  runner?: string | null,
  fallback?: string | null,
): string {
  const lower = (filePath ?? '').toLowerCase();
  if (lower.endsWith('.go')) return 'go';
  if (lower.endsWith('.rs')) return 'rust';
  if (lower.endsWith('.py') || lower.endsWith('.pyi')) return 'python';
  if (lower.endsWith('.java')) return 'java';
  if (lower.endsWith('.ts') || lower.endsWith('.tsx') || lower.endsWith('.mts') || lower.endsWith('.cts')) return 'typescript';
  if (lower.endsWith('.js') || lower.endsWith('.jsx') || lower.endsWith('.mjs') || lower.endsWith('.cjs')) return 'javascript';
  if (lower.endsWith('.sh') || lower.endsWith('.bash') || lower.endsWith('.zsh')) return 'shell';

  const r = (runner ?? '').toLowerCase();
  if (r.startsWith('go_')) return 'go';
  if (r.startsWith('cargo_')) return 'rust';
  if (r.startsWith('java_') || r === 'maven' || r === 'gradle') return 'java';
  if (r === 'pytest' || r === 'python') return 'python';
  if (r === 'node' || r === 'npm' || r === 'vitest') return 'javascript';
  if (r === 'shell') return 'shell';

  return (fallback ?? '').trim() || 'go';
}

function findMatchingGutterTest(
  items: readonly IdeGutterItem[] | undefined,
  test: Pick<IdeTestResult, 'name' | 'line'>,
): IdeGutterItem | null {
  if (!items || items.length === 0) return null;
  const name = test.name.trim();
  const baseName = name.includes('/') ? name.slice(0, name.indexOf('/')) : name;

  if (test.line != null && test.line > 0) {
    const byLine = items.find((it) => (it.kind === 'test' || it.kind === 'suite') && it.line === test.line);
    if (byLine) return byLine;
  }
  if (!name) return null;
  return (
    items.find((it) => {
      if (it.kind !== 'test' && it.kind !== 'suite') return false;
      const spec = it.run_spec;
      return (
        it.label === name ||
        it.label === baseName ||
        spec?.debug_test_name === name ||
        spec?.test_name === name ||
        spec?.test_filter === name ||
        spec?.test_name === baseName
      );
    }) ?? null
  );
}

/** Build a debug launch plan for a failed test row, or `null` when it cannot be launched. */
export function planFailedTestDebug(input: FailedTestDebugInput): FailedTestDebugPlan | null {
  const root = input.targetRoot.trim();
  if (!root) return null;
  const rawFile = input.test.file?.trim() ?? '';
  const rawName = input.test.name?.trim() ?? '';
  if (!rawFile && !rawName) return null;

  const resolvedFile = rawFile
    ? (input.resolvePath ? input.resolvePath(rawFile) : joinIfRelative(root, rawFile))
    : '';
  const gutterItems = resolvedFile ? input.gutterByPath?.[resolvedFile] : undefined;
  const matchedGutter = findMatchingGutterTest(gutterItems, input.test);
  if (matchedGutter) return { kind: 'gutter', item: matchedGutter };

  const program = resolvedFile || input.activePath?.trim() || root;
  const language = inferTestDebugLanguage(resolvedFile || input.activePath, input.runner, input.activeLanguage);
  const suite = input.test.suite?.trim() ?? '';
  const testName =
    language === 'java' && suite && rawName && !rawName.includes('#') && !rawName.includes('.')
      ? `${suite}#${rawName}`
      : rawName;

  return {
    kind: 'direct',
    language,
    launch: {
      program,
      args: [],
      cwd: root,
      ...(testName ? { test_name: testName } : {}),
    },
  };
}
