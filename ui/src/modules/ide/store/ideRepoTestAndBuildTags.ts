// Per-repository Go build tags (`//go:build` / `// +build`) and per-repository
// `Run All Tests` / `Run All Tests with Coverage` / `Debug All Tests` helpers.
// Strictly scopes test execution and Go build tags to a single repository root
// (never lumping unrelated repos in a multi-repo umbrella project).

import { extractGoBuildTagsFromSource } from '../../../lib/goBuildTags.ts';
import type {
  IdeFsEntry,
  IdeProjectEnvironment,
  IdeProjectRoot,
  IdeRunConfig,
} from './ideTypes.ts';

const VALID_BUILD_TAG_RE = /^[A-Za-z0-9_.-]+$/;

function cleanPath(p: string): string {
  const norm = p.replace(/\\/g, '/').replace(/\/+$/, '');
  return norm || '/';
}

function repoBaseName(repoPath: string): string {
  const clean = cleanPath(repoPath);
  const idx = clean.lastIndexOf('/');
  return idx >= 0 ? clean.slice(idx + 1) || clean : clean;
}

export function normalizeGoBuildTags(
  raw: string | readonly string[] | null | undefined,
): string[] {
  if (!raw) return [];
  const items = typeof raw === 'string' ? raw.split(/[,\s]+/) : raw;
  const seen = new Set<string>();
  const out: string[] = [];
  for (let i = 0; i < items.length; i++) {
    let tok = (items[i] ?? '').trim();
    if (!tok) continue;
    if (tok.startsWith('-tags=')) tok = tok.slice('-tags='.length).trim();
    for (const part of tok.split(/[,\s]+/)) {
      const clean = part.trim();
      if (!clean || !VALID_BUILD_TAG_RE.test(clean) || seen.has(clean)) continue;
      seen.add(clean);
      out.push(clean);
    }
  }
  return out;
}

export function resolveRepoRootForPath(
  primaryRoot: string,
  projectRoots: readonly Pick<IdeProjectRoot, 'path' | 'name'>[],
  candidatePath?: string | null,
): { path: string; name: string } {
  const normPrimary = cleanPath(primaryRoot);
  const fallbackName =
    projectRoots.find((r) => cleanPath(r.path) === normPrimary)?.name ??
    repoBaseName(normPrimary);
  if (!candidatePath) return { path: normPrimary, name: fallbackName };

  const normCandidate = cleanPath(candidatePath);
  let bestPath = '';
  let bestName = '';
  for (let i = 0; i < projectRoots.length; i++) {
    const rp = cleanPath(projectRoots[i].path);
    if (
      (normCandidate === rp || normCandidate.startsWith(`${rp}/`)) &&
      rp.length > bestPath.length
    ) {
      bestPath = rp;
      bestName = projectRoots[i].name || repoBaseName(rp);
    }
  }
  if (bestPath) return { path: bestPath, name: bestName };
  if (normCandidate === normPrimary || normCandidate.startsWith(`${normPrimary}/`)) {
    return { path: normPrimary, name: fallbackName };
  }
  return { path: normPrimary, name: fallbackName };
}

export function resolveRepoGoBuildTags(
  env: IdeProjectEnvironment | null | undefined,
  primaryRoot: string,
  repoRoot: string,
  sourceContent?: string | null,
): string[] {
  const normPrimary = cleanPath(primaryRoot);
  const normRepo = cleanPath(repoRoot);
  const modMap = env?.module_go_build_tags ?? {};

  let bestModLen = -1;
  let modTags: string[] | undefined;
  for (const [rawModRoot, list] of Object.entries(modMap)) {
    const mr = cleanPath(rawModRoot);
    if ((normRepo === mr || normRepo.startsWith(`${mr}/`)) && mr.length > bestModLen) {
      bestModLen = mr.length;
      modTags = list;
    }
  }

  const baseTags =
    modTags !== undefined
      ? modTags
      : normRepo === normPrimary
        ? (env?.go_build_tags ?? [])
        : (env?.go_build_tags ?? []);

  const fileTags = sourceContent ? extractGoBuildTagsFromSource(sourceContent) : [];
  return normalizeGoBuildTags([...baseTags, ...fileTags]);
}

export function mergeGoBuildTagsIntoFlags(
  existingFlags: readonly string[] | undefined,
  tags: readonly string[],
): string[] {
  const base = [...(existingFlags ?? [])];
  const normTags = normalizeGoBuildTags(tags);
  if (normTags.length === 0) return base;

  const collected: string[] = [];
  const stripped: string[] = [];
  for (let i = 0; i < base.length; i++) {
    const f = base[i];
    if (f.startsWith('-tags=') || f.startsWith('--tags=')) {
      collected.push(f.slice(f.indexOf('=') + 1));
    } else if ((f === '-tags' || f === '--tags') && i + 1 < base.length) {
      collected.push(base[i + 1]);
      i += 1;
    } else {
      stripped.push(f);
    }
  }
  const merged = normalizeGoBuildTags([...collected, ...normTags]);
  if (merged.length === 0) return stripped;
  return [...stripped, `-tags=${merged.join(',')}`];
}

export type RepoTestExecutionMode = 'run' | 'coverage' | 'debug';

export type RepoTestExecutionPlan =
  | {
      kind: 'run';
      repoRoot: string;
      repoName: string;
      adhoc: Partial<IdeRunConfig> & { name: string; runner: string };
    }
  | {
      kind: 'debug';
      repoRoot: string;
      repoName: string;
      language: string;
      launch: {
        program: string;
        args: string[];
        build_flags: string[];
        cwd: string;
      };
    };

type RepoTestEcosystem = 'go' | 'rust' | 'maven' | 'gradle' | 'vitest' | 'npm' | 'pytest';

export function detectRepoTestEcosystem(opts: {
  repoRoot: string;
  runConfigs: readonly IdeRunConfig[];
  detectedConfigs: readonly IdeRunConfig[];
  dirEntries: readonly Pick<IdeFsEntry, 'name' | 'path'>[];
  openTabs: readonly { path: string; language?: string }[];
}): RepoTestEcosystem {
  const normRepo = cleanPath(opts.repoRoot);
  const entryNames = new Set(opts.dirEntries.map((e) => e.name.toLowerCase()));
  if (entryNames.has('go.mod') || entryNames.has('go.work')) return 'go';
  if (entryNames.has('cargo.toml')) return 'rust';
  if (entryNames.has('pom.xml')) return 'maven';
  if (entryNames.has('build.gradle') || entryNames.has('build.gradle.kts')) return 'gradle';
  if (
    entryNames.has('vitest.config.ts') ||
    entryNames.has('vitest.config.js') ||
    entryNames.has('vitest.config.mts')
  ) {
    return 'vitest';
  }
  if (entryNames.has('package.json')) return 'npm';
  if (
    entryNames.has('pyproject.toml') ||
    entryNames.has('pytest.ini') ||
    entryNames.has('requirements.txt')
  ) {
    return 'pytest';
  }

  const scopedConfigs = [...opts.runConfigs, ...opts.detectedConfigs].filter((c) => {
    const cwd = c.cwd ? cleanPath(c.cwd) : '';
    const tp = c.target_path ? cleanPath(c.target_path) : '';
    return (
      cwd === normRepo ||
      cwd.startsWith(`${normRepo}/`) ||
      tp === normRepo ||
      tp.startsWith(`${normRepo}/`)
    );
  });
  for (let i = 0; i < scopedConfigs.length; i++) {
    const r = scopedConfigs[i].runner;
    if (r.startsWith('go_')) return 'go';
    if (r.startsWith('cargo_')) return 'rust';
    if (r === 'maven' || r === 'java_junit') return 'maven';
    if (r === 'gradle') return 'gradle';
    if (r === 'vitest') return 'vitest';
    if (r === 'npm' || r === 'node') return 'npm';
    if (r === 'pytest' || r === 'python') return 'pytest';
  }

  for (let i = 0; i < opts.openTabs.length; i++) {
    const t = opts.openTabs[i];
    const tp = cleanPath(t.path);
    if (tp !== normRepo && !tp.startsWith(`${normRepo}/`)) continue;
    const lang = (t.language ?? '').toLowerCase();
    if (lang === 'go' || tp.endsWith('.go')) return 'go';
    if (lang === 'rust' || tp.endsWith('.rs')) return 'rust';
    if (lang === 'java' || tp.endsWith('.java')) return 'maven';
    if (lang === 'python' || tp.endsWith('.py')) return 'pytest';
    if (
      lang === 'typescript' ||
      lang === 'javascript' ||
      /\.(?:ts|tsx|js|jsx|svelte)$/.test(tp)
    ) {
      return 'vitest';
    }
  }
  return 'go';
}

export function resolveRepoTestPlan(opts: {
  repoRoot: string;
  repoName?: string;
  mode: RepoTestExecutionMode;
  runConfigs: readonly IdeRunConfig[];
  detectedConfigs: readonly IdeRunConfig[];
  dirEntries: readonly Pick<IdeFsEntry, 'name' | 'path'>[];
  openTabs: readonly { path: string; language?: string }[];
  goBuildTags: readonly string[];
  inheritShellEnv?: boolean;
}): RepoTestExecutionPlan {
  const repoRoot = cleanPath(opts.repoRoot);
  const repoName = opts.repoName || repoBaseName(repoRoot);
  const eco = detectRepoTestEcosystem(opts);
  const goFlags = eco === 'go' ? mergeGoBuildTagsIntoFlags([], opts.goBuildTags) : [];

  if (opts.mode === 'debug') {
    if (eco === 'go') {
      return {
        kind: 'debug',
        repoRoot,
        repoName,
        language: 'go',
        launch: {
          program: `${repoRoot}/...`,
          args: [],
          build_flags: goFlags,
          cwd: repoRoot,
        },
      };
    }
    if (eco === 'rust') {
      return {
        kind: 'debug',
        repoRoot,
        repoName,
        language: 'rust',
        launch: { program: repoRoot, args: [], build_flags: [], cwd: repoRoot },
      };
    }
    if (eco === 'maven' || eco === 'gradle') {
      return {
        kind: 'debug',
        repoRoot,
        repoName,
        language: 'java',
        launch: {
          program: repoRoot,
          args: eco === 'maven' ? ['-Dmaven.surefire.debug', 'test'] : ['test', '--debug-jvm'],
          build_flags: [],
          cwd: repoRoot,
        },
      };
    }
    if (eco === 'pytest') {
      return {
        kind: 'debug',
        repoRoot,
        repoName,
        language: 'python',
        launch: { program: repoRoot, args: ['-m', 'pytest'], build_flags: [], cwd: repoRoot },
      };
    }
    return {
      kind: 'debug',
      repoRoot,
      repoName,
      language: 'typescript',
      launch: { program: repoRoot, args: [], build_flags: [], cwd: repoRoot },
    };
  }

  const isCoverage = opts.mode === 'coverage';
  const titlePrefix = isCoverage ? 'Test All with Coverage' : 'Test All';
  const name = `${titlePrefix} (${repoName})`;
  const common = {
    id: `repo-${opts.mode}-${repoName}`,
    name,
    cwd: repoRoot,
    coverage: isCoverage,
    env_overrides: {},
    inherit_shell_env: opts.inheritShellEnv ?? true,
  };

  switch (eco) {
    case 'go':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: {
          ...common,
          runner: 'go_test',
          target_path: './...',
          args: ['-v'],
          build_flags: goFlags,
        },
      };
    case 'rust':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: { ...common, runner: 'cargo_test', args: [] },
      };
    case 'maven':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: { ...common, runner: 'maven', args: ['test'] },
      };
    case 'gradle':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: { ...common, runner: 'gradle', args: ['test'] },
      };
    case 'vitest':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: { ...common, runner: 'vitest', args: ['run'] },
      };
    case 'npm':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: { ...common, runner: 'npm', args: ['test'] },
      };
    case 'pytest':
      return {
        kind: 'run',
        repoRoot,
        repoName,
        adhoc: { ...common, runner: 'pytest', args: [] },
      };
  }
}
