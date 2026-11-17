// Run configuration editor layout: which fields apply to a runner, which of
// those are everyday (shown) and which fold into "Advanced", plus the draft a
// new configuration starts from (the active tab's language and file).

import { defaultRunnerForLanguage, type IdeOpenTab } from '../store/ideTypes';

export type RunConfigField =
  | 'target' | 'command' | 'test_filter' | 'args' | 'cwd'
  | 'build_flags' | 'sdk' | 'env_file' | 'before_launch' | 'inherit_shell_env';

const TEST_RUNNERS = new Set(['go_test', 'java_junit', 'cargo_test', 'pytest', 'vitest', 'maven', 'gradle']);
const BUILD_FLAG_RUNNERS = new Set(['go_test', 'go_run', 'go_build', 'cargo_test', 'cargo_run', 'maven', 'gradle', 'java_main', 'java_junit']);
const NO_SDK_RUNNERS = new Set(['shell', 'custom']);

/** Fields the runner uses at all: everyday ones first, the rest under "Advanced". */
export function runConfigFieldLayout(runner: string): { primary: RunConfigField[]; advanced: RunConfigField[] } {
  const primary: RunConfigField[] = runner === 'custom' ? ['command'] : ['target'];
  if (TEST_RUNNERS.has(runner)) primary.push('test_filter');
  primary.push('args', 'cwd');
  const advanced: RunConfigField[] = [];
  if (BUILD_FLAG_RUNNERS.has(runner)) advanced.push('build_flags');
  if (!NO_SDK_RUNNERS.has(runner)) advanced.push('sdk');
  advanced.push('env_file', 'before_launch', 'inherit_shell_env');
  return { primary, advanced };
}

export interface RunConfigFieldValues {
  target_path: string; command: string; test_filter: string; argsText: string; cwd: string;
  buildFlagsText: string; sdk_path: string; env_file: string; before_launch: string; inherit_shell_env: boolean;
}

/** Whether a field holds something other than its default. */
export function fieldHasValue(field: RunConfigField, v: RunConfigFieldValues): boolean {
  switch (field) {
    case 'target': return Boolean(v.target_path.trim());
    case 'command': return Boolean(v.command.trim());
    case 'test_filter': return Boolean(v.test_filter.trim());
    case 'args': return Boolean(v.argsText.trim());
    case 'cwd': return Boolean(v.cwd.trim());
    case 'build_flags': return Boolean(v.buildFlagsText.trim());
    case 'sdk': return Boolean(v.sdk_path.trim());
    case 'env_file': return Boolean(v.env_file.trim());
    case 'before_launch': return Boolean(v.before_launch.trim());
    case 'inherit_shell_env': return v.inherit_shell_env === false;
  }
}

const ALL_FIELDS: readonly RunConfigField[] = [
  'target', 'command', 'test_filter', 'args', 'cwd', 'build_flags', 'sdk', 'env_file', 'before_launch', 'inherit_shell_env',
];

/** The editor's sections for this draft. A field the runner doesn't use stays
 *  hidden unless it already holds a value (a runner switch never hides data:
 *  it moves to "Advanced"). `advancedOpen` is true when anything there is set. */
export function visibleRunConfigFields(runner: string, values: RunConfigFieldValues): {
  primary: RunConfigField[]; advanced: RunConfigField[]; advancedOpen: boolean;
} {
  const layout = runConfigFieldLayout(runner);
  const used = new Set([...layout.primary, ...layout.advanced]);
  const advanced = [...layout.advanced, ...ALL_FIELDS.filter((f) => !used.has(f) && fieldHasValue(f, values))];
  return { primary: layout.primary, advanced, advancedOpen: advanced.some((f) => fieldHasValue(f, values)) };
}

const TARGET_COPY: Record<string, { label: string; placeholder: string }> = {
  go_test: { label: 'Package', placeholder: './... or ./internal/calc' },
  go_run: { label: 'Package or file', placeholder: './cmd/server' },
  go_build: { label: 'Package', placeholder: './cmd/server' },
  cargo_test: { label: 'Package', placeholder: 'my-crate' },
  cargo_run: { label: 'Binary', placeholder: 'my-app' },
  java_main: { label: 'Main class or file', placeholder: 'com.acme.App' },
  java_junit: { label: 'Test class or file', placeholder: 'com.acme.AppTest' },
  maven: { label: 'Module', placeholder: 'service-core' },
  gradle: { label: 'Project / task path', placeholder: ':app' },
  node: { label: 'Script', placeholder: 'src/index.js' },
  npm: { label: 'npm script', placeholder: 'dev' },
  vitest: { label: 'Test file or folder', placeholder: 'src/lib' },
  pytest: { label: 'Test file or folder', placeholder: 'tests/' },
  python: { label: 'Script', placeholder: 'main.py' },
  shell: { label: 'Script', placeholder: 'scripts/deploy.sh' },
};

export function targetFieldCopy(runner: string): { label: string; placeholder: string } {
  return TARGET_COPY[runner] ?? { label: 'Target', placeholder: './…' };
}

type TabLike = Pick<IdeOpenTab, 'name' | 'path' | 'language'> | null | undefined;

const TEST_FILE = /(_test\.go|^test_.*\.py|_test\.py|\.(test|spec)\.[cm]?[jt]sx?)$/;

/** The runner a new configuration starts with for the active tab. */
export function defaultRunnerForTab(tab: TabLike): string {
  if (!tab) return 'custom';
  if (TEST_FILE.test(tab.name)) {
    if (tab.language === 'go') return 'go_test';
    if (tab.language === 'python') return 'pytest';
    if (['javascript', 'typescript', 'tsx', 'jsx'].includes(tab.language)) return 'vitest';
  }
  return defaultRunnerForLanguage(tab.language);
}

/** Name / runner / target a new configuration is prefilled with. */
export function draftDefaultsForTab(tab: TabLike): { name: string; runner: string; target_path: string } {
  const runner = defaultRunnerForTab(tab);
  if (!tab || runner === 'custom') return { name: '', runner, target_path: '' };
  return { name: tab.name, runner, target_path: tab.path };
}
