// Pure helpers of the artifact view's Versions panel, status pill and mirror
// banner — kept out of the components so they are unit-tested
// (unit/designVersionsPanel.test.ts) and the big view stays small.
import type { DesignArtifact, DesignVersion } from '../../lib/api/types';

/** A Versions-panel row: one version, or a run of autosaves folded into one. */
export type VersionRow =
  | { kind: 'version'; version: DesignVersion }
  | { kind: 'fold'; key: string; versions: DesignVersion[] };

/**
 * Newest first. A run of two or more consecutive `autosave` versions folds
 * into "N autosaves" (expand to see them); the head, the approved version and
 * any version in `keep` (the one being viewed) always stay their own row, so
 * the things a person acts on are never hidden inside a fold.
 */
export function versionRows(
  versions: readonly DesignVersion[],
  opts: { headId?: string | null; approvedId?: string | null; keep?: readonly string[]; expanded?: ReadonlySet<string> } = {},
): VersionRow[] {
  const sorted = [...versions].sort((a, b) => b.seq - a.seq);
  const pinned = new Set([opts.headId, opts.approvedId, ...(opts.keep ?? [])].filter(Boolean) as string[]);
  const out: VersionRow[] = [];
  let run: DesignVersion[] = [];
  const flush = () => {
    const key = run.length ? `fold:${run[0].id}` : '';
    if (run.length >= 2 && !opts.expanded?.has(key)) out.push({ kind: 'fold', key, versions: run });
    else for (const v of run) out.push({ kind: 'version', version: v });
    run = [];
  };
  for (const v of sorted) {
    if (v.kind === 'autosave' && !pinned.has(v.id)) {
      run.push(v);
      continue;
    }
    flush();
    out.push({ kind: 'version', version: v });
  }
  flush();
  return out;
}

/**
 * The version a save conflict is resolved against: the design's REAL head
 * (`artifact.head_version_id`), never merely the newest number — side
 * versions (variants, a source sync on an adopted design) are numbered too
 * but never move the head, so a save based on one is refused forever.
 * `recent` (newest first or not) is the fallback when the detail carries no
 * head row: the head among them, else the newest version on `main`.
 */
export function conflictHead(
  d: { artifact: Pick<DesignArtifact, 'head_version_id'>; head: DesignVersion | null },
  recent: readonly DesignVersion[] = [],
): DesignVersion | null {
  const headId = d.artifact.head_version_id;
  if (d.head && (!headId || d.head.id === headId)) return d.head;
  const byId = headId ? recent.find((v) => v.id === headId) : undefined;
  if (byId) return byId;
  return [...recent].filter((v) => v.branch === 'main').sort((a, b) => b.seq - a.seq)[0] ?? null;
}

/** "Named", "Autosave", "Otto", "Restore", "Sync", "Import" — the row's kind word. */
export function versionKindLabel(kind: string): string {
  switch (kind) {
    case 'named':
      return 'Named';
    case 'autosave':
      return 'Autosave';
    case 'agent':
      return 'Otto';
    case 'restore':
      return 'Restore';
    case 'sync':
      return 'Sync';
    case 'import':
      return 'Import';
    default:
      return kind;
  }
}

/**
 * What the header's status pill shows. An approved design edited after its
 * approval is still approved (followers keep the approved version), so it
 * says "Approved v5 · edited" instead of pretending to be a draft.
 */
export function statusPillOf(
  artifact: Pick<DesignArtifact, 'status' | 'approved_version_id'>,
  headId: string | null,
  approvedSeq: number | null,
): { status: string; suffix: string } {
  if (artifact.status === 'approved' && artifact.approved_version_id && headId && headId !== artifact.approved_version_id) {
    return { status: 'approved', suffix: approvedSeq != null ? ` v${approvedSeq} · edited` : ' · edited' };
  }
  if (artifact.status === 'approved' && approvedSeq != null) return { status: 'approved', suffix: ` v${approvedSeq}` };
  return { status: artifact.status, suffix: '' };
}

/** Read-only mirror of a Product story attachment or a Canvas scene, not yet adopted. */
export function isMirrored(a: Pick<DesignArtifact, 'source_kind' | 'meta'> | null | undefined): boolean {
  return !!a?.source_kind && a.meta?.adopted !== true;
}

/** A write refused because the design is an unadopted mirror (409 `mirrored`,
 *  `DESIGN_MIRRORED_CODE`). Duck-typed so it holds for any `ApiError`. */
export function isMirroredError(e: unknown): boolean {
  const x = e as { status?: unknown; code?: unknown } | null;
  return !!x && x.status === 409 && x.code === 'mirrored';
}

/** "a story attachment" / "a Canvas board" — what a mirror (or an adopted one) came from. */
export function mirrorSourceLabel(kind: string | null | undefined): string {
  return kind === 'canvas_scene' ? 'a Canvas board' : 'a story attachment';
}

/** `X-Total-Count` as a number (null when absent or not a count). */
export function totalCountOf(header: string | null): number | null {
  if (header == null || header.trim() === '') return null;
  const n = Number(header);
  return Number.isInteger(n) && n >= 0 ? n : null;
}

/** Merge a page of older versions under the ones held (newest first, no duplicates). */
export function mergeOlder(held: readonly DesignVersion[], page: readonly DesignVersion[]): DesignVersion[] {
  const have = new Set(held.map((v) => v.id));
  return [...held, ...page.filter((v) => !have.has(v.id))];
}

/** Autosave may run: an editable working copy that is not a mirror, archived, or read-only. */
export function canAutosave(o: { readonly: boolean; mirrored: boolean; archived: boolean; viewing: boolean }): boolean {
  return !o.readonly && !o.mirrored && !o.archived && !o.viewing;
}

/** The save indicator's words. `pending` = an edit waiting for its debounced save. */
export function saveIndicatorText(state: 'idle' | 'pending' | 'saving' | 'saved' | 'error', dirty: boolean): string {
  if (state === 'saving' || state === 'pending') return 'Saving…';
  if (state === 'error' || dirty) return 'Couldn’t save';
  return 'Saved';
}
