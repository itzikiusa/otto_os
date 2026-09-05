// Allowlist HTML sanitizer for rendered markdown (`{@html}` sinks). No npm
// dependency — DOMParser-based, two passes to a fixpoint:
//   1. structure: disallowed elements are REMOVED (dangerous containers) or
//      UNWRAPPED (unknown formatting → children hoisted) until none remain, so
//      hoisted children are themselves re-examined (`<section><img onerror>`);
//   2. attributes: every remaining element is scrubbed — event handlers, unknown
//      attributes, and URLs outside the scheme allow-list are dropped.
// Defense-in-depth for vault notes, agent transcript prose and tool output
// (WebFetch text is attacker-controlled) rendered to HTML.

const ALLOWED_TAGS = new Set([
  'a', 'abbr', 'b', 'blockquote', 'br', 'code', 'del', 'details', 'div', 'em',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'img', 'input', 'kbd', 'li',
  'mark', 'ol', 'p', 'pre', 's', 'small', 'span', 'strong', 'sub', 'summary',
  'sup', 'table', 'tbody', 'td', 'tfoot', 'th', 'thead', 'tr', 'ul',
]);
/** Dropped wholesale (content included); everything else not allowed is unwrapped. */
const REMOVE_TAGS = new Set([
  'script', 'style', 'iframe', 'frame', 'frameset', 'object', 'embed', 'applet',
  'form', 'link', 'meta', 'base', 'svg', 'math', 'template', 'noscript', 'textarea',
  'select', 'option', 'video', 'audio', 'source', 'track', 'canvas',
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
  div: new Set(['data-embed-path', 'data-path', 'data-diagram']),
  span: new Set(['data-tag', 'data-path']),
};
/** data-* carriers the renderer itself emits on anchors. */
const A_DATA = new Set(['data-path', 'data-raw', 'data-anchor', 'data-unresolved']);

/** Scheme allow-list. Control chars / whitespace are stripped BEFORE the scheme
 *  check so `java\tscript:` (and entity-decoded variants) can't slip through;
 *  a URL with no scheme (relative, `#anchor`, `?q`) is fine. */
function urlOk(v: string, tag: string): boolean {
  const t = v.replace(/[\u0000-\u0020\u007f-\u00a0\s]/g, '').toLowerCase();
  if (t === '') return true;
  const m = /^([a-z][a-z0-9+.-]*):/.exec(t);
  if (!m) return true; // relative / fragment / query — no scheme
  const scheme = m[1];
  if (scheme === 'http' || scheme === 'https' || scheme === 'mailto') return true;
  if (scheme === 'data') {
    return tag === 'img' && t.startsWith('data:image/') && !t.startsWith('data:image/svg');
  }
  return false;
}

/** Sanitize an HTML string for safe `{@html}` injection. */
export function sanitizeHtml(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const body = doc.body;

  // Pass 1 — structure, to a fixpoint. Re-query after every change so hoisted
  // children (and anything nested in them) are examined too.
  for (let el = firstDisallowed(body); el; el = firstDisallowed(body)) {
    if (REMOVE_TAGS.has(el.tagName.toLowerCase())) {
      el.remove();
    } else {
      const parent = el.parentNode!;
      while (el.firstChild) parent.insertBefore(el.firstChild, el);
      el.remove();
    }
  }

  // Pass 2 — attributes on every remaining element.
  for (const node of Array.from(body.querySelectorAll('*'))) {
    const tag = node.tagName.toLowerCase();
    for (const attr of Array.from(node.attributes)) {
      const name = attr.name.toLowerCase();
      const ok =
        !name.startsWith('on') &&
        (GLOBAL_ATTRS.has(name) || TAG_ATTRS[tag]?.has(name) || (tag === 'a' && A_DATA.has(name)));
      if (!ok) {
        node.removeAttribute(attr.name);
        continue;
      }
      if ((name === 'href' || name === 'src') && !urlOk(attr.value, tag)) {
        node.removeAttribute(attr.name);
      }
    }
    // External links never keep an opener; internal ones never get a target.
    if (tag === 'a') {
      if (/^\s*https?:/i.test(node.getAttribute('href') || '')) {
        node.setAttribute('target', '_blank');
        node.setAttribute('rel', 'noopener noreferrer');
      } else {
        node.removeAttribute('target');
      }
    }
    if (tag === 'input') {
      // Only GFM task checkboxes, always inert.
      if (node.getAttribute('type') !== 'checkbox') node.remove();
      else node.setAttribute('disabled', '');
    }
  }
  return body.innerHTML;
}

function firstDisallowed(root: Element): Element | null {
  for (const node of root.querySelectorAll('*')) {
    if (!ALLOWED_TAGS.has(node.tagName.toLowerCase())) return node;
  }
  return null;
}
