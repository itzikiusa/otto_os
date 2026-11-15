// Per-repository Go build tag mutation, per-repository test execution, and
// production target launch confirmation helpers extracted from `IdeStore`.

import { api } from '../../../lib/api/client';
import { normalizeRootPath } from '../shell/ideProjectHelpers';
import {
  normalizeGoBuildTags,
  resolveRepoGoBuildTags,
  resolveRepoRootForPath,
  resolveRepoTestPlan,
  type RepoTestExecutionMode,
} from './ideRepoTestAndBuildTags';
import type {
  IdeEnvVar,
  IdeFsEntry,
  IdeOpenTab,
  IdeProjectEnvironment,
  IdeProjectRoot,
  IdeRunConfig,
  IdeTargetSpec,
} from './ideTypes';

export async function confirmIdeTargetProdLaunch(
  target: IdeTargetSpec,
  projectEnv: IdeProjectEnvironment | null,
  skipProdConfirm = false,
): Promise<{ allowed: boolean; confirmProdTargets: boolean }> {
  if (skipProdConfirm) return { allowed: true, confirmProdTargets: true };
  if (target.kind === 'local' && projectEnv?.network_bridge?.enabled) {
    const { confirmProdNetworkBridge } = await import('../shell/networkBridgeGuard.svelte');
    const allowed = await confirmProdNetworkBridge(
      projectEnv.network_bridge,
      'launch',
      target.workspace_id,
    );
    return { allowed, confirmProdTargets: allowed };
  }
  if (target.kind === 'ssh' && target.connection_id) {
    const { confirmProdSshTarget } = await import('../shell/networkBridgeGuard.svelte');
    const allowed = await confirmProdSshTarget(target.connection_id, target.workspace_id);
    return { allowed, confirmProdTargets: allowed };
  }
  return { allowed: true, confirmProdTargets: false };
}

export interface RepoActionsHost {
  readonly target: IdeTargetSpec;
  readonly projectRoots: IdeProjectRoot[];
  readonly projectEnv: IdeProjectEnvironment | null;
  readonly activePath: string | null;
  readonly treeRootEntries: IdeFsEntry[];
  childrenByDir: Record<string, IdeFsEntry[]>;
  readonly runConfigs: IdeRunConfig[];
  readonly detectedConfigs: IdeRunConfig[];
  readonly tabs: IdeOpenTab[];
  getBuffer(path: string): string;
  loadGutter(path: string, content?: string): Promise<void>;
  saveEnvironment(
    patch: Partial<IdeProjectEnvironment> & {
      env_vars?: IdeEnvVar[];
      confirm_prod_targets?: boolean;
    },
  ): Promise<boolean>;
  startRun(opts?: {
    configId?: string;
    adhoc?: Partial<IdeRunConfig>;
    keepTestcontainers?: boolean;
    coverage?: boolean;
    skipProdConfirm?: boolean;
  }): Promise<void>;
  startDebug(opts?: {
    configId?: string;
    language?: string;
    skipProdConfirm?: boolean;
    launch?: {
      program: string;
      args?: string[];
      build_flags?: string[];
      cwd?: string;
      test_name?: string;
      attach_port?: number;
      env?: Record<string, string>;
    };
  }): Promise<void>;
}

export async function performSetRepoGoBuildTags(
  host: RepoActionsHost,
  pathOrRoot: string,
  tags: readonly string[] | string,
): Promise<boolean> {
  const repo = resolveRepoRootForPath(host.target.root, host.projectRoots, pathOrRoot);
  const cleaned = normalizeGoBuildTags(tags);
  const nextModMap: Record<string, string[]> = {
    ...(host.projectEnv?.module_go_build_tags ?? {}),
    [repo.path]: cleaned,
  };
  const isPrimary = normalizeRootPath(repo.path) === normalizeRootPath(host.target.root);
  let confirmProd = false;
  if (host.target.kind === 'local' && host.projectEnv?.network_bridge?.enabled) {
    const { bridgeEnvCache } = await import('../shell/networkBridgeGuard.svelte');
    await bridgeEnvCache.ensureLoaded(host.target.workspace_id);
    confirmProd = bridgeEnvCache.isProdBridge(host.projectEnv.network_bridge);
  }
  const ok = await host.saveEnvironment({
    module_go_build_tags: nextModMap,
    ...(isPrimary ? { go_build_tags: cleaned } : {}),
    ...(confirmProd ? { confirm_prod_targets: true } : {}),
  });
  if (ok && typeof window !== 'undefined') {
    window.dispatchEvent(
      new CustomEvent('otto:ide-reconnect-lsp', { detail: { rootPath: repo.path } }),
    );
    if (host.activePath) void host.loadGutter(host.activePath, host.getBuffer(host.activePath));
  }
  return ok;
}

export async function performRunRepoTests(
  host: RepoActionsHost,
  pathOrRoot?: string | null,
  mode: RepoTestExecutionMode = 'run',
): Promise<void> {
  const repo = resolveRepoRootForPath(
    host.target.root,
    host.projectRoots,
    pathOrRoot ?? host.activePath,
  );
  const isPrimary = normalizeRootPath(repo.path) === normalizeRootPath(host.target.root);
  let dirEntries = isPrimary ? host.treeRootEntries : (host.childrenByDir[repo.path] ?? []);
  if (!isPrimary && dirEntries.length === 0 && host.target.kind === 'local' && repo.path) {
    try {
      const q = new URLSearchParams({ kind: 'local', root: repo.path, path: '', show_hidden: 'false' }).toString();
      const resp = await api.get<{ entries?: IdeFsEntry[] }>(`/ide/fs/tree?${q}`);
      if (resp.entries?.length) {
        dirEntries = resp.entries;
        host.childrenByDir = { ...host.childrenByDir, [repo.path]: resp.entries };
      }
    } catch { /* fallback to openTabs / runConfigs */ }
  }
  const plan = resolveRepoTestPlan({
    repoRoot: repo.path,
    repoName: repo.name,
    mode,
    runConfigs: host.runConfigs,
    detectedConfigs: host.detectedConfigs,
    dirEntries,
    openTabs: host.tabs,
    goBuildTags: resolveRepoGoBuildTags(host.projectEnv, host.target.root, repo.path),
    inheritShellEnv: host.projectEnv?.inherit_shell_env ?? true,
  });
  if (plan.kind === 'debug') {
    await host.startDebug({ language: plan.language, launch: plan.launch });
  } else {
    await host.startRun({ adhoc: plan.adhoc, coverage: mode === 'coverage' });
  }
}
