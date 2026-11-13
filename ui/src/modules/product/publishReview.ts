// Pure helpers behind the Product publish review (PublishReviewDialog): WHERE
// a publish lands (the site + the issue/page) and whether a rewrite was
// written against an older copy of the story. No Svelte, no API client — the
// unit test drives them with plain objects.

/** The site a story lives on, from its URL ("acme.atlassian.net"); '' when
 *  the URL is missing or not a URL. */
export function storySite(url: string | null | undefined): string {
  if (!url) return '';
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}

/** The slice of a version row this needs. */
export interface VersionRef {
  kind: string;
  version_no: number;
}

/** True when a `source` version (an import or Refresh of what Jira/Confluence
 *  holds) is NEWER than the suggestion: the rewrite was made from an older
 *  copy, so publishing it replaces whatever changed upstream since — e.g. the
 *  teammate's edit a conflict-Refresh just pulled in. `version_no` is per
 *  story and only grows, so it orders rows without trusting clocks. */
export function suggestionIsStale(suggested: VersionRef | null | undefined, versions: readonly VersionRef[], source?: VersionRef | null): boolean {
  if (!suggested) return false;
  const newer = (v: VersionRef | null | undefined) => !!v && v.kind === 'source' && v.version_no > suggested.version_no;
  return newer(source) || versions.some(newer);
}

/** The quiet line a stale suggestion carries (Rewrite tab and its confirm). */
export const STALE_SUGGESTION = 'Based on an older version of the story';
