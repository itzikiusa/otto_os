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
 *  `<foreignObject>` markdown labels; `<style>` stays (D2 themes via classes)
 *  but is re-scoped to the diagram afterwards (`scopeD2Styles`). */
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

/** Split CSS into its top-level blocks (`prelude { body }`), comments
 *  stripped. Unbalanced input yields what parsed so far. */
function cssBlocks(css: string): Array<{ prelude: string; body: string }> {
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Array<{ prelude: string; body: string }> = [];
  let i = 0;
  while (i < src.length) {
    const open = src.indexOf('{', i);
    if (open < 0) break;
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth++;
      else if (src[j] === '}') depth--;
      j++;
    }
    if (depth !== 0) break;
    // A bare `@import …;` / `@charset …;` before the brace is simply dropped.
    const prelude = src.slice(i, open).replace(/@(import|charset|namespace)[^;]*;/gi, '').trim();
    out.push({ prelude, body: src.slice(open + 1, j - 1) });
    i = j;
  }
  return out;
}

/** Rewrite a D2 `<style>` so every rule only matches inside this diagram
 *  (S11-12). `{@html}` puts the SVG into the main document, so a stray rule
 *  would otherwise restyle the whole app (UI redress, attribute-selector
 *  exfiltration). Selectors not already under `.scope` get it prefixed;
 *  `@font-face` / `@keyframes` (D2's own fonts and animations) are kept;
 *  `@media` is scoped recursively; every other at-rule (`@import`, …) goes. */
export function scopeD2Css(css: string, scope: string): string {
  const prefix = `.${scope}`;
  return cssBlocks(css)
    .map(({ prelude, body }) => {
      if (prelude.startsWith('@')) {
        const at = prelude.slice(1).split(/[\s({]/)[0].toLowerCase();
        if (at === 'font-face' || at.endsWith('keyframes')) return `${prelude}{${body}}`;
        if (at === 'media') return `${prelude}{${scopeD2Css(body, scope)}}`;
        return '';
      }
      if (prelude === '') return '';
      const selectors = prelude
        .split(',')
        .map((sel) => sel.trim())
        .filter((sel) => sel !== '')
        .map((sel) => (sel === prefix || sel.startsWith(`${prefix} `) || sel.startsWith(`${prefix}.`) || sel.startsWith(`${prefix}:`) ? sel : `${prefix} ${sel}`));
      return selectors.length ? `${selectors.join(',')}{${body}}` : '';
    })
    .join('');
}

/** The diagram's own scope class (`d2-<digits>`, on its root `<svg>`). */
export function d2ScopeClass(svg: string): string | null {
  const m = /class="[^"]*\b(d2-\d+)\b/.exec(svg);
  return m ? m[1] : null;
}

/** Scope every `<style>` in a (sanitized) D2 SVG to its diagram; without a
 *  scope class to anchor on, the styles are dropped (fail closed). */
export function scopeD2Styles(svg: string): string {
  const scope = d2ScopeClass(svg);
  return svg.replace(/(<style\b[^>]*>)([\s\S]*?)(<\/style>)/gi, (_m, open: string, css: string, close: string) =>
    scope ? `${open}${scopeD2Css(css, scope)}${close}` : `${open}${close}`,
  );
}

/** Sanitize a D2 SVG string. Returns `null` — fail CLOSED — when the purifier
 *  can't run (no DOM) or nothing SVG survives. */
export function sanitizeD2Svg(svg: string, purify: DOMPurify): string | null {
  if (!purify.isSupported) return null;
  purify.addHook('uponSanitizeAttribute', d2AttributeHook);
  try {
    const out = purify.sanitize(svg, { ...D2_PURIFY_CONFIG, RETURN_TRUSTED_TYPE: false });
    const s = String(out);
    return s.includes('<svg') ? scopeD2Styles(s) : null;
  } finally {
    purify.removeHook('uponSanitizeAttribute', d2AttributeHook);
  }
}
