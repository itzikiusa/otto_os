// Toolchains, Shell Environment, Project Environments, Run Configurations,
// and DAP Debugger helpers for `IdeStore`.

import { api, ApiError } from '../../../lib/api/client';
import { idePreferences } from '../../../lib/idePreferences.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { pollWhileVisible, type Poller } from '../../../lib/poll';
import { auth } from '../../../lib/stores/auth.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { ideCollabNotices } from './ideCollabNotices.svelte';
import {
  attachDebugSessionStream,
  normalizeIncomingDebugSnapshot,
  refreshWatchValues,
  syncSnapshotBreakpointsToStore,
} from './ideDebugSnapshotSync';
import {
  mergeGoBuildTagsIntoFlags,
  resolveRepoGoBuildTags,
  resolveRepoRootForPath,
} from './ideRepoTestAndBuildTags';
export {
  attachDebugSessionStream,
  normalizeIncomingDebugSnapshot,
  refreshWatchValues,
  syncSnapshotBreakpointsToStore,
};
import {
  collabGuestPost, defaultRunnerForLanguage, IdeCollabHttpError, ideTargetKey, streamIdeSse, type IdeBottomTab,
  type IdeBreakpoint, type IdeCollabRoom, type IdeDebugSessionSnapshot, type IdeDetectedSdk,
  type IdeEnvVar, type IdeGutterItem, type IdeInstalledSdk, type IdeInstallJob,
  type IdeOpenTab, type IdeProjectEnvironment, type IdeProjectRoot, type IdePythonVenv,
  type IdeReplEntry, type IdeRunConfig, type IdeRunState, type IdeShellEnvResp,
  type IdeTargetSpec, type IdeToolchainEcosystem, type IdeVariable, type IdeWatch,
} from './ideTypes';

export interface DebugAndEnvHost {
  target: IdeTargetSpec;
  readonly targetPayload: IdeTargetSpec;
  readonly projectRoots: IdeProjectRoot[];
  targetOs: string;
  targetArch: string;
  activePath: string | null;
  readonly activeTab: IdeOpenTab | null;
  toolchains: IdeToolchainEcosystem[];
  detectedSdks: IdeDetectedSdk[];
  toolchainsLoading: boolean;
  toolchainsError: string | null;
  installJobs: IdeInstallJob[];
  projectEnv: IdeProjectEnvironment | null;
  effectiveEnv: Record<string, string>;
  shellEnv: IdeShellEnvResp | null;
  shellEnvLoading: boolean;
  shellEnvAttempted?: boolean;
  shellEnvError?: string | null;
  keepTestcontainersAlive?: boolean;
  pythonVenvs: IdePythonVenv[];
  dotenvFiles: string[];
  runConfigs: IdeRunConfig[];
  detectedConfigs: IdeRunConfig[];
  selectedConfigId: string | null;
  envLoading: boolean;
  envError: string | null;
  activeRun: IdeRunState | null;
  runsHistory: IdeRunState[];
  readonly allBreakpoints: IdeBreakpoint[];
  readonly effectiveBreakpoints: IdeBreakpoint[];
  breakOnExceptions: boolean;
  activeDebug: IdeDebugSessionSnapshot | null;
  expandedVariables: Record<number, IdeVariable[]>;
  watches: IdeWatch[];
  replEntries: IdeReplEntry[];
  debugLoading: boolean;
  debugStreamActive?: boolean;
  lastDebugLaunch?: {
    configId?: string;
    language?: string;
    launch?: {
      program: string;
      args?: string[];
      build_flags?: string[];
      cwd?: string;
      test_name?: string;
      attach_port?: number;
      env?: Record<string, string>;
      env_file?: string | null;
      inherit_shell_env?: boolean;
      sdk_path?: string | null;
    };
  } | null;
  recordRunLaunch?(runId: string, memo: { configId?: string; adhoc?: Partial<IdeRunConfig>; keepTestcontainers?: boolean; coverage?: boolean }): void;
  activeMavenProfiles: string[];
  activeSpringProfiles: string[];
  buildSkipTests: boolean;
  buildOffline: boolean;
  collabIsGuest: boolean;
  collabRoom: IdeCollabRoom | null;
  collabPeerId: string | null;
  collabPeerSecret: string | null;
  collabGuestToken: string | null;
  targetQuery(extra?: Record<string, string | boolean | null | undefined>): string;
  openBottomTab(tab: IdeBottomTab): void;
  openFile(path: string, opts?: { line?: number; col?: number; preview?: boolean }): Promise<void>;
  syncAdditionalRootsFromEnv?(roots: string[]): void;
}

export function languageFromRunner(runner: string | undefined, fallback: string): string {
  if (!runner) return fallback;
  if (runner.startsWith('go_')) return 'go';
  if (runner.startsWith('cargo_')) return 'rust';
  if (runner.startsWith('java_') || runner === 'maven' || runner === 'gradle') return 'java';
  if (runner === 'pytest' || runner === 'python') return 'python';
  if (runner === 'node' || runner === 'npm' || runner === 'vitest') return 'javascript';
  if (runner === 'shell') return 'shell';
  return fallback;
}

export function mergeRunConfigs(saved: IdeRunConfig[], detected: IdeRunConfig[]): IdeRunConfig[] {
  const explicit = saved.filter((c) => !c.auto_detected);
  const autoFromSaved = saved.filter((c) => c.auto_detected);
  const seenIds = new Set(explicit.map((c) => c.id));
  const seenNames = new Set(explicit.map((c) => `${c.runner}:${c.name.toLowerCase()}`));
  const mergedAuto: IdeRunConfig[] = [];
  for (const item of [...autoFromSaved, ...detected]) {
    const key = `${item.runner}:${item.name.toLowerCase()}`;
    if (seenIds.has(item.id) || seenNames.has(key)) continue;
    seenIds.add(item.id);
    seenNames.add(key);
    mergedAuto.push({ ...item, auto_detected: true });
  }
  return [...explicit, ...mergedAuto];
}

export async function fetchToolchains(
  host: DebugAndEnvHost,
  onEnsureJobs: () => void,
  opts: { includeCatalog?: boolean } = {},
): Promise<void> {
  if (!host.target.root) return;
  host.toolchainsLoading = true;
  host.toolchainsError = null;
  try {
    const q = host.targetQuery({ include_catalog: opts.includeCatalog ? 'true' : undefined });
    const resp = await api.long.get<{
      target_os: string; target_arch: string; ecosystems: IdeToolchainEcosystem[];
      detected?: IdeDetectedSdk[]; jobs: IdeInstallJob[];
    }>(`/ide/toolchains?${q}`);
    if (resp.target_os) host.targetOs = resp.target_os;
    if (resp.target_arch) host.targetArch = resp.target_arch;
    const ecosystems = resp.ecosystems ?? [];
    host.toolchains = ecosystems;
    const rawDetected: IdeDetectedSdk[] = [
      ...(resp.detected ?? []),
      ...ecosystems.flatMap((eco) =>
        (eco.detected ?? []).map((d) => ({ ...d, ecosystem: d.ecosystem || eco.ecosystem })),
      ),
    ];
    const seen = new Set<string>();
    const normalizedDetected: IdeDetectedSdk[] = [];
    for (let i = 0; i < rawDetected.length; i++) {
      const d = rawDetected[i];
      const p = (d.path || d.home_path || '').trim();
      if (!p) continue;
      const key = `${d.ecosystem}:${p}`;
      if (seen.has(key)) continue;
      seen.add(key);
      normalizedDetected.push({
        ...d,
        name: d.name || d.label || `${d.ecosystem.toUpperCase()} ${d.version || ''}`.trim(),
        path: p,
        home_path: d.home_path || p,
      });
    }
    host.detectedSdks = normalizedDetected;
    host.installJobs = resp.jobs ?? [];
    onEnsureJobs();
  } catch (e) { host.toolchainsError = loadErrorText(e); }
  finally { host.toolchainsLoading = false; }
}

export async function registerLocalToolchain(
  host: DebugAndEnvHost,
  ecosystem: string,
  sdkPath: string,
): Promise<IdeInstalledSdk | null> {
  const clean = sdkPath.trim();
  if (!clean) return null;
  try {
    const resp = await api.post<{ sdk: IdeInstalledSdk }>('/ide/toolchains/local', {
      target: host.targetPayload,
      ecosystem,
      path: clean,
    });
    const sdk = resp.sdk;
    host.toolchains = host.toolchains.map((eco) =>
      eco.ecosystem === ecosystem
        ? { ...eco, installed: [sdk, ...eco.installed.filter((x) => x.path !== sdk.path)] }
        : eco,
    );
    toasts.success(`Added local ${ecosystem.toUpperCase()} SDK`, sdk.version || clean);
    return sdk;
  } catch (e) {
    toastError('Couldn’t register local SDK', e);
    return null;
  }
}

export async function fetchShellEnv(host: DebugAndEnvHost, reload = false): Promise<IdeShellEnvResp | null> {
  host.shellEnvAttempted = true;
  if (!auth.isRoot) return null;
  host.shellEnvLoading = true;
  host.shellEnvError = null;
  try {
    const resp = reload
      ? await api.post<IdeShellEnvResp>('/ide/shell-env/reload', { target: host.targetPayload })
      : await api.get<IdeShellEnvResp>(`/ide/shell-env?${host.targetQuery()}`);
    const normalized: IdeShellEnvResp = {
      ...resp,
      shell: resp.shell || '/bin/zsh',
      vars: resp.vars ?? resp.env ?? {},
    };
    host.shellEnv = normalized;
    host.shellEnvError = null;
    if (reload) toasts.success('Reloaded shell environment', `${Object.keys(normalized.vars).length} variables`);
    return normalized;
  } catch (e) {
    host.shellEnvError = loadErrorText(e);
    if (reload) toastError('Couldn’t reload shell environment', e);
    return null;
  } finally {
    host.shellEnvLoading = false;
  }
}

export function startJobsPoller(host: DebugAndEnvHost, prev: Poller | null, onDone: () => void): Poller | null {
  if (!host.installJobs.some((j) => j.status === 'running')) { prev?.stop(); return null; }
  if (prev) return prev;
  const poller = pollWhileVisible(async (signal) => {
    const running = host.installJobs.filter((j) => j.status === 'running');
    if (running.length === 0) { poller.stop(); return; }
    let finishedAny = false;
    for (const job of running) {
      try {
        const resp = await api.bg.get<{ job: IdeInstallJob }>(`/ide/toolchains/jobs/${encodeURIComponent(job.id)}`, signal);
        host.installJobs = host.installJobs.map((j) => (j.id === resp.job.id ? resp.job : j));
        if (resp.job.status !== 'running') {
          finishedAny = true;
          if (resp.job.status === 'error') {
            toasts.error('Couldn’t install toolchain', resp.job.error || 'Installation exited with an error');
          }
        }
      } catch (e) {
        if (signal.aborted) return;
        if (e instanceof ApiError && e.status === 404) {
          const msg = 'Install job is no longer available on the daemon';
          host.installJobs = host.installJobs.map((j) =>
            j.id === job.id ? { ...j, status: 'error', error: msg } : j,
          );
          finishedAny = true;
          toasts.error('Couldn’t install toolchain', msg);
        }
      }
    }
    if (finishedAny) onDone();
  }, { ms: 1200, immediate: false });
  return poller;
}

export async function fetchEnvironments(host: DebugAndEnvHost): Promise<void> {
  if (!host.target.root) return;
  const key = ideTargetKey(host.target);
  const stale = (): boolean => ideTargetKey(host.target) !== key;
  host.envLoading = true;
  host.envError = null;
  try {
    const resp = await api.get<{
      env: IdeProjectEnvironment;
      python_venvs: IdePythonVenv[];
      dotenv_files: string[];
      configs: IdeRunConfig[];
      detected_configs?: IdeRunConfig[];
      effective_env?: Record<string, string>;
    }>(`/ide/environments?${host.targetQuery()}`);
    // The project switched while this was in flight: its run configs belong to the old one.
    if (stale()) return;
    host.projectEnv = resp.env;
    host.effectiveEnv = resp.effective_env ?? {};
    if (resp.env?.additional_roots?.length) {
      host.syncAdditionalRootsFromEnv?.(resp.env.additional_roots);
    }
    host.pythonVenvs = resp.python_venvs ?? [];
    host.dotenvFiles = resp.dotenv_files ?? [];
    const backendDetected = resp.detected_configs ?? [];
    host.detectedConfigs = backendDetected.length > 0 ? backendDetected : host.detectedConfigs;
    const combined = mergeRunConfigs(resp.configs ?? [], host.detectedConfigs);
    host.runConfigs = combined;
    const explicitFirst = combined.find((c) => !c.auto_detected);
    const hasCurrent = host.selectedConfigId && combined.some((c) => c.id === host.selectedConfigId);
    if (!hasCurrent || (host.selectedConfigId?.startsWith('auto:') && explicitFirst)) {
      host.selectedConfigId = explicitFirst?.id ?? combined[0]?.id ?? null;
    }
  } catch (e) { if (!stale()) host.envError = loadErrorText(e); }
  finally { if (!stale()) host.envLoading = false; }
}

export async function persistEnvironment(
  host: DebugAndEnvHost,
  patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[]; confirm_prod_targets?: boolean },
): Promise<boolean> {
  try {
    // PUT replaces the whole environment: never merge a one-field patch onto an unloaded env
    // (that would send `env_vars: []` and wipe the stored vars, secrets and SDK homes).
    if (!host.projectEnv) await fetchEnvironments(host);
    if (!host.projectEnv) throw new Error(host.envError ?? 'Project environment isn’t loaded yet.');
    const { confirm_prod_targets, ...envPatch } = patch;
    const merged: IdeProjectEnvironment = { inherit_shell_env: true, ...host.projectEnv, ...envPatch };
    const resp = await api.put<{ env: IdeProjectEnvironment }>('/ide/environments', {
      target: host.targetPayload,
      ...merged,
      ...(confirm_prod_targets ? { confirm_prod_targets: true } : {}),
    });
    host.projectEnv = resp.env;
    toasts.success('Saved project environment');
    return true;
  } catch (e) { toastError('Couldn’t save environment', e); return false; }
}

export async function upsertRunConfig(host: DebugAndEnvHost, cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> {
  try {
    const cleanId = cfg.id && !cfg.id.startsWith('auto:') ? cfg.id : undefined;
    const existing = cleanId ? host.runConfigs.find((c) => c.id === cleanId) : undefined;
    const resp = await api.post<{ config?: IdeRunConfig } & Partial<IdeRunConfig>>('/ide/run-configs', {
      target: host.targetPayload,
      id: cleanId,
      name: cfg.name,
      runner: cfg.runner,
      command: cfg.command ?? null,
      args: cfg.args ?? [],
      build_flags: cfg.build_flags ?? [],
      cwd: cfg.cwd ?? null,
      env_overrides: cfg.env_overrides ?? {},
      env_file: cfg.env_file ?? null,
      inherit_shell_env: cfg.inherit_shell_env ?? true,
      sdk_path: cfg.sdk_path ?? null,
      toolchain_id: cfg.toolchain_id ?? null,
      compound_configs: cfg.compound_configs ?? [],
      auto_detected: false,
      before_launch: cfg.before_launch ?? null,
      target_path: cfg.target_path ?? null,
      test_filter: cfg.test_filter ?? null,
      maven_profiles: cfg.maven_profiles ?? existing?.maven_profiles ?? [],
      spring_profiles: cfg.spring_profiles ?? existing?.spring_profiles ?? [],
      skip_tests: cfg.skip_tests ?? existing?.skip_tests ?? false,
      offline: cfg.offline ?? existing?.offline ?? false,
      coverage: cfg.coverage ?? existing?.coverage ?? false,
    });
    const saved = (resp.config ?? resp) as IdeRunConfig;
    await fetchEnvironments(host);
    if (saved?.id) host.selectedConfigId = saved.id;
    return saved;
  } catch (e) { toastError('Couldn’t save run configuration', e); return null; }
}

function resolveModuleSdkOverride(host: DebugAndEnvHost, candidatePath?: string | null): string | null {
  if (!candidatePath) return null;
  const prefsMap = idePreferences.current.moduleSdkMap ?? {};
  const envMap = host.projectEnv?.module_sdks;
  const map = envMap && Object.keys(envMap).length > 0 ? { ...prefsMap, ...envMap } : prefsMap;
  let bestLen = 0;
  let bestSdk: string | null = null;
  for (const [modRoot, sdkPath] of Object.entries(map)) {
    if (sdkPath && (candidatePath === modRoot || candidatePath.startsWith(`${modRoot}/`)) && modRoot.length > bestLen) {
      bestLen = modRoot.length;
      bestSdk = sdkPath;
    }
  }
  return bestSdk;
}

export async function launchIdeRun(
  host: DebugAndEnvHost,
  opts: {
    configId?: string;
    adhoc?: Partial<IdeRunConfig>;
    keepTestcontainers?: boolean;
    coverage?: boolean;
    confirmProdTargets?: boolean;
  },
  prevPoller: Poller | null,
  prevSseCtl: AbortController | null = null,
): Promise<{ poller: Poller | null; sseCtl: AbortController | null }> {
  if (!host.target.root) return { poller: prevPoller, sseCtl: prevSseCtl };
  const keepTc = Boolean(opts.keepTestcontainers || host.keepTestcontainersAlive);
  const wantCoverage = Boolean(opts.coverage || opts.adhoc?.coverage);
  const rawId = opts.configId ?? (opts.adhoc ? undefined : host.selectedConfigId ?? undefined);
  const selectedCfg = rawId ? host.runConfigs.find((c) => c.id === rawId) : undefined;
  const isClientAuto = Boolean(rawId?.startsWith('auto:') || selectedCfg?.auto_detected);
  const cfgId = isClientAuto || keepTc || wantCoverage ? undefined : rawId;
  const rawAdhoc = opts.adhoc ?? ((isClientAuto || keepTc || wantCoverage) && selectedCfg ? selectedCfg : (!cfgId && host.activeTab ? {
    id: 'adhoc', name: `Run ${host.activeTab.name}`, runner: defaultRunnerForLanguage(host.activeTab.language),
    args: [], env_overrides: {}, inherit_shell_env: host.projectEnv?.inherit_shell_env ?? true, target_path: host.activeTab.path,
  } : undefined));
  const candidatePath = rawAdhoc?.target_path || rawAdhoc?.cwd || selectedCfg?.cwd || host.activePath;
  const modSdk = resolveModuleSdkOverride(host, candidatePath);
  const effectiveRunner = rawAdhoc?.runner ?? selectedCfg?.runner ?? '';
  const repoInfo = resolveRepoRootForPath(host.target.root, host.projectRoots, candidatePath);
  const openSourceForGo =
    effectiveRunner.startsWith('go_') && host.activeTab && (rawAdhoc?.target_path === host.activeTab.path || (!rawAdhoc?.target_path && !selectedCfg))
      ? host.activeTab.content
      : null;
  const goTags = effectiveRunner.startsWith('go_')
    ? resolveRepoGoBuildTags(host.projectEnv, host.target.root, repoInfo.path, openSourceForGo)
    : [];
  const springEnv: Record<string, string> = host.activeSpringProfiles.length > 0
    ? { SPRING_PROFILES_ACTIVE: host.activeSpringProfiles.join(',') }
    : {};
  const tcEnv: Record<string, string> = keepTc ? { TESTCONTAINERS_RYUK_DISABLED: 'true' } : {};
  const adhoc = rawAdhoc
    ? {
        ...rawAdhoc,
        build_flags: goTags.length > 0 ? mergeGoBuildTagsIntoFlags(rawAdhoc.build_flags, goTags) : rawAdhoc.build_flags,
        coverage: wantCoverage || rawAdhoc.coverage,
        sdk_path: rawAdhoc.sdk_path ?? modSdk ?? null,
        env_overrides: { ...springEnv, ...tcEnv, ...(rawAdhoc.env_overrides ?? {}) },
      }
    : undefined;
  try {
    host.openBottomTab('run');
    const resp = await api.post<{ run: IdeRunState }>('/ide/runs', {
      target: host.targetPayload,
      project_folders: host.projectRoots.map((r) => r.path),
      config_id: cfgId,
      adhoc,
      maven_profiles: host.activeMavenProfiles,
      spring_profiles: host.activeSpringProfiles,
      skip_tests: host.buildSkipTests,
      offline: host.buildOffline,
      coverage: wantCoverage || undefined,
      ...(opts.confirmProdTargets ? { confirm_prod_targets: true } : {}),
    });
    host.activeRun = resp.run;
    host.recordRunLaunch?.(resp.run.id, {
      configId: cfgId,
      adhoc,
      keepTestcontainers: keepTc,
      coverage: wantCoverage || undefined,
    });
    prevPoller?.stop();
    prevSseCtl?.abort();
    const sseCtl = new AbortController();
    let sseActive = false;
    let pollerRef: Poller | null = null;
    const stopBoth = (): void => { sseCtl.abort(); pollerRef?.stop(); };
    const applyRunState = (incoming: IdeRunState): void => {
      const boundedOutput = incoming.output.length > 2000 ? incoming.output.slice(-2000) : incoming.output;
      const liveRun: IdeRunState = boundedOutput === incoming.output ? incoming : { ...incoming, output: boundedOutput };
      host.activeRun = liveRun;
      if (liveRun.status !== 'running') {
        const archived = liveRun.output.length > 200 ? { ...liveRun, output: liveRun.output.slice(-200) } : liveRun;
        host.runsHistory = [archived, ...host.runsHistory.filter((x) => x.id !== archived.id)].slice(0, 8);
        stopBoth();
      }
    };
    void streamIdeSse(`/ide/runs/${encodeURIComponent(resp.run.id)}/events`, {}, sseCtl.signal, (raw) => {
      try {
        const parsed = JSON.parse(raw) as { run?: IdeRunState } & Partial<IdeRunState>;
        const runSnap = (parsed.run ?? parsed) as IdeRunState;
        if (runSnap?.id) {
          sseActive = true;
          applyRunState(runSnap);
        }
      } catch { /* heartbeat */ }
    }, () => { sseActive = true; }).catch(() => {}).finally(() => { sseActive = false; });
    pollerRef = pollWhileVisible(async (signal) => {
      if (sseActive && host.activeRun?.status === 'running') return;
      try {
        const r = await api.bg.get<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(resp.run.id)}`, signal);
        applyRunState(r.run);
      } catch (e) {
        if (signal.aborted) return;
        if (e instanceof ApiError && e.status === 404) {
          if (host.activeRun && host.activeRun.id === resp.run.id) {
            const evicted: IdeRunState = { ...host.activeRun, status: 'failed' };
            host.activeRun = evicted;
            const archived = evicted.output.length > 200 ? { ...evicted, output: evicted.output.slice(-200) } : evicted;
            host.runsHistory = [archived, ...host.runsHistory.filter((x) => x.id !== archived.id)].slice(0, 8);
          }
          toasts.error('Run ended unexpectedly', 'Run state is no longer available on the daemon.');
          stopBoth();
        }
      }
    }, { ms: 750, floorMs: 500, immediate: false });
    return { poller: pollerRef, sseCtl };
  } catch (e) {
    toastError('Couldn’t start run', e);
    return { poller: prevPoller, sseCtl: prevSseCtl };
  }
}

export function launchGutterAction(
  host: DebugAndEnvHost,
  item: IdeGutterItem,
  mode: 'run' | 'debug' | 'run_keep_tc',
  onRun: (adhoc: Partial<IdeRunConfig>, keepTestcontainers?: boolean) => void,
  onDebug: (opts: { language: string; launch: { program: string; args: string[]; build_flags?: string[]; cwd: string; test_name?: string } }) => void,
): void {
  const spec = item.run_spec ?? {};
  const filePath = spec.target_path || spec.program || host.activePath || '';
  const lang = spec.language || item.language || host.activeTab?.language || 'go';
  const repoInfo = resolveRepoRootForPath(host.target.root, host.projectRoots, filePath || spec.cwd);
  const openDoc = host.activeTab && (!filePath || host.activeTab.path === filePath) ? host.activeTab.content : null;
  const goFlags = lang === 'go'
    ? mergeGoBuildTagsIntoFlags([], resolveRepoGoBuildTags(host.projectEnv, host.target.root, repoInfo.path, openDoc))
    : [];
  if (mode === 'debug') {
    onDebug({
      language: lang,
      launch: {
        program: spec.debug_program || filePath,
        args: spec.args ?? [],
        ...(goFlags.length > 0 ? { build_flags: goFlags } : {}),
        cwd: spec.cwd || repoInfo.path || host.target.root,
        test_name: spec.debug_test_name || spec.test_name || spec.test_filter || (item.kind === 'test' ? item.label : undefined),
      },
    });
    return;
  }
  const keepTc = mode === 'run_keep_tc';
  onRun(
    {
      id: `gutter-${item.line}`, name: item.label, runner: spec.runner || defaultRunnerForLanguage(item.language),
      command: spec.command ?? null, args: spec.args ?? [],
      ...(goFlags.length > 0 ? { build_flags: goFlags } : {}),
      cwd: spec.cwd ?? repoInfo.path ?? null,
      env_overrides: keepTc ? { TESTCONTAINERS_RYUK_DISABLED: 'true' } : {},
      inherit_shell_env: host.projectEnv?.inherit_shell_env ?? true,
      target_path: filePath || null, test_filter: spec.test_filter || spec.test_name || null,
    },
    keepTc,
  );
}

export async function launchDebugSession(
  host: DebugAndEnvHost,
  opts: {
    configId?: string;
    language?: string;
    confirmProdTargets?: boolean;
    launch?: { program: string; args?: string[]; build_flags?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string>; env_file?: string | null; inherit_shell_env?: boolean; sdk_path?: string | null };
  },
  prevCtl: AbortController | null,
  onSnapshot: (snap: IdeDebugSessionSnapshot) => void,
): Promise<AbortController | null> {
  if (!host.target.root) return prevCtl;
  const rawCfgId = opts.configId ?? host.selectedConfigId ?? undefined;
  const cfg = rawCfgId ? host.runConfigs.find((c) => c.id === rawCfgId) : undefined;
  const fallbackLang = host.activeTab?.language || 'go';
  const language = opts.language || languageFromRunner(cfg?.runner, fallbackLang);
  const program = opts.launch?.program || cfg?.target_path || host.activePath || host.target.root;
  const persistedConfigId = cfg && !cfg.auto_detected && !cfg.id.startsWith('auto:') ? cfg.id : undefined;
  host.debugLoading = true;
  host.openBottomTab('debug');
  try {
    const modSdk = resolveModuleSdkOverride(host, program);
    const repoInfo = resolveRepoRootForPath(host.target.root, host.projectRoots, opts.launch?.cwd || program);
    const openDocForGo = language === 'go' && host.activeTab && host.activeTab.path === program ? host.activeTab.content : null;
    const goTags = language === 'go'
      ? resolveRepoGoBuildTags(host.projectEnv, host.target.root, repoInfo.path, openDocForGo)
      : [];
    const springEnv: Record<string, string> = host.activeSpringProfiles.length > 0
      ? { SPRING_PROFILES_ACTIVE: host.activeSpringProfiles.join(',') }
      : {};
    const tcEnv: Record<string, string> = host.keepTestcontainersAlive ? { TESTCONTAINERS_RYUK_DISABLED: 'true' } : {};
    const baseLaunch = opts.launch ?? {
      program,
      args: cfg?.args ?? [],
      build_flags: cfg?.build_flags ?? [],
      cwd: cfg?.cwd || repoInfo.path || host.target.root,
      test_name: cfg?.test_filter || undefined,
      env: cfg?.env_overrides ?? {},
      env_file: cfg?.env_file ?? null,
      inherit_shell_env: cfg?.inherit_shell_env ?? host.projectEnv?.inherit_shell_env ?? true,
      sdk_path: cfg?.sdk_path ?? modSdk ?? null,
    };
    const mergedBuildFlags = goTags.length > 0
      ? mergeGoBuildTagsIntoFlags(baseLaunch.build_flags, goTags)
      : baseLaunch.build_flags;
    const effectiveLaunch = {
      ...baseLaunch,
      ...(mergedBuildFlags && mergedBuildFlags.length > 0 ? { build_flags: mergedBuildFlags } : {}),
      sdk_path: baseLaunch.sdk_path ?? modSdk ?? null,
      env: { ...springEnv, ...tcEnv, ...(baseLaunch.env ?? {}) },
    };
    const resp = await api.post<{ session: IdeDebugSessionSnapshot }>('/ide/debug/sessions', {
      target: host.targetPayload,
      project_folders: host.projectRoots.map((r) => r.path),
      language,
      config_id: persistedConfigId,
      launch: effectiveLaunch,
      maven_profiles: host.activeMavenProfiles,
      spring_profiles: host.activeSpringProfiles,
      skip_tests: host.buildSkipTests,
      offline: host.buildOffline,
      breakpoints: host.effectiveBreakpoints,
      break_on_exceptions: Boolean(host.breakOnExceptions),
      ...(opts.confirmProdTargets ? { confirm_prod_targets: true } : {}),
    });
    host.lastDebugLaunch = { configId: persistedConfigId, language, launch: effectiveLaunch };
    host.expandedVariables = {};
    onSnapshot(resp.session);
    return attachDebugSessionStream(host, resp.session.id, prevCtl, onSnapshot);
  } catch (e) {
    toastError('Couldn’t start debugger', e);
    return prevCtl;
  } finally {
    host.debugLoading = false;
  }
}

export async function sendDebugAction(
  host: DebugAndEnvHost,
  action: string,
  extra: { thread_id?: number; frame_id?: number; path?: string; line?: number },
  onSnapshot: (snap: IdeDebugSessionSnapshot) => void,
): Promise<void> {
  if (!host.activeDebug) return;
  try {
    if (host.collabIsGuest && host.collabRoom && host.collabPeerId && host.collabPeerSecret) {
      const resp = await collabGuestPost<{ session: IdeDebugSessionSnapshot }>(
        `/ide/collab/guest/${encodeURIComponent(host.collabRoom.id)}/debug-action`,
        host.collabGuestToken,
        { peer_id: host.collabPeerId, peer_secret: host.collabPeerSecret, debug_id: host.activeDebug.id, action, ...extra },
      );
      if (resp.session) onSnapshot(resp.session);
      return;
    }
    const resp = await api.post<{ session: IdeDebugSessionSnapshot }>(
      `/ide/debug/sessions/${encodeURIComponent(host.activeDebug.id)}/command`,
      {
        action, thread_id: extra.thread_id ?? host.activeDebug.active_thread_id ?? undefined,
        frame_id: extra.frame_id ?? host.activeDebug.active_frame_id ?? undefined, path: extra.path, line: extra.line,
      },
    );
    onSnapshot(resp.session);
  } catch (e) {
    // The host owns its debug session: a guest's refused action (Restart) is said next to
    // the button, not as a raw error toast.
    if (host.collabIsGuest && e instanceof IdeCollabHttpError && e.status === 403) {
      ideCollabNotices.debug = e.message ? `${e.message.charAt(0).toUpperCase()}${e.message.slice(1)}` : 'Only the host can do that.';
      return;
    }
    toastError(`Couldn’t run debugger ${action.replace('_', ' ')}`, e);
  }
}

export async function installToolchainJob(
  host: DebugAndEnvHost,
  ecosystem: string,
  component: 'sdk' | 'lsp' | 'dap' | 'all',
  version: string | undefined,
  ensurePoller: () => void,
): Promise<IdeInstallJob | null> {
  try {
    const resp = await api.post<{ job: IdeInstallJob }>('/ide/toolchains/install', {
      target: host.targetPayload, ecosystem, component, version,
    });
    host.installJobs = [resp.job, ...host.installJobs.filter((j) => j.id !== resp.job.id)].slice(0, 12);
    ensurePoller();
    return resp.job;
  } catch (e) {
    toastError(`Couldn’t install ${ecosystem} ${component}`, e);
    return null;
  }
}

export async function createProjectPythonVenv(
  host: DebugAndEnvHost & { loadEnvironments(): Promise<void>; saveEnvironment(p: Partial<IdeProjectEnvironment>): Promise<boolean> },
  venvDir = '.venv',
  pythonVersion?: string,
  moduleRoot?: string,
): Promise<{ path: string; interpreter_path: string; version: string } | null> {
  const effectiveRoot = (moduleRoot || host.target.root).replace(/\/+$/, '');
  try {
    type RawVenvPayload = { path?: string; interpreter_path?: string; python_path?: string; version?: string };
    const raw = await api.long.post<{ venv?: RawVenvPayload } & RawVenvPayload>(
      '/ide/environments/python-venv',
      { target: host.targetPayload, root: effectiveRoot, venv_dir: venvDir, python_version: pythonVersion || undefined },
    );
    const v = raw.venv ?? raw;
    const normPath = v.path || `${effectiveRoot}/${venvDir.replace(/^\.\//, '')}`;
    const interp = v.interpreter_path || v.python_path || `${normPath}/bin/python`;
    const normalized = { path: normPath, interpreter_path: interp, version: v.version || '' };
    await host.loadEnvironments();
    if (moduleRoot) {
      await host.saveEnvironment({ module_sdks: { ...(host.projectEnv?.module_sdks ?? {}), [moduleRoot]: interp } });
    } else {
      await host.saveEnvironment({ python_interpreter: interp });
    }
    return normalized;
  } catch (e) {
    toastError('Couldn’t create Python virtual environment', e);
    return null;
  }
}

export async function removeSavedRunConfig(
  host: DebugAndEnvHost & { targetQuery(): string },
  id: string,
): Promise<void> {
  try {
    await api.del(`/ide/run-configs/${encodeURIComponent(id)}?${host.targetQuery()}`);
    host.runConfigs = host.runConfigs.filter((c) => c.id !== id);
    if (host.selectedConfigId === id) host.selectedConfigId = host.runConfigs[0]?.id ?? null;
  } catch (e) {
    toastError('Couldn’t delete run configuration', e);
  }
}
