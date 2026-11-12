// Product → story → Design tab: the pure half of StoryDesigns (no Svelte, no
// fetch — unit-tested in ui/unit/storyDesigns.test.ts). A story's
// designs live in Design Hall; these helpers name the routes the tab opens
// and the counts it shows.
import { pluralNoun } from '../../lib/plural';

/** The artifact route opened FROM a story: the artifact view reads
 *  `from=story/<id>` from the hash and shows a breadcrumb back to that story.
 *  `tab` (`otto`) opens a side tab, as the lobby's Generate does. */
export function storyArtifactRoute(artifactId: string, storyId: string, tab?: 'otto'): string {
  return `design/a/${encodeURIComponent(artifactId)}${tab ? `/${tab}` : ''}?from=story/${encodeURIComponent(storyId)}`;
}

/**
 * A design card's link (`#/design/a/<id>`, the shared ArtifactCard's href) as
 * the same artifact opened from this story, or null for any other link (a
 * group heading, a story page) — those keep their own target.
 */
export function storyLinkRoute(href: string | null | undefined, storyId: string): string | null {
  const m = /^#\/design\/a\/([^/?#]+)$/.exec(href ?? '');
  if (!m) return null;
  let id: string;
  try {
    id = decodeURIComponent(m[1]);
  } catch {
    return null;
  }
  return storyArtifactRoute(id, storyId);
}

/** The Design Hall page of a story's designs (an epic: its children's too). */
export function storyHallRoute(storyId: string): string {
  return `design/story/${encodeURIComponent(storyId)}`;
}

/** "3 designs" / "1 design" / "No designs yet"; `more` when the count is a
 *  capped page (the line says "100+"). */
export function designCountLabel(n: number, more = false): string {
  if (n <= 0) return 'No designs yet';
  // A capped page ("1+") always reads as more than one.
  return `${n}${more ? '+' : ''} ${pluralNoun(more ? Infinity : n, 'design')}`;
}
