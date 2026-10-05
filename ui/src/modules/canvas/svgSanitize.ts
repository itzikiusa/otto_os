// Sanitizer for diagram SVG that is about to hit an `innerHTML` / `{@html}`
// sink. D2 renders whatever the diagram source says — a label can carry a
// `link: javascript:…`, and markdown labels become `<foreignObject>` HTML — and
// that source is often agent- or attacker-authored (vault notes, transcripts,
// fetched pages). So every D2 render is purified here before any caller sees it.
//
// DOMPurify does the parsing/allow-listing (SVG + filters + HTML for
// foreignObject); the hook below narrows links to http(s) / in-document
// fragments and drops every event-handler attribute even if a future DOMPurify
// profile allowed one. Pure module (no runtime imports) so `node --test` can
// exercise the hook without a DOM; the caller passes the DOMPurify instance.

import type { Config, DOMPurify, UponSanitizeAttributeHookEvent } from 'dompurify';

/** DOMPurify config: SVG (with filters) + the HTML profile, which D2 needs for
 *  `<foreignObject>` markdown labels; `<style>` stays (D2 themes via classes). */
export const D2_PURIFY_CONFIG: Config = {
  USE_PROFILES: { svg: true, svgFilters: true, html: true },
  ADD_TAGS: ['foreignObject'],
  FORBID_TAGS: ['script', 'iframe', 'object', 'embed', 'form', 'base', 'meta', 'link'],
  // Never let DOMPurify keep a `data:` URI on an arbitrary element.
  ALLOW_DATA_ATTR: false,
};

const URL_ATTRS = new Set(['href', 'xlink:href', 'src', 'action', 'formaction']);

/** True when `value` may stay on a link-ish attribute: an http(s) URL or an
 *  in-document `#fragment` (D2 cross-references its own markers/clip paths).
 *  Control chars / whitespace are stripped first so `java\tscript:` can't
 *  slip past the scheme check. */
export function svgUrlAllowed(value: string): boolean {
  // eslint-disable-next-line no-control-regex
  const t = value.replace(/[\u0000- \u007f- \s]/g, '').toLowerCase();
  if (t === '') return true;
  if (t.startsWith('#')) return true;
  return t.startsWith('https://') || t.startsWith('http://');
}

/** `uponSanitizeAttribute` hook: drop event handlers + non-http(s) URLs. */
export function d2AttributeHook(_node: Element, ev: UponSanitizeAttributeHookEvent): void {
  const name = ev.attrName.toLowerCase();
  if (name.startsWith('on')) {
    ev.keepAttr = false;
    return;
  }
  if (URL_ATTRS.has(name) && !svgUrlAllowed(ev.attrValue)) ev.keepAttr = false;
}

/** Sanitize a D2 SVG string. Returns `null` — fail CLOSED — when the purifier
 *  can't run (no DOM) or nothing SVG survives. */
export function sanitizeD2Svg(svg: string, purify: DOMPurify): string | null {
  if (!purify.isSupported) return null;
  purify.addHook('uponSanitizeAttribute', d2AttributeHook);
  try {
    const out = purify.sanitize(svg, { ...D2_PURIFY_CONFIG, RETURN_TRUSTED_TYPE: false });
    const s = String(out);
    return s.includes('<svg') ? s : null;
  } finally {
    purify.removeHook('uponSanitizeAttribute', d2AttributeHook);
  }
}
