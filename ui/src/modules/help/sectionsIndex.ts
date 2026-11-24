// Help's guide index, in the page: every guide's title, group, route,
// summary and shortcuts — the rail, the default guide and ⌘K need only that.
// The guide text is one lazy chunk (./sections.ts) fetched when Help opens.
// The film manifest is globbed too, so a build without
// `lib/walkthroughs/film.json` still compiles (no film, no crash).
import index from 'virtual:help-guide-index';
import { DEFAULT_GUIDE, parseFilmManifest, type FilmManifest, type GuideSection } from './guide';
import { buildGuides } from './guideList';

/** Every guide in rail order, without its text (`body` holds only the
 *  Keyboard shortcuts table). */
export const GUIDE_INDEX: GuideSection[] = buildGuides(index);

export function guideIndexById(id: string | undefined): GuideSection | undefined {
  return id ? GUIDE_INDEX.find((g) => g.id === id) : undefined;
}

/** The guide the page opens on: Getting started, else the first one. */
export const DEFAULT_GUIDE_ID: string = guideIndexById(DEFAULT_GUIDE)?.id ?? GUIDE_INDEX[0]?.id ?? DEFAULT_GUIDE;

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
