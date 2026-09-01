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
//
// TRUST: for a live tab, `id`/`data-*` VALUES come straight off a page Otto
// doesn't control — unlike reader mode's own sanitized render, which almost
// never carries one from the original page (so it falls through to the
// always-structural nth-of-type path in practice). A hostile page can set
// `data-testid` to an arbitrarily long, arbitrary string (including literal
// newlines — `data-*` values aren't whitespace-restricted the way `id` is).
// The server enforces the real limit at annotation-creation time
// (`SELECTOR_MAX_CHARS` in `routes/browser.rs`, which also FENCES the
// selector as untrusted content before ever splicing it into agent-facing
// text) — this cap is defense-in-depth, not the trust boundary: a candidate
// that would exceed it is skipped in favor of the next priority tier rather
// than truncated (truncating an attribute-selector string risks leaving it
// syntactically broken), and the nth-of-type fallback (never attacker text,
// only tag names/counts) is capped as an absolute last resort.

const DATA_ATTR_CANDIDATES = ['data-testid', 'data-test', 'data-id', 'data-qa'];

/** Selectors longer than this are rejected — see the TRUST note above. */
export const MAX_SELECTOR_LEN = 300;

/** Escape a value for embedding inside a `[attr="…"]` selector STRING (not an
 *  identifier — `CSS.escape` is for identifier position, e.g. `#id`/`.class`,
 *  and over-escapes a quoted-string value, so it's deliberately NOT used
 *  here). Backslash-escaping the two characters that matter inside a
 *  double-quoted CSS string (`"`, `\`) is both correct and all that's needed. */
function escapeAttrValue(v: string): string {
  return v.replace(/(["\\])/g, '\\$1');
}

/** Build a `#id`-shorthand selector for `id`. Prefers the platform's own
 *  `CSS.escape` (identifier-position escaping — correctly handles a leading
 *  digit, a colon from a framework, unicode, etc., so it always produces a
 *  valid bare selector); falls back to using `id` as-is when it's already a
 *  simple identifier, else the `[id="…"]` attribute-selector form, for the
 *  injected-overlay context where `CSS.escape` may not exist on an
 *  older/embedded WebKit. */
function idSelector(id: string): string {
  if (typeof CSS !== 'undefined' && typeof CSS.escape === 'function') {
    return `#${CSS.escape(id)}`;
  }
  return /^[A-Za-z_][A-Za-z0-9_-]*$/.test(id) ? `#${id}` : `[id="${escapeAttrValue(id)}"]`;
}

/** Build a selector for `el`, scoped to `root`. Prefers `el`'s own `id` or a
 *  `data-*` test attribute (checked on `el` only, not climbed, so the result
 *  stays short and single-element); falls back to a chain of
 *  `tag:nth-of-type(n)` steps from `root` (exclusive) down to `el` for
 *  elements with neither. Stops climbing once it reaches `root` or runs out
 *  of parents. A candidate longer than `MAX_SELECTOR_LEN` is skipped in favor
 *  of the next priority tier (see the TRUST note above); the final
 *  nth-of-type fallback is hard-truncated at the cap as a last resort. */
export function buildSelector(el: Element, root: Element): string {
  const id = el.getAttribute('id');
  if (id) {
    const s = idSelector(id);
    if (s.length <= MAX_SELECTOR_LEN) return s;
  }
  for (const attr of DATA_ATTR_CANDIDATES) {
    const v = el.getAttribute(attr);
    if (v) {
      const s = `[${attr}="${escapeAttrValue(v)}"]`;
      if (s.length <= MAX_SELECTOR_LEN) return s;
    }
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
  const path = steps.join(' > ');
  return path.length > MAX_SELECTOR_LEN ? path.slice(0, MAX_SELECTOR_LEN) : path;
}
