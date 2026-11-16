// Pure preference, range, segment-grouping, line-selection, and hunk-apply
// helpers for `DiffViewer.svelte` and `DiffRowView.svelte`.

import { api } from '../api/client';
import type { DiffLine, DiffResp, FileDiff, Hunk, HunkOp, StageHunkResp } from '../api/types';
import { confirmer } from '../confirm.svelte';
import { plural } from '../plural';
import { toasts } from '../toast.svelte';
import { stageErrorToast } from './diff-dom';
import { buildFileRows, fileStat, inHunk, ROW_KEY_SEP, type Row } from './diff-model';
import { findScroller } from './diff-virtual';

const PREF_SPLIT = 'otto_diff_split';

export function readSplitPref(): boolean {
  try {
    return localStorage.getItem(PREF_SPLIT) === '1';
  } catch {
    return false;
  }
}

export function writeSplitPref(v: boolean): void {
  try {
    localStorage.setItem(PREF_SPLIT, v ? '1' : '0');
  } catch {
    /* ignore blocked storage */
  }
}

export function formatDiffBytes(n: number): string {
  return n >= 1024 * 1024
    ? `${(n / 1024 / 1024).toFixed(1)} MB`
    : `${Math.max(1, Math.round(n / 1024))} KB`;
}

export function parseRange(r: string): [number, number] {
  const c = r.indexOf(':');
  return [Number(r.slice(0, c)), Number(r.slice(c + 1))];
}

export function computeDiffTotals(diff: DiffResp): { add: number; del: number } {
  if (diff.total_added != null && diff.total_deleted != null) {
    return { add: diff.total_added, del: diff.total_deleted };
  }
  let add = 0;
  let del = 0;
  for (let i = 0; i < diff.files.length; i++) {
    const s = fileStat(diff.files[i]);
    add += s.add;
    del += s.del;
  }
  return { add, del };
}

export interface DiffSegment {
  key: string;
  group: boolean;
  split: boolean;
  items: { r: Row; i: number }[];
}

export function buildDiffSegments(
  rs: Row[],
  s: number,
  e: number,
  splitMode: boolean,
): DiffSegment[] {
  const out: DiffSegment[] = [];
  let g: DiffSegment | null = null;
  for (let i = s; i < e && i < rs.length; i++) {
    const r = rs[i];
    if (inHunk(r)) {
      const gk = `${r.file.path}${ROW_KEY_SEP}g${r.hi}`;
      if (!g || g.key !== gk) {
        g = { key: gk, group: true, split: r.kind === 'split' || splitMode, items: [] };
        out.push(g);
      }
      g.items.push({ r, i });
    } else {
      g = null;
      out.push({ key: r.key, group: false, split: false, items: [{ r, i }] });
    }
  }
  return out;
}

export interface DiffLineSelection {
  path: string;
  hunk: number;
  lines: Set<number>;
  anchor: number;
}

export function toggleDiffLineSelection(
  cur: DiffLineSelection | null,
  shiftKey: boolean,
  path: string,
  hi: number,
  li: number,
  line: DiffLine,
  hunkLines?: DiffLine[],
): DiffLineSelection | null {
  if (line.origin === 'context') return cur;
  const active = cur !== null && cur.path === path && cur.hunk === hi ? cur : null;
  if (shiftKey && active) {
    const [a, b] = active.anchor <= li ? [active.anchor, li] : [li, active.anchor];
    const lines = new Set(active.lines);
    for (let i = a; i <= b; i++) {
      if (!hunkLines || hunkLines[i]?.origin !== 'context') lines.add(i);
    }
    return { path, hunk: hi, lines, anchor: active.anchor };
  }
  const lines = active ? new Set(active.lines) : new Set<number>();
  if (lines.has(li)) lines.delete(li);
  else lines.add(li);
  return lines.size === 0 ? null : { path, hunk: hi, lines, anchor: li };
}

export async function runStageOrDiscardHunk(opts: {
  repoId: string;
  file: FileDiff;
  hi: number;
  hunk: Hunk;
  op: HunkOp;
  sel: DiffLineSelection | null;
  isStale: () => boolean;
}): Promise<StageHunkResp | null> {
  const { repoId, file, hi, hunk, op, sel, isStale } = opts;
  const picked =
    sel !== null && sel.path === file.path && sel.hunk === hi && sel.lines.size > 0
      ? [...sel.lines].sort((a, b) => a - b)
      : undefined;
  if (op === 'discard') {
    const what = picked ? plural(picked.length, 'changed line') : 'this whole hunk';
    const ok = await confirmer.ask(
      `Discard ${what}? This rewrites the working file and cannot be undone from the file — a backup stash \`otto: backup before hunk discard\` is kept.`,
      { title: picked ? 'Discard lines' : 'Discard hunk', confirmLabel: 'Discard', danger: true },
    );
    if (!ok) return null;
    if (isStale()) {
      toasts.info(
        'Diff changed',
        'The file changed while you were confirming — nothing was discarded. Pick the lines again.',
      );
      return null;
    }
  }
  try {
    return await api.post<StageHunkResp>(`/repos/${repoId}/stage-hunk`, {
      path: file.path,
      hunk_index: hi,
      hunk_header: hunk.header,
      fingerprint: file.fingerprint,
      lines: picked,
      op,
      confirm: op === 'discard' ? true : undefined,
    });
  } catch (e) {
    stageErrorToast(e);
    return null;
  }
}

export function activeDiffScroller(bodyEl: HTMLDivElement | undefined): HTMLElement | null {
  if (!bodyEl) return null;
  for (let p = findScroller(bodyEl); p; p = findScroller(p)) {
    if (p.scrollHeight > p.clientHeight + 1) return p;
  }
  return null;
}

export type DiffRowCache = Map<string, { deps: unknown[]; rows: Row[] }>;

const sameDeps = (a: unknown[], b: unknown[]) => a.every((x, i) => x === b[i]);

export function buildCachedDiffRows(opts: {
  files: FileDiff[];
  overrides: Record<string, boolean>;
  defaultCollapsed: (f: FileDiff) => boolean;
  uncappedMap: Map<string, Set<number>>;
  loadedMap: Map<string, FileDiff>;
  errorsMap: Map<string, string>;
  fullSet: Set<string>;
  split: boolean;
  canLoad: boolean;
  prevCache: DiffRowCache;
}): { rows: Row[]; nextCache: DiffRowCache } {
  const {
    files,
    overrides,
    defaultCollapsed,
    uncappedMap,
    loadedMap,
    errorsMap,
    fullSet,
    split,
    canLoad,
    prevCache,
  } = opts;
  const out: Row[] = [];
  const nextCache: DiffRowCache = new Map();
  for (let fi = 0; fi < files.length; fi++) {
    const file = files[fi];
    const eff = loadedMap.get(file.path) ?? file;
    const collapsed = overrides[file.path] ?? defaultCollapsed(file);
    const uncapped = uncappedMap.get(file.path);
    const loadError = errorsMap.get(file.path);
    const forceFull = fullSet.has(file.path);
    const deps = [eff, collapsed, fi === 0, split, uncapped, loadError, forceFull, canLoad];
    const hit = prevCache.get(file.path);
    const fr =
      hit && sameDeps(hit.deps, deps)
        ? hit.rows
        : buildFileRows({
            file,
            eff,
            collapsed,
            first: fi === 0,
            split,
            fc: undefined,
            uncapped,
            composer: null,
            loadError,
            forceFull,
            canLoad,
          });
    nextCache.set(file.path, { deps, rows: fr });
    for (let j = 0; j < fr.length; j++) out.push(fr[j]);
  }
  return { rows: out, nextCache };
}
