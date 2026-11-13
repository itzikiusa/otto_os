// Toolchains, Environments, Run Configurations, and DAP Debugger helpers for IdeStore.
// Extracted so `ideStore.svelte.ts` stays comfortably under the 800-line file-loc budget.

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
  type IdeInstallJob,
  type IdeOpenTab,
  type IdeProjectEnvironment,
  type IdePythonVenv,
  type IdeReplEntry,
  type IdeRunConfig,
  type IdeRunState,
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
  pythonVenvs: IdePythonVenv[];
  dotenvFiles: string[];
  runConfigs: IdeRunConfig[];
  selectedConfigId: string | null;
  envLoading: boolean;
  envError: string | null;
  activeRun: IdeRunState | null;
  runsHistory: IdeRunState[];
  readonly allBreakpoints: IdeBreakpoint[];
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
}

export async function fetchToolchains(host: DebugAndEnvHost, onEnsureJobs: () => void): Promise<void> {
  if (!host.target.root) return;
  host.toolchainsLoading = true;
  host.toolchainsError = null;
  try {
    const resp = await api.long.get<{ target_os: string; target_arch: string; ecosystems: IdeToolchainEcosystem[]; jobs: IdeInstallJob[] }>(`/ide/toolchains?${host.targetQuery()}`);
    if (resp.target_os) host.targetOs = resp.target_os;
    if (resp.target_arch) host.targetArch = resp.target_arch;
    host.toolchains = resp.ecosystems ?? [];
    host.installJobs = resp.jobs ?? [];
    onEnsureJobs();
  } catch (e) { host.toolchainsError = loadErrorText(e); }
  finally { host.toolchainsLoading = false; }
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
    const resp = await api.get<{ env: IdeProjectEnvironment; python_venvs: IdePythonVenv[]; dotenv_files: string[]; configs: IdeRunConfig[] }>(`/ide/environments?${host.targetQuery()}`);
    host.projectEnv = resp.env;
    host.pythonVenvs = resp.python_venvs ?? [];
    host.dotenvFiles = resp.dotenv_files ?? [];
    host.runConfigs = resp.configs ?? [];
    if (!host.selectedConfigId && host.runConfigs.length > 0) host.selectedConfigId = host.runConfigs[0].id;
  } catch (e) { host.envError = loadErrorText(e); }
  finally { host.envLoading = false; }
}

export async function persistEnvironment(host: DebugAndEnvHost, patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[] }): Promise<boolean> {
  try {
    const merged = { ...(host.projectEnv ?? { env_vars: [] }), ...patch };
    const resp = await api.put<{ env: IdeProjectEnvironment }>('/ide/environments', { target: host.targetPayload, ...merged });
    host.projectEnv = resp.env;
    toasts.success('Saved project environment');
    return true;
  } catch (e) { toastError('Couldn’t save environment', e); return false; }
}

export async function upsertRunConfig(host: DebugAndEnvHost, cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> {
  try {
    const resp = await api.post<{ config?: IdeRunConfig } & Partial<IdeRunConfig>>('/ide/run-configs', {
      target: host.targetPayload, id: cfg.id || undefined, name: cfg.name, runner: cfg.runner,
      command: cfg.command ?? null, args: cfg.args ?? [], cwd: cfg.cwd ?? null,
      env_overrides: cfg.env_overrides ?? {}, before_launch: cfg.before_launch ?? null,
      target_path: cfg.target_path ?? null, test_filter: cfg.test_filter ?? null,
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
  const cfgId = opts.configId ?? (opts.adhoc ? undefined : host.selectedConfigId ?? undefined);
  const adhoc = opts.adhoc ?? (!cfgId && host.activeTab ? {
    id: 'adhoc', name: `Run ${host.activeTab.name}`, runner: defaultRunnerForLanguage(host.activeTab.language),
    args: [], env_overrides: {}, target_path: host.activeTab.path,
  } : undefined);
  try {
    host.openBottomTab('run');
    const resp = await api.post<{ run: IdeRunState }>('/ide/runs', { target: host.targetPayload, config_id: cfgId, adhoc });
    host.activeRun = resp.run;
    prevPoller?.stop();
    const poller = pollWhileVisible(async (signal) => {
      const r = await api.bg.get<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(resp.run.id)}`, signal);
      host.activeRun = r.run;
      if (r.run.status !== 'running') {
        host.runsHistory = [r.run, ...host.runsHistory.filter((x) => x.id !== r.run.id)].slice(0, 15);
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
        args: [],
        cwd: spec.cwd || host.target.root,
        test_name: spec.debug_test_name || spec.test_name || spec.test_filter || (item.kind === 'test' ? item.label : undefined),
      },
    });
    return;
  }
  onRun({
    id: `gutter-${item.line}`, name: item.label, runner: spec.runner || defaultRunnerForLanguage(item.language),
    command: spec.command ?? null, args: spec.args ?? [], cwd: spec.cwd ?? null, env_overrides: {},
    target_path: filePath || null, test_filter: spec.test_filter || spec.test_name || null,
  });
}

export async function launchDebugSession(
  host: DebugAndEnvHost,
  opts: { configId?: string; language?: string; launch?: { program: string; args?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string> } },
  prevCtl: AbortController | null,
  onSnapshot: (snap: IdeDebugSessionSnapshot) => void,
): Promise<AbortController | null> {
  if (!host.target.root) return prevCtl;
  const cfg = host.runConfigs.find((c) => c.id === (opts.configId ?? host.selectedConfigId));
  const language = opts.language || host.activeTab?.language || 'go';
  const program = opts.launch?.program || cfg?.target_path || host.activePath || host.target.root;
  host.debugLoading = true;
  host.openBottomTab('debug');
  try {
    const resp = await api.post<{ session: IdeDebugSessionSnapshot }>('/ide/debug/sessions', {
      target: host.targetPayload, language, config_id: opts.configId ?? cfg?.id,
      launch: opts.launch ?? { program, args: cfg?.args ?? [], cwd: cfg?.cwd || host.target.root, test_name: cfg?.test_filter || undefined, env: cfg?.env_overrides ?? {} },
      breakpoints: host.allBreakpoints,
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
  } catch (e) { toastError(`Debugger ${action.replace('_', ' ')} failed`, e); }
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
