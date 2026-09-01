// Shared "build a stable CSS selector for a clicked element" algorithm, used
// by both the reader-mode click-to-annotate overlay (ReaderView.svelte) and
// the live-tab picker overlay (overlay.js, injected into a native webview via
// `browser_eval` — see BrowserView.svelte). Priority: `#id` > a stable
// `data-*` test attribute on the element itself > an nth-of-type tag-path
// from `root` (exclusive) down to `el` — the fallback every element gets,
// and the ONLY option for reader mode's own sanitized render, which rarely
// carries an id or test attribute from the original page.
//
// `overlay.js` duplicates this in plain JS by hand rather than importing it —
// it's shipped as raw source text and `eval`'d straight into a live tab's own
// JS context (a live-tab webview is denied Tauri IPC, so it can't reach back
// into this app's bundle to `import` it either). Keep the two in sync if this
// algorithm changes.

const DATA_ATTR_CANDIDATES = ['data-testid', 'data-test', 'data-id', 'data-qa'];

/** Escape a value for embedding inside a `[attr="…"]` selector string. */
function escapeAttrValue(v: string): string {
  return v.replace(/(["\\])/g, '\\$1');
}

/** True when `id` is a valid bare CSS identifier — some generators emit ids
 *  that aren't (a leading digit, a colon from a framework), which need the
 *  attribute-selector form instead of `#id`. */
function isSimpleId(id: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_-]*$/.test(id);
}

/** Build a selector for `el`, scoped to `root`. Prefers `el`'s own `id` or a
 *  `data-*` test attribute (checked on `el` only, not climbed, so the result
 *  stays short and single-element); falls back to a chain of
 *  `tag:nth-of-type(n)` steps from `root` (exclusive) down to `el` for
 *  elements with neither. Stops climbing once it reaches `root` or runs out
 *  of parents. */
export function buildSelector(el: Element, root: Element): string {
  const id = el.getAttribute('id');
  if (id) return isSimpleId(id) ? `#${id}` : `[id="${escapeAttrValue(id)}"]`;
  for (const attr of DATA_ATTR_CANDIDATES) {
    const v = el.getAttribute(attr);
    if (v) return `[${attr}="${escapeAttrValue(v)}"]`;
  }

  const steps: string[] = [];
  let cur: Element | null = el;
  while (cur && cur !== root) {
    const tag = cur.tagName.toLowerCase();
    const parent: Element | null = cur.parentElement;
    if (!parent) {
      steps.unshift(tag);
      break;
    }
    const currentTag = cur.tagName;
    const siblings = Array.from(parent.children).filter((c) => c.tagName === currentTag);
    const idx = siblings.indexOf(cur) + 1;
    steps.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${idx})` : tag);
    cur = parent === root ? null : parent;
  }
  return steps.join(' > ');
}
