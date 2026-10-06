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

// ── D2 `<style>` scoping (S11-12 → S18-301 / S11-311) ─────────────────────
// `{@html}` puts the SVG into the main document, so a stray rule would restyle
// the whole app (UI redress, `url()` beacons, attribute-selector exfiltration).
// The CSS is parsed by the BROWSER's own parser (`CSSStyleSheet.replaceSync`)
// and re-emitted from the CSSOM — never split by hand: a hand-rolled splitter
// and the browser disagree on strings / escapes / comments (`content:"}"`
// ended a rule early and let the next one through unscoped). What survives:
//   - style rules, every selector anchored under `.d2-N` (a selector that
//     already starts with the scope may not hop out with `~` / `+`);
//   - `@media` (recursively);
//   - `@font-face` only for the diagram's own `d2-N-…` families with `data:`
//     sources, `@keyframes` only for D2's own names — both are global names.
// Declarations that would fetch (`url(http…)`, `image-set()`, …) are dropped.
// The result is re-parsed and verified; anything unexpected → '' (fail closed).

/** One declaration, browser-serialized: `[property, value, important]`. */
export type CssDecl = [string, string, boolean];

/** The slice of the CSSOM the scoper reads, normalized so `node --test` can
 *  drive the emitter without a DOM. */
export type CssNode =
  | { type: 'style'; selector: string; decls: CssDecl[] }
  | { type: 'media'; condition: string; rules: CssNode[] }
  | { type: 'font-face'; decls: CssDecl[] }
  | { type: 'keyframes'; name: string; frames: Array<{ key: string; decls: CssDecl[] }> }
  | { type: 'other' };

/** Parses CSS into `CssNode`s, or `null` when no parser is available. */
export type CssParser = (css: string) => CssNode[] | null;

type CssCtor = abstract new (...args: never[]) => unknown;

function cssCtor(name: string): CssCtor | null {
  const c = (globalThis as unknown as Record<string, unknown>)[name];
  return typeof c === 'function' ? (c as CssCtor) : null;
}

function declsOf(style: CSSStyleDeclaration): CssDecl[] {
  const out: CssDecl[] = [];
  for (let i = 0; i < style.length; i++) {
    const name = style.item(i);
    out.push([name, style.getPropertyValue(name), style.getPropertyPriority(name) === 'important']);
  }
  return out;
}

function nodesOf(list: CSSRuleList): CssNode[] {
  const Style = cssCtor('CSSStyleRule');
  const Media = cssCtor('CSSMediaRule');
  const Font = cssCtor('CSSFontFaceRule');
  const Frames = cssCtor('CSSKeyframesRule');
  const out: CssNode[] = [];
  for (const r of Array.from(list)) {
    // Nested rules inside a style rule (CSS nesting) are not read → dropped.
    if (Style && r instanceof Style) {
      const s = r as CSSStyleRule;
      out.push({ type: 'style', selector: s.selectorText, decls: declsOf(s.style) });
    } else if (Media && r instanceof Media) {
      const m = r as CSSMediaRule;
      out.push({ type: 'media', condition: m.media.mediaText, rules: nodesOf(m.cssRules) });
    } else if (Font && r instanceof Font) {
      out.push({ type: 'font-face', decls: declsOf((r as CSSFontFaceRule).style) });
    } else if (Frames && r instanceof Frames) {
      const k = r as CSSKeyframesRule;
      const frames = Array.from(k.cssRules).map((f) => ({
        key: (f as CSSKeyframeRule).keyText,
        decls: declsOf((f as CSSKeyframeRule).style),
      }));
      out.push({ type: 'keyframes', name: k.name, frames });
    } else {
      out.push({ type: 'other' });
    }
  }
  return out;
}

/** The browser's own CSS parser; `null` (→ styles dropped) without one. */
export const parseCssBrowser: CssParser = (css) => {
  const Sheet = (globalThis as unknown as { CSSStyleSheet?: new () => CSSStyleSheet }).CSSStyleSheet;
  if (typeof Sheet !== 'function') return null;
  try {
    const sheet = new Sheet();
    if (typeof sheet.replaceSync !== 'function') return null;
    sheet.replaceSync(css);
    return nodesOf(sheet.cssRules);
  } catch {
    return null;
  }
};

/** Walk `text` at top level (outside strings / brackets / parens), calling
 *  `visit(char, index)` for each top-level char. Returns false when the text
 *  is not well-formed (unterminated string, unbalanced brackets). */
function walkTopLevel(text: string, visit: (ch: string, i: number) => void): boolean {
  let depth = 0;
  let quote = '';
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quote) {
      if (ch === '\\') i++;
      else if (ch === quote) quote = '';
      else if (ch === '\n') return false;
      continue;
    }
    if (ch === '\\') {
      i++;
      continue;
    }
    if (ch === '"' || ch === "'") quote = ch;
    else if (ch === '(' || ch === '[') depth++;
    else if (ch === ')' || ch === ']') {
      if (--depth < 0) return false;
    } else if (depth === 0) visit(ch, i);
  }
  return quote === '' && depth === 0;
}

/** A serialized selector list / value must not be able to leave its slot. */
function contained(text: string): boolean {
  let ok = true;
  const closed = walkTopLevel(text, (ch) => {
    if (ch === '{' || ch === '}' || ch === ';') ok = false;
  });
  return ok && closed && !/\/\*|<!--|-->/.test(text.replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, ''));
}

/** Split a selector list on its top-level commas (`:is(a, b)` stays whole). */
export function splitSelectorList(list: string): string[] {
  const cuts: number[] = [];
  if (!walkTopLevel(list, (ch, i) => ch === ',' && cuts.push(i))) return [];
  const out: string[] = [];
  let from = 0;
  for (const c of [...cuts, list.length]) {
    const sel = list.slice(from, c).trim();
    if (sel) out.push(sel);
    from = c + 1;
  }
  return out;
}

const COMPOUND_CONT = /[.:[#]/;

/** True when `sel` is confined to the diagram: it starts with the scope
 *  compound and the first combinator after that compound goes DOWN the tree
 *  (descendant / `>`), never sideways (`~` / `+`) into the app. */
export function isScopedSelector(sel: string, scope: string): boolean {
  const prefix = `.${scope}`;
  if (!sel.startsWith(prefix)) return false;
  const rest = sel.slice(prefix.length);
  if (rest !== '' && !/^\s/.test(rest) && !COMPOUND_CONT.test(rest[0]) && rest[0] !== '>') return false;
  // First top-level combinator (or whitespace run) after the scope compound.
  let combinator = '';
  let seenSpace = false;
  const ok = walkTopLevel(rest, (ch) => {
    if (combinator) return;
    if (ch === '~' || ch === '+' || ch === '>') combinator = ch;
    else if (/\s/.test(ch)) seenSpace = true;
    else if (seenSpace) combinator = ' ';
  });
  return ok && combinator !== '~' && combinator !== '+';
}

/** Anchor one selector under the scope; `null` when it can't be confined. */
export function scopeSelector(sel: string, scope: string): string | null {
  const prefix = `.${scope}`;
  const anchored = sel.startsWith(prefix) && (sel.length === prefix.length || /[\s.:[#>~+]/.test(sel[prefix.length]));
  const out = anchored ? sel : `${prefix} ${sel}`;
  return isScopedSelector(out, scope) ? out : null;
}

/** Functions in a value that make the browser fetch something. */
const FETCHING = /\b(url|src|image-set|-webkit-image-set|image|cross-fade|-webkit-cross-fade|element|-moz-element)\s*\(/gi;

/** A declaration value may stay when it is contained and fetches nothing
 *  beyond an in-document `#ref` or an inline `data:` URI. */
export function safeCssValue(value: string): boolean {
  if (!contained(value)) return false;
  for (const m of value.matchAll(FETCHING)) {
    if (m[1].toLowerCase() !== 'url') return false;
    const arg = value
      .slice((m.index ?? 0) + m[0].length)
      .trimStart()
      .replace(/^["']/, '')
      .toLowerCase();
    if (!arg.startsWith('#') && !arg.startsWith('data:')) return false;
  }
  return true;
}

const PROP_NAME = /^(--[\w-]+|-?[a-z][a-z0-9-]*)$/i;

function emitDecls(decls: CssDecl[]): string {
  return decls
    .filter(([name, value]) => PROP_NAME.test(name) && safeCssValue(value))
    .map(([name, value, important]) => `${name}:${value}${important ? ' !important' : ''}`)
    .join(';');
}

function unquote(v: string): string {
  return v.trim().replace(/^(["'])(.*)\1$/, '$2');
}

/** D2's own global names: `@font-face { font-family: d2-N-font-… }`,
 *  `@keyframes d2Transition-d2-N-<i>` and the fixed `dashdraw`. */
function ownFontFace(decls: CssDecl[], scope: string): boolean {
  const family = decls.find(([n]) => n === 'font-family');
  return !!family && unquote(family[1]).startsWith(`${scope}-`);
}

function ownKeyframes(name: string, scope: string): boolean {
  return /^[\w-]+$/.test(name) && (name === 'dashdraw' || name.startsWith(`d2Transition-${scope}-`));
}

/** Re-emit parsed rules, scoped to `scope` (exported for tests). */
export function emitScopedCss(nodes: CssNode[], scope: string): string {
  let out = '';
  for (const n of nodes) {
    if (n.type === 'style') {
      if (!contained(n.selector)) continue;
      const sels = splitSelectorList(n.selector).map((s) => scopeSelector(s, scope));
      if (!sels.length || sels.some((s) => s === null)) continue;
      out += `${sels.join(',')}{${emitDecls(n.decls)}}`;
    } else if (n.type === 'media') {
      const inner = emitScopedCss(n.rules, scope);
      if (inner && contained(n.condition)) out += `@media ${n.condition}{${inner}}`;
    } else if (n.type === 'font-face') {
      // A face whose `src` would fetch is dropped whole (not left src-less).
      if (ownFontFace(n.decls, scope) && n.decls.every(([, v]) => safeCssValue(v)))
        out += `@font-face{${emitDecls(n.decls)}}`;
    } else if (n.type === 'keyframes') {
      if (!ownKeyframes(n.name, scope)) continue;
      const frames = n.frames.filter((f) => contained(f.key)).map((f) => `${f.key}{${emitDecls(f.decls)}}`);
      out += `@keyframes ${n.name}{${frames.join('')}}`;
    }
    // Anything else (@import, @supports, @layer, @property, …) is dropped.
  }
  return out;
}

/** True when every parsed rule is one `emitScopedCss` could have produced. */
function verifyScoped(nodes: CssNode[], scope: string): boolean {
  return nodes.every((n) => {
    if (n.type === 'style') {
      const sels = splitSelectorList(n.selector);
      return sels.length > 0 && sels.every((s) => isScopedSelector(s, scope)) && n.decls.every(([, v]) => safeCssValue(v));
    }
    if (n.type === 'media') return verifyScoped(n.rules, scope);
    if (n.type === 'font-face') return ownFontFace(n.decls, scope) && n.decls.every(([, v]) => safeCssValue(v));
    if (n.type === 'keyframes') return ownKeyframes(n.name, scope);
    return false;
  });
}

/** Rewrite a D2 `<style>` so every rule only matches inside this diagram.
 *  `parse` defaults to the browser's parser; without one the styles go. */
export function scopeD2Css(css: string, scope: string, parse: CssParser = parseCssBrowser): string {
  if (!/^d2-\d+$/.test(scope)) return '';
  const nodes = parse(css);
  if (!nodes) return '';
  const out = emitScopedCss(nodes, scope);
  // Re-parse what we emit: it must read back as exactly what we meant.
  const check = parse(out);
  return check && verifyScoped(check, scope) ? out : '';
}

/** The diagram's own scope class (`d2-<digits>`) — read from D2's own
 *  wrapper (`<svg data-d2-version …><svg class="d2-N d2-svg" …>`: the first
 *  two `<svg>` start tags), never from a (label-influenced) inner element. */
export function d2ScopeClass(svg: string): string | null {
  const tags = svg.matchAll(/<svg\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi);
  for (let n = 0; n < 2; n++) {
    const tag = tags.next();
    if (tag.done) return null;
    const cls = /\sclass\s*=\s*(?:"([^"]*)"|'([^']*)')/i.exec(tag.value[0]);
    if (!cls) continue;
    const m = /(?:^|\s)(d2-\d+)(?=\s|$)/.exec(cls[1] ?? cls[2] ?? '');
    return m ? m[1] : null;
  }
  return null;
}

/** Text of a serialized SVG `<style>` is entity-escaped (`&gt;`, `&amp;`, …);
 *  decode before parsing, re-escape after so no markup can come back out. */
function decodeStyleText(text: string): string {
  return text.replace(/<!\[CDATA\[|\]\]>/g, '').replace(/&(#x[0-9a-f]+|#\d+|amp|lt|gt|quot|apos|nbsp);/gi, (_m, e: string) => {
    const k = e.toLowerCase();
    if (k.startsWith('#x')) return String.fromCodePoint(parseInt(k.slice(2), 16) || 0xfffd);
    if (k.startsWith('#')) return String.fromCodePoint(parseInt(k.slice(1), 10) || 0xfffd);
    return { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' }[k] ?? '';
  });
}

function encodeStyleText(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/** Scope every `<style>` in a (sanitized) D2 SVG to its diagram; without a
 *  scope class to anchor on, the styles are dropped (fail closed). */
export function scopeD2Styles(svg: string, parse: CssParser = parseCssBrowser): string {
  const scope = d2ScopeClass(svg);
  return svg.replace(/(<style\b[^>]*>)([\s\S]*?)(<\/style>)/gi, (_m, open: string, css: string, close: string) =>
    scope ? `${open}${encodeStyleText(scopeD2Css(decodeStyleText(css), scope, parse))}${close}` : `${open}${close}`,
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
