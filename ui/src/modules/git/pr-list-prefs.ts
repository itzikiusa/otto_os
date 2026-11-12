// PrList's per-repo preferences (sort, state chip, filters, folded groups)
// and the Sort / Filters menu items built from them. Kept out of the
// component so PrList stays a layout file.
import type { PrState } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { SORT_LABEL, type CiFilter, type PrSort } from './pr-query';

export type DraftFilter = 'any' | 'hide' | 'only';

export interface PrListPrefs {
  sort: PrSort;
  state: PrState;
  draft: DraftFilter;
  ci: CiFilter | null;
  author: string | null;
  mine: boolean;
  /** Only open PRs that ask you for a review — a SERVER filter
   *  (`review=me`, the forge's own stable-id match), so it pages correctly. */
  review: boolean;
  folded: string[];
}

export const DEFAULT_PREFS: PrListPrefs = {
  sort: 'updated',
  state: 'open',
  draft: 'any',
  ci: null,
  author: null,
  mine: false,
  review: false,
  folded: [],
};

/** The filter fields "Clear filters" resets (sort/state/folds stay). */
export const CLEARED_FILTERS: Partial<PrListPrefs> = { draft: 'any', ci: null, author: null, mine: false, review: false };

const prefsKey = (id: string) => `otto_prlist_prefs_${id}`;

export function readPrefs(id: string): PrListPrefs {
  try {
    const raw = localStorage.getItem(prefsKey(id));
    return raw ? { ...DEFAULT_PREFS, ...(JSON.parse(raw) as Partial<PrListPrefs>) } : { ...DEFAULT_PREFS };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export function writePrefs(id: string, p: PrListPrefs): void {
  try {
    localStorage.setItem(prefsKey(id), JSON.stringify(p));
  } catch {
    /* blocked storage: the choice just doesn't persist */
  }
}

export const activeFilters = (p: PrListPrefs): number =>
  (p.draft !== 'any' ? 1 : 0) + (p.ci ? 1 : 0) + (p.author ? 1 : 0) + (p.mine ? 1 : 0) + (p.review ? 1 : 0);

/** The list's server query: a state, or `<state>~review` for "review
 *  requested from you" (its own cache key — those rows are another list). */
export const listToken = (state: PrState, review: boolean): string => (review ? `${state}~review` : state);

/** `GET /repos/{id}/prs` for a {@link listToken}. The review filter lists
 *  open PRs only, whatever the state chip says. */
export function prListUrl(repoId: string, token: string, page: number, perPage: number): string {
  const [state, review] = token.split('~');
  return review
    ? `/repos/${repoId}/prs?state=open&review=me&page=${page}&per_page=${perPage}`
    : `/repos/${repoId}/prs?state=${state}&page=${page}&per_page=${perPage}`;
}

export function sortItems(p: PrListPrefs, anyStats: boolean, set: (x: Partial<PrListPrefs>) => void): MenuItem[] {
  const order: PrSort[] = ['updated', 'newest', 'oldest', 'changed', 'number', 'title'];
  return order
    .filter((s) => s !== 'changed' || anyStats || p.sort === 'changed')
    .map((s) => ({ label: SORT_LABEL[s], checked: p.sort === s, action: () => set({ sort: s }) }));
}

/** The Filters menu; `pickAuthor` opens the (filterable) author sub-menu. */
export function filterItems(
  p: PrListPrefs,
  opts: { hasHandles: boolean; authors: string[]; pickAuthor: () => void },
  set: (x: Partial<PrListPrefs>) => void,
): MenuItem[] {
  const ci = (v: CiFilter, label: string): MenuItem => ({
    label,
    checked: p.ci === v,
    action: () => set({ ci: p.ci === v ? null : v }),
  });
  const items: MenuItem[] = [];
  if (opts.hasHandles) items.push({ label: 'Authored by you', icon: 'user', checked: p.mine, action: () => set({ mine: !p.mine }) });
  items.push({ label: 'Review requested from you', icon: 'eye', hint: 'open', checked: p.review, action: () => set({ review: !p.review }) });
  items.push({
    label: p.author ? `Author: ${p.author}` : 'Author…',
    icon: 'people',
    disabled: opts.authors.length === 0,
    action: opts.pickAuthor,
  });
  items.push({ separator: true });
  items.push({ label: 'Hide drafts', checked: p.draft === 'hide', action: () => set({ draft: p.draft === 'hide' ? 'any' : 'hide' }) });
  items.push({ label: 'Only drafts', checked: p.draft === 'only', action: () => set({ draft: p.draft === 'only' ? 'any' : 'only' }) });
  items.push({ separator: true });
  items.push(ci('passing', 'CI passing'), ci('failing', 'CI failing'), ci('pending', 'CI running'));
  if (activeFilters(p) > 0) {
    items.push({ separator: true });
    items.push({ label: 'Clear filters', icon: 'x', action: () => set({ ...CLEARED_FILTERS }) });
  }
  return items;
}

export function authorItems(p: PrListPrefs, authors: string[], set: (x: Partial<PrListPrefs>) => void): MenuItem[] {
  return [
    { label: 'Anyone', checked: p.author === null, pinned: true, action: () => set({ author: null }) },
    ...authors.map((a) => ({ label: a, checked: p.author === a, action: () => set({ author: a }) })),
  ];
}
