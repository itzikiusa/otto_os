// Reader view links: the extracted page keeps its links as markdown, and the
// sanitizing renderer turns them into `<a target="_blank">`. A click must
// instead move THIS tab through the browser's normal (daemon-guarded)
// navigation, and only ever to a web page — never `javascript:`, `file:`,
// `data:` or an app scheme.

/** What a click on a reader-view link with `href` does: `anchor` for an
 *  in-page `#fragment` (left to the browser), `open` with the absolute
 *  http(s) URL to navigate this tab to, or `block` for anything else. */
export function readerLinkAction(
  href: string | null,
  pageUrl: string,
): { kind: 'anchor' } | { kind: 'open'; url: string } | { kind: 'block' } {
  const raw = (href ?? '').trim();
  if (raw.startsWith('#')) return { kind: 'anchor' };
  if (!raw) return { kind: 'block' };
  try {
    const url = new URL(raw, pageUrl);
    return url.protocol === 'http:' || url.protocol === 'https:' ? { kind: 'open', url: url.href } : { kind: 'block' };
  } catch {
    return { kind: 'block' };
  }
}
