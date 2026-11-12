// Design Hall — the pure half of DesignGrid (no Svelte, no fetch): how a
// scope + the person's filter chips become ONE server query, whether a
// live-patched card still belongs in the grid, and how loaded cards group
// (Recent · Story · Project). Unit-tested in ui/unit/designGrid.test.ts.

import type { DesignListFilter } from '../../lib/api/design';
import type { DesignArtifact, DesignSearchHit, DesignStatus, DesignStudio } from '../../lib/api/types';
import type { IconName } from '../../lib/components/Icon.svelte';

/** The badge of a MIRRORED design (`source_kind` set): where it is made and
 *  kept in sync from. `null` for a design that lives in Design Hall. */
export function mirrorLabel(kind: DesignArtifact['source_kind'] | string | undefined): { label: string; icon: IconName } | null {
  if (kind === 'product_attachment') return { label: 'From a story', icon: 'ticket' };
  if (kind === 'canvas_scene') return { label: 'From Canvas', icon: 'shapes' };
  return kind ? { label: 'Mirrored', icon: 'link' } : null;
}

export type GroupBy = 'recent' | 'story' | 'project';

/** The filter chips over a grid. `archived` shows ONLY archived designs (the
 *  one place they can be found and restored); `mine` is matched on the
 *  client — the list API has no author-id filter — so it narrows the pages
 *  loaded so far and "Load more" keeps paging. */
export interface GridFilters {
  studio: DesignStudio | '';
  status: Exclude<DesignStatus, 'archived'> | '';
  mine: boolean;
  archived: boolean;
}

export const NO_FILTERS: GridFilters = { studio: '', status: '', mine: false, archived: false };

/** The scope a grid is pinned to (a project, a studio, a story — with an
 *  epic's children — or nothing: every design the caller may view). */
export type GridScope = Pick<DesignListFilter, 'workspace_id' | 'project_id' | 'studio' | 'story_id' | 'include_children'>;

export function activeFilterCount(f: GridFilters): number {
  return (f.studio ? 1 : 0) + (f.status ? 1 : 0) + (f.mine ? 1 : 0) + (f.archived ? 1 : 0);
}

/** Scope + chips → the `GET /design/search?q=` filter (keyset-paged by the
 *  caller). The scope wins over a chip that would contradict it. */
export function gridQuery(scope: GridScope, f: GridFilters): DesignListFilter {
  const q: DesignListFilter = { ...scope }; // `qs` drops the empty keys
  if (f.studio && !scope.studio) q.studio = f.studio;
  if (f.archived) {
    q.status = 'archived';
    q.include_archived = true;
  } else if (f.status) {
    q.status = f.status;
  }
  return q;
}

/** Does a (live-patched) artifact still belong in this grid? A status /
 *  project / studio change can move a card out (archived from a live list,
 *  unarchived from the Archived list). Story membership arrives as link
 *  events, which reload the grid instead. */
export function matchesGrid(a: DesignArtifact, scope: GridScope, f: GridFilters, meId: string | null | undefined): boolean {
  if (scope.project_id && a.project_id !== scope.project_id) return false;
  const studio = scope.studio || f.studio;
  if (studio && a.studio !== studio) return false;
  if (f.archived) {
    if (a.status !== 'archived') return false;
  } else if (a.status === 'archived' || (f.status && a.status !== f.status)) {
    return false;
  }
  if (f.mine && (!meId || a.created_by !== meId)) return false;
  return true;
}

/** Everything a card shows of an artifact (a live patch that changes none of
 *  it leaves the grid untouched — no re-render, no thumbnail refetch). */
export function sameCard(a: DesignArtifact, b: DesignArtifact): boolean {
  return (
    a.updated_at === b.updated_at &&
    a.title === b.title &&
    a.status === b.status &&
    a.project_id === b.project_id &&
    a.studio === b.studio &&
    a.head_version_id === b.head_version_id &&
    a.thumb_blob === b.thumb_blob &&
    a.source_kind === b.source_kind
  );
}

/** The keyset cursor after the last loaded row (`updated_at|id`, newest first). */
export function cursorAfter(hits: readonly DesignSearchHit[]): string | undefined {
  const last = hits[hits.length - 1]?.artifact;
  return last ? `${last.updated_at}|${last.id}` : undefined;
}

export interface CardGroup {
  key: string;
  /** Section heading; `null` for the single flat Recent group. */
  label: string | null;
  /** Where the heading links (a story or project page), if anywhere. */
  href: string | null;
  hits: DesignSearchHit[];
}

export interface GroupLabels {
  story: (id: string) => { key: string; title: string } | undefined;
  project: (id: string) => string | undefined;
}

/**
 * Group LOADED cards. Recent is one flat group (server order, newest first).
 * Story: one section per linked story — a design implementing two stories
 * shows under both — then "No story". Project: one section per project, then
 * "Unfiled". Sections keep first-seen order, i.e. the newest work leads.
 */
export function groupHits(hits: readonly DesignSearchHit[], by: GroupBy, labels: GroupLabels): CardGroup[] {
  if (by === 'recent') return [{ key: 'recent', label: null, href: null, hits: [...hits] }];
  const groups = new Map<string, CardGroup>();
  const none: DesignSearchHit[] = [];
  const add = (key: string, mk: () => Omit<CardGroup, 'hits' | 'key'>, h: DesignSearchHit) => {
    let g = groups.get(key);
    if (!g) groups.set(key, (g = { key, ...mk(), hits: [] }));
    g.hits.push(h);
  };
  for (const h of hits) {
    if (by === 'story') {
      const ids = [...new Set(h.story_ids)];
      if (!ids.length) none.push(h);
      for (const sid of ids) {
        add(`s:${sid}`, () => {
          const s = labels.story(sid);
          return { label: s ? `${s.key} · ${s.title}` : 'A linked story', href: `#/design/story/${encodeURIComponent(sid)}` };
        }, h);
      }
    } else {
      const pid = h.artifact.project_id;
      if (!pid) none.push(h);
      else add(`p:${pid}`, () => ({ label: labels.project(pid) ?? 'A project', href: `#/design/p/${encodeURIComponent(pid)}` }), h);
    }
  }
  const out = [...groups.values()];
  if (none.length) out.push({ key: 'none', label: by === 'story' ? 'No story' : 'Unfiled', href: null, hits: none });
  return out;
}
