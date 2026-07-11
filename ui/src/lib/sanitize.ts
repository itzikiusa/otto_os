// Allowlist HTML sanitizer for rendered markdown (`{@html}` sinks). No npm
// dependency — DOMParser-based: disallowed elements are dropped (their text
// kept for inline formatting tags we don't know), event handlers and
// javascript:/data: URLs are stripped. Defense-in-depth for vault notes and
// any other untrusted markdown rendered to HTML.

const ALLOWED_TAGS = new Set([
  'a', 'abbr', 'b', 'blockquote', 'br', 'code', 'del', 'details', 'div', 'em',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'img', 'input', 'kbd', 'li',
  'mark', 'ol', 'p', 'pre', 's', 'small', 'span', 'strong', 'sub', 'summary',
  'sup', 'table', 'tbody', 'td', 'tfoot', 'th', 'thead', 'tr', 'ul',
]);

/** Attributes allowed per-tag (plus the global set). */
const GLOBAL_ATTRS = new Set(['class', 'id', 'title', 'dir', 'lang']);
const TAG_ATTRS: Record<string, Set<string>> = {
  a: new Set(['href', 'target', 'rel']),
  img: new Set(['src', 'alt', 'width', 'height', 'loading']),
  input: new Set(['type', 'checked', 'disabled']),
  ol: new Set(['start']),
  td: new Set(['colspan', 'rowspan', 'align']),
  th: new Set(['colspan', 'rowspan', 'align']),
  div: new Set(['data-embed-path', 'data-path']),
  span: new Set(['data-tag', 'data-path']),
};
/** data-* carriers the renderer itself emits on anchors. */
const A_DATA = new Set(['data-path', 'data-raw', 'data-anchor', 'data-unresolved']);

function urlOk(v: string, tag: string): boolean {
  const t = v.trim().toLowerCase();
  if (t.startsWith('javascript:') || t.startsWith('vbscript:')) return false;
  if (t.startsWith('data:')) return tag === 'img' && t.startsWith('data:image/');
  return true;
}

/** Sanitize an HTML string for safe `{@html}` injection. */
export function sanitizeHtml(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const walk = (node: Element) => {
    for (const child of Array.from(node.children)) {
      const tag = child.tagName.toLowerCase();
      if (!ALLOWED_TAGS.has(tag)) {
        // Drop dangerous containers wholesale; unwrap unknown formatting.
        if (['script', 'style', 'iframe', 'object', 'embed', 'form', 'link', 'meta', 'base', 'svg', 'math'].includes(tag)) {
          child.remove();
        } else {
          const parent = child.parentNode!;
          while (child.firstChild) parent.insertBefore(child.firstChild, child);
          child.remove();
        }
        continue;
      }
      for (const attr of Array.from(child.attributes)) {
        const name = attr.name.toLowerCase();
        const ok =
          GLOBAL_ATTRS.has(name) ||
          TAG_ATTRS[tag]?.has(name) ||
          (tag === 'a' && A_DATA.has(name));
        if (!ok || name.startsWith('on')) {
          child.removeAttribute(attr.name);
          continue;
        }
        if ((name === 'href' || name === 'src') && !urlOk(attr.value, tag)) {
          child.removeAttribute(attr.name);
        }
      }
      // External links never keep an opener.
      if (tag === 'a' && (child.getAttribute('href') || '').match(/^https?:/i)) {
        child.setAttribute('target', '_blank');
        child.setAttribute('rel', 'noopener noreferrer');
      }
      if (tag === 'input') {
        // Only GFM task checkboxes, always inert.
        if (child.getAttribute('type') !== 'checkbox') child.remove();
        else child.setAttribute('disabled', '');
      }
      walk(child);
    }
  };
  walk(doc.body);
  return doc.body.innerHTML;
}
