// Stream, watch-evaluation, and snapshot/breakpoint normalization helpers for IDE DAP sessions.

import { api } from '../../../lib/api/client';
import { loadErrorText } from '../../../lib/loadError';
import { canonicalizeLocalProjectPath } from '../shell/ideProjectHelpers';
import type { DebugAndEnvHost } from './ideDebugAndEnvController';
import {
  streamIdeSse,
  type IdeBreakpoint,
  type IdeDebugSessionSnapshot,
  type IdeProjectRoot,
  type IdeTargetSpec,
  type IdeWatch,
} from './ideTypes';

export function attachDebugSessionStream(
  host: DebugAndEnvHost,
  sessionId: string,
  prevCtl: AbortController | null,
  onSnapshot: (snap: IdeDebugSessionSnapshot) => void,
): AbortController {
  prevCtl?.abort();
  const ctl = new AbortController();
  host.debugStreamActive = false;
  void streamIdeSse(`/ide/debug/sessions/${encodeURIComponent(sessionId)}/events`, {}, ctl.signal, (raw) => {
    try {
      const parsed = JSON.parse(raw) as { session?: IdeDebugSessionSnapshot } & Partial<IdeDebugSessionSnapshot>;
      const snap = (parsed.session ?? parsed) as IdeDebugSessionSnapshot;
      if (snap?.id) onSnapshot(snap);
    } catch { /* heartbeat */ }
  }, () => { host.debugStreamActive = true; }).catch(() => {}).finally(() => { host.debugStreamActive = false; });
  return ctl;
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

export function normalizeIncomingDebugSnapshot(
  host: { target: IdeTargetSpec; readonly projectRoots: IdeProjectRoot[]; resolvePath(p: string): string },
  raw: IdeDebugSessionSnapshot,
): { snap: IdeDebugSessionSnapshot; paused: string | null } {
  const normPath = (p?: string | null): string | null => {
    if (!p) return null;
    const resolved = host.resolvePath(p);
    return host.target.kind === 'local'
      ? canonicalizeLocalProjectPath(resolved, host.projectRoots)
      : resolved;
  };
  const stack_frames = (raw.stack_frames ?? []).map((f) =>
    f.path ? { ...f, path: normPath(f.path) } : f,
  );
  const rawPaused = raw.paused_file || raw.stopped_file || (raw.state === 'paused' ? stack_frames[0]?.path : null);
  const paused = normPath(rawPaused);
  const pausedLine = raw.paused_line || raw.stopped_line || (raw.state === 'paused' ? (stack_frames[0]?.line ?? null) : null);
  const breakpoints = (raw.breakpoints ?? []).map((bp) =>
    bp.path ? { ...bp, path: normPath(bp.path) ?? bp.path } : bp,
  );
  const output = raw.output?.length > 1000 ? raw.output.slice(-1000) : (raw.output ?? []);
  const snap: IdeDebugSessionSnapshot = {
    ...raw,
    paused_file: paused,
    paused_line: pausedLine,
    stack_frames,
    breakpoints,
    output,
  };
  return { snap, paused };
}

export function syncSnapshotBreakpointsToStore(
  host: { breakpointsByPath: Record<string, IdeBreakpoint[]>; resolvePath?(p: string): string },
  snap: IdeDebugSessionSnapshot,
): void {
  if (snap.state === 'terminated' || snap.state === 'error') {
    let changed = false;
    const cleared: Record<string, IdeBreakpoint[]> = {};
    for (const [p, list] of Object.entries(host.breakpointsByPath)) {
      cleared[p] = list.map((bp) => {
        if (bp.verified !== undefined || bp.message !== undefined) {
          changed = true;
          const { verified: _v, message: _m, ...rest } = bp;
          return rest;
        }
        return bp;
      });
    }
    if (changed) host.breakpointsByPath = cleared;
    return;
  }

  if (!snap.breakpoints || snap.breakpoints.length === 0) return;
  const incomingByPath = new Map<string, IdeBreakpoint[]>();
  for (let i = 0; i < snap.breakpoints.length; i++) {
    const bp = snap.breakpoints[i];
    if (!bp.path) continue;
    const key = host.resolvePath ? host.resolvePath(bp.path) : bp.path;
    const arr = incomingByPath.get(key);
    if (arr) arr.push(bp);
    else incomingByPath.set(key, [bp]);
  }
  if (incomingByPath.size === 0) return;

  let changed = false;
  const nextMap: Record<string, IdeBreakpoint[]> = { ...host.breakpointsByPath };
  for (const [path, incoming] of incomingByPath.entries()) {
    const existing = nextMap[path];
    if (!existing || existing.length === 0) continue;
    const updated = existing.map((ubp, idx) => {
      const match =
        incoming.find((abp) => abp.line === ubp.line) ??
        (incoming.length === existing.length ? incoming[idx] : undefined);
      if (!match) return ubp;
      const nextVerified = typeof match.verified === 'boolean' ? match.verified : ubp.verified;
      const nextMsg = match.message !== undefined ? match.message : ubp.message;
      const nextLine = match.line > 0 ? match.line : ubp.line;
      if (nextVerified !== ubp.verified || nextMsg !== ubp.message || nextLine !== ubp.line) {
        changed = true;
        return { ...ubp, line: nextLine, verified: nextVerified, message: nextMsg };
      }
      return ubp;
    });
    nextMap[path] = updated;
  }
  if (changed) host.breakpointsByPath = nextMap;
}
