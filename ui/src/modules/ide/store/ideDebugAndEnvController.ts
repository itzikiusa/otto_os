// Toolchains, Shell Environment, Project Environments, Run Configurations,
// and DAP Debugger helpers for `IdeStore`.

import { api } from '../../../lib/api/client';
import { loadErrorText } from '../../../lib/loadError';
import { pollWhileVisible, type Poller } from '../../../lib/poll';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import {
  collabGuestPost,
  defaultRunnerForLanguage,
  streamIdeSse,
  type IdeBottomTab,
  type IdeBreakpoint,
  type IdeCollabRoom,
  type IdeDebugSessionSnapshot,
  type IdeEnvVar,
  type IdeGutterItem,
  type IdeInstalledSdk,
  type IdeInstallJob,
  type IdeOpenTab,
  type IdeProjectEnvironment,
  type IdePythonVenv,
  type IdeReplEntry,
  type IdeRunConfig,
  type IdeRunState,
  type IdeShellEnvResp,
  type IdeTargetSpec,
  type IdeToolchainEcosystem,
  type IdeVariable,
  type IdeWatch,
} from './ideTypes';

export interface DebugAndEnvHost {
  target: IdeTargetSpec;
  readonly targetPayload: IdeTargetSpec;
  targetOs: string;
  targetArch: string;
  activePath: string | null;
  readonly activeTab: IdeOpenTab | null;
  toolchains: IdeToolchainEcosystem[];
  toolchainsLoading: boolean;
  toolchainsError: string | null;
  installJobs: IdeInstallJob[];
  projectEnv: IdeProjectEnvironment | null;
  effectiveEnv: Record<string, string>;
  shellEnv: IdeShellEnvResp | null;
  shellEnvLoading: boolean;
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
      target_os: string; target_arch: string; ecosystems: IdeToolchainEcosystem[]; jobs: IdeInstallJob[];
    }>(`/ide/toolchains?${q}`);
    if (resp.target_os) host.targetOs = resp.target_os;
    if (resp.target_arch) host.targetArch = resp.target_arch;
    host.toolchains = resp.ecosystems ?? [];
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
  host.shellEnvLoading = true;
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
    if (reload) toasts.success('Reloaded shell environment', `${Object.keys(normalized.vars).length} variables`);
    return normalized;
  } catch (e) {
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
      const resp = await api.bg.get<{ job: IdeInstallJob }>(`/ide/toolchains/jobs/${encodeURIComponent(job.id)}`, signal);
      host.installJobs = host.installJobs.map((j) => (j.id === resp.job.id ? resp.job : j));
      if (resp.job.status !== 'running') finishedAny = true;
    }
    if (finishedAny) onDone();
  }, { ms: 1200, immediate: false });
  return poller;
}

export async function fetchEnvironments(host: DebugAndEnvHost): Promise<void> {
  if (!host.target.root) return;
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
      shell_env?: IdeShellEnvResp | null;
    }>(`/ide/environments?${host.targetQuery()}`);
    host.projectEnv = resp.env;
    host.effectiveEnv = resp.effective_env ?? {};
    if (resp.shell_env) {
      host.shellEnv = { ...resp.shell_env, vars: resp.shell_env.vars ?? resp.shell_env.env ?? {} };
    }
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
  } catch (e) { host.envError = loadErrorText(e); }
  finally { host.envLoading = false; }
}

export async function persistEnvironment(host: DebugAndEnvHost, patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[] }): Promise<boolean> {
  try {
    const merged: IdeProjectEnvironment = {
      inherit_shell_env: true,
      ...(host.projectEnv ?? { env_vars: [] }),
      ...patch,
    };
    const resp = await api.put<{ env: IdeProjectEnvironment }>('/ide/environments', { target: host.targetPayload, ...merged });
    host.projectEnv = resp.env;
    toasts.success('Saved project environment');
    return true;
  } catch (e) { toastError('Couldn’t save environment', e); return false; }
}

export async function upsertRunConfig(host: DebugAndEnvHost, cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> {
  try {
    const cleanId = cfg.id && !cfg.id.startsWith('auto:') ? cfg.id : undefined;
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
    });
    const saved = (resp.config ?? resp) as IdeRunConfig;
    await fetchEnvironments(host);
    if (saved?.id) host.selectedConfigId = saved.id;
    return saved;
  } catch (e) { toastError('Couldn’t save run configuration', e); return null; }
}

export async function launchIdeRun(
  host: DebugAndEnvHost,
  opts: { configId?: string; adhoc?: Partial<IdeRunConfig> },
  prevPoller: Poller | null,
): Promise<Poller | null> {
  if (!host.target.root) return prevPoller;
  const rawId = opts.configId ?? (opts.adhoc ? undefined : host.selectedConfigId ?? undefined);
  const selectedCfg = rawId ? host.runConfigs.find((c) => c.id === rawId) : undefined;
  const isClientAuto = Boolean(rawId?.startsWith('auto:') || selectedCfg?.auto_detected);
  const cfgId = isClientAuto ? undefined : rawId;
  const adhoc = opts.adhoc ?? (isClientAuto && selectedCfg ? selectedCfg : (!cfgId && host.activeTab ? {
    id: 'adhoc', name: `Run ${host.activeTab.name}`, runner: defaultRunnerForLanguage(host.activeTab.language),
    args: [], env_overrides: {}, inherit_shell_env: host.projectEnv?.inherit_shell_env ?? true, target_path: host.activeTab.path,
  } : undefined));
  try {
    host.openBottomTab('run');
    const resp = await api.post<{ run: IdeRunState }>('/ide/runs', { target: host.targetPayload, config_id: cfgId, adhoc });
    host.activeRun = resp.run;
    prevPoller?.stop();
    const poller = pollWhileVisible(async (signal) => {
      const r = await api.bg.get<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(resp.run.id)}`, signal);
      const boundedOutput = r.run.output.length > 2000 ? r.run.output.slice(-2000) : r.run.output;
      const liveRun: IdeRunState = boundedOutput === r.run.output ? r.run : { ...r.run, output: boundedOutput };
      host.activeRun = liveRun;
      if (liveRun.status !== 'running') {
        const archived: IdeRunState = liveRun.output.length > 200
          ? { ...liveRun, output: liveRun.output.slice(-200) }
          : liveRun;
        host.runsHistory = [archived, ...host.runsHistory.filter((x) => x.id !== archived.id)].slice(0, 8);
        poller.stop();
      }
    }, { ms: 750, floorMs: 500, immediate: false });
    return poller;
  } catch (e) {
    toastError('Couldn’t start run', e);
    return prevPoller;
  }
}

export function launchGutterAction(
  host: DebugAndEnvHost,
  item: IdeGutterItem,
  mode: 'run' | 'debug',
  onRun: (adhoc: Partial<IdeRunConfig>) => void,
  onDebug: (opts: { language: string; launch: { program: string; args: string[]; cwd: string; test_name?: string } }) => void,
): void {
  const spec = item.run_spec ?? {};
  const filePath = spec.target_path || spec.program || host.activePath || '';
  if (mode === 'debug') {
    onDebug({
      language: spec.language || item.language || host.activeTab?.language || 'go',
      launch: {
        program: spec.debug_program || filePath,
        args: spec.args ?? [],
        cwd: spec.cwd || host.target.root,
        test_name: spec.debug_test_name || spec.test_name || spec.test_filter || (item.kind === 'test' ? item.label : undefined),
      },
    });
    return;
  }
  onRun({
    id: `gutter-${item.line}`, name: item.label, runner: spec.runner || defaultRunnerForLanguage(item.language),
    command: spec.command ?? null, args: spec.args ?? [], cwd: spec.cwd ?? null, env_overrides: {},
    inherit_shell_env: host.projectEnv?.inherit_shell_env ?? true,
    target_path: filePath || null, test_filter: spec.test_filter || spec.test_name || null,
  });
}

export async function launchDebugSession(
  host: DebugAndEnvHost,
  opts: { configId?: string; language?: string; launch?: { program: string; args?: string[]; build_flags?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string>; env_file?: string | null; inherit_shell_env?: boolean; sdk_path?: string | null } },
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
    const resp = await api.post<{ session: IdeDebugSessionSnapshot }>('/ide/debug/sessions', {
      target: host.targetPayload,
      language,
      config_id: persistedConfigId,
      launch: opts.launch ?? {
        program,
        args: cfg?.args ?? [],
        build_flags: cfg?.build_flags ?? [],
        cwd: cfg?.cwd || host.target.root,
        test_name: cfg?.test_filter || undefined,
        env: cfg?.env_overrides ?? {},
        env_file: cfg?.env_file ?? null,
        inherit_shell_env: cfg?.inherit_shell_env ?? host.projectEnv?.inherit_shell_env ?? true,
        sdk_path: cfg?.sdk_path ?? null,
      },
      breakpoints: host.effectiveBreakpoints,
    });
    host.expandedVariables = {};
    onSnapshot(resp.session);
    prevCtl?.abort();
    const ctl = new AbortController();
    void streamIdeSse(`/ide/debug/sessions/${encodeURIComponent(resp.session.id)}/events`, {}, ctl.signal, (raw) => {
      try {
        const parsed = JSON.parse(raw) as { session?: IdeDebugSessionSnapshot } & Partial<IdeDebugSessionSnapshot>;
        const snap = (parsed.session ?? parsed) as IdeDebugSessionSnapshot;
        if (snap?.id) onSnapshot(snap);
      } catch { /* heartbeat */ }
    });
    return ctl;
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
  } catch (e) { toastError(`Couldn’t run debugger ${action.replace('_', ' ')}`, e); }
}

export async function refreshWatchValues(host: DebugAndEnvHost): Promise<void> {
  if (!host.activeDebug || host.activeDebug.state !== 'paused' || host.watches.length === 0) return;
  const updated: IdeWatch[] = [];
  for (const w of host.watches) {
    try {
      const resp = await api.bg.post<{ result: string; var_type?: string }>(
        `/ide/debug/sessions/${encodeURIComponent(host.activeDebug.id)}/evaluate`,
        { expression: w.expression, frame_id: host.activeDebug.active_frame_id ?? undefined, context: 'watch' },
      );
      updated.push({ ...w, value: resp.result, var_type: resp.var_type, error: null });
    } catch (e) { updated.push({ ...w, value: '', error: loadErrorText(e) }); }
  }
  host.watches = updated;
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
): Promise<{ path: string; interpreter_path: string; version: string } | null> {
  try {
    const resp = await api.long.post<{ path: string; interpreter_path: string; version: string }>(
      '/ide/environments/python-venv',
      { target: host.targetPayload, venv_dir: venvDir, python_version: pythonVersion || undefined },
    );
    await host.loadEnvironments();
    await host.saveEnvironment({ python_interpreter: resp.interpreter_path });
    return resp;
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

export function normalizeIncomingDebugSnapshot(
  host: { target: IdeTargetSpec; resolvePath(p: string): string },
  raw: IdeDebugSessionSnapshot,
): { snap: IdeDebugSessionSnapshot; paused: string | null } {
  let snap = raw;
  const stripPrivate = (p: string) => p.replace(/^\/private(?=\/(?:var|tmp)(?:\/|$))/, '');
  const root = host.target.root.replace(/\/+$/, '');
  let paused = snap.paused_file ? host.resolvePath(snap.paused_file) : null;
  if (paused && host.target.kind === 'local' && root) {
    const cRoot = stripPrivate(root);
    const cPaused = stripPrivate(paused);
    if (cPaused === cRoot || cPaused.startsWith(`${cRoot}/`)) {
      paused = `${root}${cPaused.slice(cRoot.length)}`;
      snap = { ...snap, paused_file: paused };
    }
  }
  if (snap.output?.length > 1000) snap = { ...snap, output: snap.output.slice(-1000) };
  return { snap, paused };
}
