// Toolchain install jobs (SDK / gopls / Delve), the "Delve too old → Update Delve
// and retry" flow, and project Python venvs. Split out of ideDebugAndEnvController.ts
// (which re-exports these) to keep that file under the size ratchet.

import { api } from '../../../lib/api/client';
import { toastError } from '../../../lib/toastError';
import type { DebugAndEnvHost } from './ideDebugAndEnvController';
import type { IdeInstallJob, IdeProjectEnvironment } from './ideTypes';

/** "Update Delve": rebuild it with the project's Go, then start the same debug session again. */
export async function updateDelveAndRetry(host: DebugAndEnvHost, retry: () => Promise<void>): Promise<void> {
  const job = await installToolchainJob(host, 'go', 'dap', undefined, () => {});
  if (!job) return;
  for (let i = 0; i < 300; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    const cur = (await api.get<{ job: IdeInstallJob }>(`/ide/toolchains/jobs/${encodeURIComponent(job.id)}`)).job;
    if (cur.status === 'error') throw new Error(cur.error || 'Delve install failed');
    if (cur.status === 'done') return retry();
  }
}

/** The daemon's `tool_outdated:go:dap — …` launch error (Delve built for an older Go)
 *  as a notice with an "Update Delve" action; null for any other error. */
export function toolOutdatedNotice(e: unknown): { title: string; body: string; action: string } | null {
  const msg = e instanceof Error ? e.message : String(e ?? '');
  const at = msg.indexOf('tool_outdated:go:dap');
  if (at < 0) return null;
  const body = msg.slice(at).replace(/^tool_outdated:go:dap\s*[—-]\s*/, '');
  return { title: 'Delve is too old for this Go', body, action: 'Update Delve' };
}

export async function installToolchainJob(
  host: DebugAndEnvHost,
  ecosystem: string,
  component: 'sdk' | 'lsp' | 'dap' | 'all',
  version: string | undefined,
  ensurePoller: () => void,
): Promise<IdeInstallJob | null> {
  try {
    // gopls / Delve are built with the project's own Go, so they match it.
    const goRoot = ecosystem === 'go' && component !== 'sdk'
      ? host.toolchains.find((e) => e.ecosystem === 'go')?.installed.find((s) => s.active)?.path ?? null
      : null;
    const resp = await api.post<{ job: IdeInstallJob }>('/ide/toolchains/install', {
      target: host.targetPayload, ecosystem, component, version, ...(goRoot ? { go_root: goRoot } : {}),
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
