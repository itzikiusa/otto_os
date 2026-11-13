// Pure helpers for a Hall copy that can fall behind its source — a Canvas
// board or a story attachment the daemon mirrors live (crates/otto-design
// `import.rs`). Kept out of the components so they are unit-tested
// (unit/designMirrorSync.test.ts).
//
// - `meta.sync_error {reason, at}`: the source changed but could not be
//   copied (too large with its images, an image missing, a read error); the
//   copy keeps its last good content and says it is behind. The daemon clears
//   it on the next successful sync.
// - After "Edit here" the Hall owns the head: a later source change (often an
//   agent drawing on the Canvas board) lands as a side version
//   `variant/sync-<ts>/1` that never moves the head — the banner points at it.
import type { DesignArtifact, DesignVersion } from '../../lib/api/types';

/** Why a Hall copy is behind its source, or null when it is in step. */
export function syncErrorOf(a: Pick<DesignArtifact, 'meta'> | null | undefined): { reason: string; at: string | null } | null {
  const raw = a?.meta?.sync_error as { reason?: unknown; at?: unknown } | null | undefined;
  if (!raw || typeof raw !== 'object') return null;
  const reason = typeof raw.reason === 'string' && raw.reason.trim() ? raw.reason.trim() : 'its latest change couldn’t be copied';
  return { reason, at: typeof raw.at === 'string' ? raw.at : null };
}

/** "Canvas board" / "story attachment" — the source's noun. */
export function sourceNoun(kind: string | null | undefined): string {
  return kind === 'canvas_scene' ? 'Canvas board' : 'story attachment';
}

/** A side version the source sync wrote (`variant/sync-…`, kind `sync`). */
export function isSourceSideVersion(v: Pick<DesignVersion, 'branch' | 'kind'>): boolean {
  return typeof v.branch === 'string' && v.branch.startsWith('variant/sync-');
}

/** A Versions-panel row's note for a version that is not on the main line. */
export function sideVersionNote(v: Pick<DesignVersion, 'branch' | 'kind'>, sourceKind: string | null | undefined): string | null {
  if (!v.branch || v.branch === 'main') return null;
  return isSourceSideVersion(v) ? `from the ${sourceNoun(sourceKind)}` : 'variant';
}

/**
 * The newest change from the source that arrived AFTER the person took the
 * design over and that nobody accepted yet (an accepted variant leaves a main
 * version whose `provenance.accepted_variant.version_id` names it). Null for a
 * design that is not adopted, or whose source is gone.
 */
export function pendingSourceVersion(
  versions: readonly DesignVersion[],
  a: Pick<DesignArtifact, 'source_kind' | 'meta'> | null | undefined,
): DesignVersion | null {
  if (!a?.source_kind || a.meta?.adopted !== true) return null;
  const at = a.meta?.adopted_at;
  const adoptedAt = typeof at === 'string' ? Date.parse(at) : NaN;
  const accepted = new Set(
    versions
      .map((v) => (v.provenance?.accepted_variant as { version_id?: unknown } | undefined)?.version_id)
      .filter((id): id is string => typeof id === 'string'),
  );
  const side = versions
    .filter((v) => isSourceSideVersion(v) && !accepted.has(v.id))
    .filter((v) => Number.isNaN(adoptedAt) || Date.parse(v.created_at) >= adoptedAt)
    .sort((x, y) => y.seq - x.seq);
  return side[0] ?? null;
}

/** What `#/canvas` learned about a board's Hall copy (nav.ts `findSceneHallCopy`). */
export type SceneHallCopy = { id: string; adopted: boolean; behind: string | null };

/** Open the Hall copy? Only one that is in step — or adopted (the Hall owns it;
 *  its banner says it is behind). Anything else opens in the Canvas editor. */
export function opensInHall(copy: SceneHallCopy | null | 'unavailable'): copy is SceneHallCopy {
  return !!copy && copy !== 'unavailable' && (!copy.behind || copy.adopted);
}

/** The one line above the Canvas editor saying why the board opened there. */
export function legacyEditorNote(copy: SceneHallCopy | null | 'unavailable'): string {
  if (copy === 'unavailable') return 'Design Hall can’t show this board, so it opens in the Canvas editor. Your changes save to the board.';
  if (!copy) return 'This board isn’t in Design Hall yet (it may be too large to copy there), so it opens in the Canvas editor. Your changes save to the board.';
  if (copy.behind) return `Design Hall’s copy of this board is behind (${copy.behind}), so it opens in the Canvas editor. Your changes save to the board.`;
  return 'This board opens in the Canvas editor. Your changes save to the board.';
}
