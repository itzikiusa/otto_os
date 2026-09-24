// Help → the guides and the tour film, loaded at build time.
//
// Every `./sections/*.md` file is bundled as a raw string (Vite glob, eager) and
// parsed once; the page lists whatever files exist, so a guide added by another
// branch shows up with no code change. The film manifest is globbed too, so a
// build without `lib/walkthroughs/film.json` still compiles (no film, no crash).
// The pure parsing lives in ./guide.ts (unit-tested).

import { SIDEBAR_MODULES, groupLabel } from '../../lib/sidebar';
import {
  DEFAULT_GUIDE,
  normalizeGroup,
  orderSections,
  parseFilmManifest,
  parseSection,
  type FilmManifest,
  type GuideSection,
} from './guide';

const files = import.meta.glob<string>('./sections/*.md', { query: '?raw', import: 'default', eager: true });

/** A module guide whose frontmatter group is missing/unknown lands in its
 *  sidebar section. */
function sidebarGroup(id: string) {
  const def = SIDEBAR_MODULES.find((m) => m.id === id);
  return def ? normalizeGroup(groupLabel(def.group)) : null;
}

function load(): GuideSection[] {
  const seen = new Set<string>();
  const out: GuideSection[] = [];
  for (const [path, raw] of Object.entries(files)) {
    const s = parseSection(path, raw, sidebarGroup);
    if (!s || seen.has(s.id)) continue; // unusable or a duplicate id: first wins
    seen.add(s.id);
    out.push(s);
  }
  // Sidebar order; the Database Explorer and Message Brokers views (opened
  // from Connections, no rows of their own) read right after Connections.
  const order = SIDEBAR_MODULES.flatMap((m) => (m.id === 'connections' ? [m.id, 'database', 'brokers'] : [m.id]));
  return orderSections(out, order);
}

/** Every guide, in rail order (group, then Basics reading order / sidebar order). */
export const GUIDES: GuideSection[] = load();

export function guideById(id: string | undefined): GuideSection | undefined {
  return id ? GUIDES.find((g) => g.id === id) : undefined;
}

/** The guide the page opens on: Getting started, else the first one. */
export const DEFAULT_GUIDE_ID: string = guideById(DEFAULT_GUIDE)?.id ?? GUIDES[0]?.id ?? DEFAULT_GUIDE;

const filmFiles = import.meta.glob<unknown>('../../lib/walkthroughs/film.json', { import: 'default', eager: true });

/** The single tour film, or null when this build has no (usable) manifest. */
export const FILM: FilmManifest | null = parseFilmManifest(Object.values(filmFiles)[0]);

/** Captions bundled with the app (`lib/walkthroughs/<name>.vtt`), keyed by file
 *  name. A bundled track plays from a same-origin blob: URL, which works even
 *  when the video host sends no CORS headers (GitHub release assets don't, so a
 *  remote <track> there can never load). */
export const BUNDLED_CAPTIONS: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>('../../lib/walkthroughs/*.vtt', { query: '?raw', import: 'default', eager: true }),
  ).map(([p, raw]) => [p.split('/').pop() ?? p, raw]),
);
