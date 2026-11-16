import type { DiffResp, FileDiff } from '../api/types.ts';
import type { ComposerAt } from './diff-model.ts';

export interface CarriedState {
  overrides: Record<string, boolean>;
  composer: ComposerAt | null;
  uncapped: Map<string, Set<number>>;
  full: Set<string>;
}

export function carryViewState(prev: CarriedState, paths: ReadonlySet<string>): CarriedState {
  const overrides: Record<string, boolean> = {};
  for (const [p, v] of Object.entries(prev.overrides)) if (paths.has(p)) overrides[p] = v;
  const uncapped = new Map<string, Set<number>>();
  for (const [p, v] of prev.uncapped) if (paths.has(p)) uncapped.set(p, v);
  return {
    overrides,
    composer: prev.composer && paths.has(prev.composer.path) ? prev.composer : null,
    uncapped,
    full: new Set([...prev.full].filter((p) => paths.has(p))),
  };
}

export function carryViewed(prev: ReadonlySet<string>, paths: ReadonlySet<string>): Set<string> {
  return new Set([...prev].filter((p) => paths.has(p)));
}

export function fileStamp(f: FileDiff): string {
  if (f.fingerprint) return `fp:${f.fingerprint}`;
  return [
    'sum',
    f.status ?? '',
    f.old_path ?? '',
    f.added ?? '',
    f.deleted ?? '',
    f.is_binary ? 'bin' : '',
    f.hunks.map((h) => h.header).join('\u001f'),
  ].join('\u001e');
}

export function unchangedPaths(prev: DiffResp | null, next: DiffResp): Set<string> {
  const out = new Set<string>();
  if (!prev) return out;
  const before = new Map(prev.files.map((f) => [f.path, fileStamp(f)]));
  for (const f of next.files) if (before.get(f.path) === fileStamp(f)) out.add(f.path);
  return out;
}
