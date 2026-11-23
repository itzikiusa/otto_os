// Stream, watch-evaluation, and snapshot/breakpoint normalization helpers for IDE DAP sessions.

import { api } from '../../../lib/api/client';
import { loadErrorText } from '../../../lib/loadError';
import { canonicalizeLocalProjectPath } from '../shell/ideProjectHelpers';
import type { DebugAndEnvHost } from './ideDebugAndEnvController';
import {
  ideTargetKey,
  streamIdeSse,
  type IdeBreakpoint,
  type IdeDebugSessionSnapshot,
  type IdeProjectRoot,
  type IdeReplEntry,
  type IdeRunState,
  type IdeTargetSpec,
  type IdeVariable,
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
  const key = host.target ? ideTargetKey(host.target) : null;
  const stale = (): boolean => key !== null && Boolean(host.target) && ideTargetKey(host.target) !== key;
  host.debugStreamActive = false;
  void streamIdeSse(`/ide/debug/sessions/${encodeURIComponent(sessionId)}/events`, {}, ctl.signal, (raw) => {
    if (stale()) { ctl.abort(); return; }
    try {
      const parsed = JSON.parse(raw) as { session?: IdeDebugSessionSnapshot } & Partial<IdeDebugSessionSnapshot>;
      const snap = (parsed.session ?? parsed) as IdeDebugSessionSnapshot;
      if (snap?.id) onSnapshot(snap);
    } catch { /* heartbeat */ }
  }, () => {
    if (stale()) { ctl.abort(); return; }
    host.debugStreamActive = true;
  }).catch(() => {}).finally(() => { if (!stale()) host.debugStreamActive = false; });
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
    ...(raw.selected_frame_file ? { selected_frame_file: normPath(raw.selected_frame_file) } : {}),
    stack_frames,
    breakpoints,
    output,
  };
  return { snap, paused };
}

/** DAP variable references are only valid for one stop, and delve hands out the
 *  same numbers again, so every new stop (`stop_seq`, even on the same line of a
 *  loop) or frame drops every cached expansion and seeds each scope from the
 *  snapshot. Keyed on the old numbers, Locals kept showing the first stop's
 *  variables after Continue. */
export function syncStopVariables(
  host: { expandedVariables: Record<number, IdeVariable[]> },
  prev: IdeDebugSessionSnapshot | null,
  snap: IdeDebugSessionSnapshot,
): void {
  const stop = (s: IdeDebugSessionSnapshot | null): string =>
    s?.state === 'paused' ? `${s.id}:${s.stop_seq}:${s.active_thread_id}:${s.active_frame_id}:${s.paused_file}:${s.paused_line}` : '';
  if (snap.state !== 'paused' || stop(prev) === stop(snap)) return;
  const fresh: Record<number, IdeVariable[]> = {};
  for (const sc of snap.scopes ?? []) if (sc.variables_reference > 0 && sc.variables?.length) fresh[sc.variables_reference] = sc.variables;
  host.expandedVariables = fresh;
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

/** Where the editor follows a paused session: the frame the person selected,
 *  else the stop itself (`selected_frame_*` is absent on daemons before it). */
export function debugRevealTarget(
  snap: Pick<IdeDebugSessionSnapshot, 'state' | 'paused_file' | 'paused_line' | 'selected_frame_file' | 'selected_frame_line'> | null | undefined,
): { path: string; line: number; key: string } | null {
  if (!snap || snap.state !== 'paused') return null;
  const path = snap.selected_frame_file || snap.paused_file;
  const line = snap.selected_frame_file ? snap.selected_frame_line : snap.paused_line;
  return path && line ? { path, line, key: `${path}:${line}` } : null;
}

/** A different session arrived: the old one's REPL results and watch values
 *  describe another process. Input history and the watch expressions stay. */
export function resetForNewDebugSession(
  host: { replEntries: IdeReplEntry[]; watches: IdeWatch[] },
  prev: Pick<IdeDebugSessionSnapshot, 'id'> | null | undefined,
  next: Pick<IdeDebugSessionSnapshot, 'id'>,
): void {
  if (!prev || prev.id === next.id) return;
  if (host.replEntries.length > 0) host.replEntries = [];
  if (host.watches.some((w) => w.value !== '…' || w.error)) {
    host.watches = host.watches.map((w) => ({ ...w, value: '…', var_type: null, error: null }));
  }
}

/** Sequence number of `run.output[0]`: the daemon counts every line ever
 *  written (`output_seq`), and both it and the client trim from the front, so
 *  line `i` is `base + i`. A console clears/keys by this, not by index. */
export function runOutputSeqBase(run: Pick<IdeRunState, 'output' | 'output_seq'>): number {
  return Math.max(1, (run.output_seq ?? run.output.length) - run.output.length + 1);
}
