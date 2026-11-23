// Containers console API client — thin typed wrappers over the generic `api`
// helper for every `/containers/*` route served by `otto-containers`
// (nested under `/api/v1` by `otto-server`, matching `k8s.ts` and `aws.ts`).

import { api } from './client';
import type { Session } from './types';
import type {
  ContainerActionResp,
  ContainerDetail,
  ContainerExecReq,
  ContainerLogsResp,
  ContainerStatsResp,
  ContainersContextsResp,
  ContainersListResp,
  ContainersStatusResp,
  ImageInspectResp,
  ImagesListResp,
  InstallRuntimeReq,
  NetworksListResp,
  PruneResp,
  PullImageReq,
  PullImageResp,
  RunContainerReq,
  RunContainerResp,
  RuntimeStatusResp,
  SelectContextReq,
  StackActionResp,
  SystemPruneReq,
  SystemPruneResp,
  VolumesListResp,
} from './types/containers';

const enc = encodeURIComponent;

function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null) continue;
    u.set(k, String(v));
  }
  const s = u.toString();
  return s ? `?${s}` : '';
}

export interface ContainerLogsOpts {
  tail?: number;
  since?: number;
  timestamps?: boolean;
}

export const containersApi = {
  // --- daemon status, embedded runtime & contexts --------------------------
  status: (signal?: AbortSignal) => api.get<ContainersStatusResp>('/containers/status', signal),
  runtime: (signal?: AbortSignal) => api.get<RuntimeStatusResp>('/containers/runtime', signal),
  installRuntime: (body: InstallRuntimeReq = {}) =>
    api.post<RuntimeStatusResp>('/containers/runtime/install', body),
  startRuntime: () => api.post<RuntimeStatusResp>('/containers/runtime/start', {}),
  stopRuntime: () => api.post<RuntimeStatusResp>('/containers/runtime/stop', {}),
  restartRuntime: () => api.post<RuntimeStatusResp>('/containers/runtime/restart', {}),
  contexts: (signal?: AbortSignal) => api.get<ContainersContextsResp>('/containers/contexts', signal),
  selectContext: (body: SelectContextReq) =>
    api.post<ContainersContextsResp>('/containers/contexts/select', body),

  // --- containers ----------------------------------------------------------
  list: (opts: { all?: boolean } = {}, signal?: AbortSignal) =>
    api.get<ContainersListResp>(`/containers${qs({ all: opts.all })}`, signal),
  inspect: (id: string, signal?: AbortSignal) =>
    api.get<ContainerDetail>(`/containers/${enc(id)}`, signal),
  revealSecrets: (id: string) =>
    api.post<ContainerDetail>(`/containers/${enc(id)}/secrets/reveal`, {}),
  run: (body: RunContainerReq) => api.post<RunContainerResp>('/containers/run', body),
  start: (id: string) => api.post<ContainerActionResp>(`/containers/${enc(id)}/start`, {}),
  stop: (id: string, opts: { timeoutSecs?: number } = {}) =>
    api.post<ContainerActionResp>(`/containers/${enc(id)}/stop${qs({ t: opts.timeoutSecs })}`, {}),
  restart: (id: string, opts: { timeoutSecs?: number } = {}) =>
    api.post<ContainerActionResp>(`/containers/${enc(id)}/restart${qs({ t: opts.timeoutSecs })}`, {}),
  pause: (id: string) => api.post<ContainerActionResp>(`/containers/${enc(id)}/pause`, {}),
  unpause: (id: string) => api.post<ContainerActionResp>(`/containers/${enc(id)}/unpause`, {}),
  kill: (id: string, opts: { signal?: string } = {}) =>
    api.post<ContainerActionResp>(`/containers/${enc(id)}/kill${qs({ signal: opts.signal })}`, {}),
  remove: (id: string, opts: { force?: boolean; volumes?: boolean } = {}) =>
    api.del<ContainerActionResp>(
      `/containers/${enc(id)}${qs({ force: opts.force || undefined, v: opts.volumes || undefined })}`,
    ),

  // --- compose stacks ------------------------------------------------------
  stackStart: (project: string) =>
    api.post<StackActionResp>(`/containers/stacks/${enc(project)}/start`, {}),
  stackStop: (project: string) =>
    api.post<StackActionResp>(`/containers/stacks/${enc(project)}/stop`, {}),
  stackRestart: (project: string) =>
    api.post<StackActionResp>(`/containers/stacks/${enc(project)}/restart`, {}),

  // --- logs, stats & PTY exec ----------------------------------------------
  logs: (id: string, opts: ContainerLogsOpts = {}, signal?: AbortSignal) =>
    api.get<ContainerLogsResp>(
      `/containers/${enc(id)}/logs${qs({
        tail: opts.tail,
        since: opts.since,
        timestamps: opts.timestamps,
      })}`,
      signal,
    ),
  stats: (id: string, signal?: AbortSignal) =>
    api.get<ContainerStatsResp>(`/containers/${enc(id)}/stats`, signal),
  exec: (id: string, body: ContainerExecReq) =>
    api.post<Session>(`/containers/${enc(id)}/exec`, body),

  // --- images --------------------------------------------------------------
  listImages: (signal?: AbortSignal) => api.get<ImagesListResp>('/containers/images', signal),
  inspectImage: (ref: string, signal?: AbortSignal) =>
    api.get<ImageInspectResp>(`/containers/images/${enc(ref)}`, signal),
  pullImage: (body: PullImageReq) => api.post<PullImageResp>('/containers/images/pull', body),
  removeImage: (id: string, opts: { force?: boolean } = {}) =>
    api.del<ContainerActionResp>(
      `/containers/images/${enc(id)}${qs({ force: opts.force || undefined })}`,
    ),
  pruneImages: (opts: { danglingOnly?: boolean } = {}) =>
    api.post<PruneResp>(
      `/containers/images/prune${qs({ dangling_only: opts.danglingOnly })}`,
      {},
    ),

  // --- volumes -------------------------------------------------------------
  listVolumes: (signal?: AbortSignal) => api.get<VolumesListResp>('/containers/volumes', signal),
  removeVolume: (name: string, opts: { force?: boolean } = {}) =>
    api.del<ContainerActionResp>(
      `/containers/volumes/${enc(name)}${qs({ force: opts.force || undefined })}`,
    ),
  pruneVolumes: () => api.post<PruneResp>('/containers/volumes/prune', {}),

  // --- networks ------------------------------------------------------------
  listNetworks: (signal?: AbortSignal) => api.get<NetworksListResp>('/containers/networks', signal),
  removeNetwork: (id: string) => api.del<ContainerActionResp>(`/containers/networks/${enc(id)}`),
  pruneNetworks: () => api.post<PruneResp>('/containers/networks/prune', {}),

  // --- system prune --------------------------------------------------------
  systemPrune: (body: SystemPruneReq = {}) =>
    api.post<SystemPruneResp>('/containers/system/prune', body),
};
