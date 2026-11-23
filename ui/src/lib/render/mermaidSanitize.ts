// Sanitizer for Mermaid SVG that is about to hit an `{@html}` sink.
// Even with `securityLevel: 'strict'`, Mermaid's default `secure` list leaves
// `themeCSS` open and `classDef`/style directives can still carry `url(https:…)`
// or stray rules into `<style>`, which lands in the main app document.
//
// Mirroring `modules/canvas/svgSanitize.ts` (for D2), every Mermaid render goes
// through DOMPurify (SVG + filters + HTML for `<foreignObject>` labels) plus
// browser-CSSOM `<style>` scoping anchored to the diagram's root `#<id>`.
// Pure module (no runtime DOM import) so `node --test` drives it directly.

import type { Config, DOMPurify, UponSanitizeAttributeHookEvent } from 'dompurify';
import { escapeHtml } from '../html.ts';

/** Mermaid directives locked against untrusted diagram `%%{init: …}%%` blocks. */
export const MERMAID_SECURE_KEYS = [
  'secure',
  'securityLevel',
  'startOnLoad',
  'maxTextSize',
  'suppressErrorRendering',
  'maxEdges',
  'themeCSS',
  'fontFamily',
  'altFontFamily',
  'themeVariables',
] as const;

export const MERMAID_PURIFY_CONFIG: Config = {
  USE_PROFILES: { svg: true, svgFilters: true, html: true },
  ADD_TAGS: ['foreignObject'],
  FORBID_TAGS: [
    'script',
    'iframe',
    'object',
    'embed',
    'form',
    'base',
    'meta',
    'link',
    'img',
    'image',
    'feimage',
    'feImage',
  ],
  ALLOW_DATA_ATTR: false,
};

const URL_ATTRS = new Set(['href', 'xlink:href', 'src', 'action', 'formaction']);
const LINK_ATTRS = new Set(['href', 'xlink:href']);

function stripUrlControlChars(value: string): string {
  // eslint-disable-next-line no-control-regex
  return value.replace(/[\u0000-\u0020\u007f-\u00a0\s]/g, '').toLowerCase();
}

/** True when `value` is empty or an in-document `#fragment` reference. */
export function mermaidFragmentAllowed(value: string): boolean {
  const t = stripUrlControlChars(value);
  return t === '' || t.startsWith('#');
}

/** True when `value` may stay on an `<a>` link attribute: an http(s) URL or an
 *  in-document `#fragment` (Mermaid markers/clip paths). Control chars and
 *  whitespace are stripped first so `java\tscript:` cannot slip past. */
export function mermaidUrlAllowed(value: string): boolean {
  const t = stripUrlControlChars(value);
  if (t === '' || t.startsWith('#')) return true;
  return t.startsWith('https://') || t.startsWith('http://');
}

/** `uponSanitizeAttribute` hook: drop event handlers, restrict external URLs to
 *  `<a href>`, and sanitize inline `style` attributes against fetch beacons and
 *  layout-escape overlays. */
export function mermaidAttributeHook(node: Element, ev: UponSanitizeAttributeHookEvent): void {
  const name = ev.attrName.toLowerCase();
  if (name.startsWith('on')) {
    ev.keepAttr = false;
    return;
  }
  if (name === 'style') {
    const cleaned = sanitizeMermaidInlineStyle(ev.attrValue);
    if (!cleaned) {
      ev.keepAttr = false;
    } else {
      ev.attrValue = cleaned;
    }
    return;
  }
  if (URL_ATTRS.has(name)) {
    const tag = (node?.tagName ?? '').toLowerCase();
    const ok =
      tag === 'a' && LINK_ATTRS.has(name)
        ? mermaidUrlAllowed(ev.attrValue)
        : mermaidFragmentAllowed(ev.attrValue);
    if (!ok) ev.keepAttr = false;
  }
}

// ── Mermaid `<style>` scoping ───────────────────────────────────────────────

export type CssDecl = [string, string, boolean];

export type CssNode =
  | { type: 'style'; selector: string; decls: CssDecl[] }
  | { type: 'media'; condition: string; rules: CssNode[] }
  | { type: 'keyframes'; name: string; frames: Array<{ key: string; decls: CssDecl[] }> }
  | { type: 'other' };

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
  const Frames = cssCtor('CSSKeyframesRule');
  const out: CssNode[] = [];
  for (const r of Array.from(list)) {
    if (Style && r instanceof Style) {
      const s = r as CSSStyleRule;
      out.push({ type: 'style', selector: s.selectorText, decls: declsOf(s.style) });
    } else if (Media && r instanceof Media) {
      const m = r as CSSMediaRule;
      out.push({ type: 'media', condition: m.media.mediaText, rules: nodesOf(m.cssRules) });
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

function contained(text: string): boolean {
  let ok = true;
  const closed = walkTopLevel(text, (ch) => {
    if (ch === '{' || ch === '}' || ch === ';') ok = false;
  });
  return ok && closed && !/\/\*|<!--|--!?>/.test(text.replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, ''));
}

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
const SAFE_ID = /^[A-Za-z_][A-Za-z0-9_-]*$/;

/** True when `sel` starts with `#<scopeId>` and the first combinator after
 *  that compound goes DOWN the tree (descendant / `>`), never sideways (`~`/`+`). */
export function isScopedMermaidSelector(sel: string, scopeId: string): boolean {
  const prefix = `#${scopeId}`;
  if (!sel.startsWith(prefix)) return false;
  const rest = sel.slice(prefix.length);
  if (rest !== '' && !/^\s/.test(rest) && !COMPOUND_CONT.test(rest[0]) && rest[0] !== '>') return false;
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

export function scopeMermaidSelector(sel: string, scopeId: string): string | null {
  const prefix = `#${scopeId}`;
  const anchored = sel.startsWith(prefix) && (sel.length === prefix.length || /[\s.:[#>~+]/.test(sel[prefix.length]));
  const out = anchored ? sel : `${prefix} ${sel}`;
  return isScopedMermaidSelector(out, scopeId) ? out : null;
}

const FETCHING = /\b(url|src|image-set|-webkit-image-set|image|cross-fade|-webkit-cross-fade|element|-moz-element)\s*\(/gi;
const BLOCKED_CSS_PROPS = new Set(['z-index', 'behavior', '-moz-binding']);
const SAFE_POSITIONS = new Set(['static', 'relative']);

function unescapeCss(text: string): string {
  return text.replace(
    /\\([0-9a-fA-F]{1,6})[ \t\r\n\f]?|\\([^\r\n\f])/g,
    (_m, hex: string | undefined, ch: string | undefined) => {
      if (hex) {
        const cp = parseInt(hex, 16);
        return cp > 0 && cp <= 0x10ffff ? String.fromCodePoint(cp) : '\ufffd';
      }
      return ch ?? '';
    },
  );
}

export function safeMermaidCssValue(value: string): boolean {
  if (!contained(value)) return false;
  const decoded = unescapeCss(value);
  for (const m of decoded.matchAll(FETCHING)) {
    if (m[1].toLowerCase() !== 'url') return false;
    const arg = decoded
      .slice((m.index ?? 0) + m[0].length)
      .trimStart()
      .replace(/^["']/, '')
      .toLowerCase();
    if (!arg.startsWith('#') && !arg.startsWith('data:')) return false;
  }
  return true;
}

const PROP_NAME = /^(--[\w-]+|-?[a-z][a-z0-9-]*)$/i;

/** True when a CSS property + value pair is safe inside a Mermaid SVG (blocks
 *  external fetches as well as layout-escape overlay properties like
 *  `position: fixed`, `position: absolute`, `position: sticky`, and `z-index`). */
export function safeMermaidCssDecl(name: string, value: string): boolean {
  const prop = name.trim().toLowerCase();
  if (!PROP_NAME.test(prop)) return false;
  if (BLOCKED_CSS_PROPS.has(prop)) return false;
  if (!safeMermaidCssValue(value)) return false;
  if (prop === 'position') {
    const pos = unescapeCss(value).trim().toLowerCase();
    if (!SAFE_POSITIONS.has(pos)) return false;
  }
  return true;
}

/** Sanitize an inline `style="..."` attribute value on an SVG or foreignObject
 *  HTML element. Fails closed (`''`) on malformed syntax and drops unsafe
 *  declarations while preserving valid Mermaid node/label styling. */
export function sanitizeMermaidInlineStyle(raw: string): string {
  const src = raw.trim();
  if (!src) return '';
  const cuts: number[] = [];
  if (!walkTopLevel(src, (ch, i) => ch === ';' && cuts.push(i))) return '';
  const kept: string[] = [];
  let from = 0;
  for (const c of [...cuts, src.length]) {
    const part = src.slice(from, c).trim();
    from = c + 1;
    if (!part) continue;
    const colon = part.indexOf(':');
    if (colon <= 0) continue;
    const name = part.slice(0, colon).trim();
    let val = part.slice(colon + 1).trim();
    const important = /\s*!important$/i.test(val);
    if (important) val = val.replace(/\s*!important$/i, '').trim();
    if (!val || !safeMermaidCssDecl(name, val)) continue;
    kept.push(`${name}:${val}${important ? ' !important' : ''}`);
  }
  return kept.join(';');
}

function emitDecls(decls: CssDecl[]): string {
  return decls
    .filter(([name, value]) => safeMermaidCssDecl(name, value))
    .map(([name, value, important]) => `${name}:${value}${important ? ' !important' : ''}`)
    .join(';');
}

function safeKeyframesName(name: string): boolean {
  return /^[\w-]+$/.test(name);
}

export function emitScopedMermaidCss(nodes: CssNode[], scopeId: string): string {
  let out = '';
  for (const n of nodes) {
    if (n.type === 'style') {
      if (!contained(n.selector)) continue;
      const sels = splitSelectorList(n.selector).map((s) => scopeMermaidSelector(s, scopeId));
      if (!sels.length || sels.some((s) => s === null)) continue;
      const decls = emitDecls(n.decls);
      if (decls) out += `${sels.join(',')}{${decls}}`;
    } else if (n.type === 'media') {
      const inner = emitScopedMermaidCss(n.rules, scopeId);
      if (inner && contained(n.condition)) out += `@media ${n.condition}{${inner}}`;
    } else if (n.type === 'keyframes') {
      if (!safeKeyframesName(n.name)) continue;
      const frames = n.frames
        .filter((f) => contained(f.key))
        .map((f) => `${f.key}{${emitDecls(f.decls)}}`);
      out += `@keyframes ${n.name}{${frames.join('')}}`;
    }
    // @import, @font-face, @layer, @property, etc. are dropped.
  }
  return out;
}

function verifyScoped(nodes: CssNode[], scopeId: string): boolean {
  return nodes.every((n) => {
    if (n.type === 'style') {
      const sels = splitSelectorList(n.selector);
      return sels.length > 0 && sels.every((s) => isScopedMermaidSelector(s, scopeId)) && n.decls.every(([k, v]) => safeMermaidCssDecl(k, v));
    }
    if (n.type === 'media') return verifyScoped(n.rules, scopeId);
    if (n.type === 'keyframes') {
      return safeKeyframesName(n.name) && n.frames.every((f) => f.decls.every(([k, v]) => safeMermaidCssDecl(k, v)));
    }
    return false;
  });
}

/** Rewrite a Mermaid `<style>` block so every rule is confined to `#<scopeId>`. */
export function scopeMermaidCss(css: string, scopeId: string, parse: CssParser = parseCssBrowser): string {
  if (!SAFE_ID.test(scopeId)) return '';
  const nodes = parse(css);
  if (!nodes) return '';
  const out = emitScopedMermaidCss(nodes, scopeId);
  const check = parse(out);
  return check && verifyScoped(check, scopeId) ? out : '';
}

/** Extract the root `<svg id="…">` scope id. */
export function mermaidScopeId(svg: string): string | null {
  const m = /<svg\b(?:[^>"']|"[^"]*"|'[^']*')*>/i.exec(svg);
  if (!m) return null;
  const idMatch = /\sid\s*=\s*(?:"([^"]*)"|'([^']*)')/i.exec(m[0]);
  const id = idMatch ? (idMatch[1] ?? idMatch[2] ?? '') : '';
  return SAFE_ID.test(id) ? id : null;
}

function decodeStyleText(text: string): string {
  return text.replace(/<!\[CDATA\[|\]\]>/g, '').replace(/&(#x[0-9a-f]+|#\d+|amp|lt|gt|quot|apos|nbsp);/gi, (_m, e: string) => {
    const k = e.toLowerCase();
    if (k.startsWith('#x')) return String.fromCodePoint(parseInt(k.slice(2), 16) || 0xfffd);
    if (k.startsWith('#')) return String.fromCodePoint(parseInt(k.slice(1), 10) || 0xfffd);
    return { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: '\u00a0' }[k] ?? '';
  });
}

/** Scope every `<style>` in a sanitized Mermaid SVG to its root `<svg id>`.
 *  Without a safe scope id, `<style>` bodies are emptied (fail closed). */
export function scopeMermaidStyles(svg: string, parse: CssParser = parseCssBrowser): string {
  const scopeId = mermaidScopeId(svg);
  return svg.replace(/(<style\b[^>]*>)([\s\S]*?)(<\/style>)/gi, (_m, open: string, css: string, close: string) =>
    scopeId ? `${open}${escapeHtml(scopeMermaidCss(decodeStyleText(css), scopeId, parse))}${close}` : `${open}${close}`,
  );
}

/** Sanitize a Mermaid SVG string. Returns `null` (fail closed) if the purifier
 *  cannot run or no `<svg` survives. */
export function sanitizeMermaidSvg(svg: string, purify: DOMPurify, parse: CssParser = parseCssBrowser): string | null {
  if (!purify.isSupported) return null;
  purify.addHook('uponSanitizeAttribute', mermaidAttributeHook);
  try {
    const out = purify.sanitize(svg, { ...MERMAID_PURIFY_CONFIG, RETURN_TRUSTED_TYPE: false });
    const s = String(out);
    return s.includes('<svg') ? scopeMermaidStyles(s, parse) : null;
  } finally {
    purify.removeHook('uponSanitizeAttribute', mermaidAttributeHook);
  }
}
