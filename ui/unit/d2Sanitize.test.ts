// D2 SVG goes into innerHTML sinks (Markdown, D2Canvas, FileViewer,
// DiagramPreview, CanvasPanel), so `renderD2` purifies it centrally. node:test
// has no DOM, so this drives the sanitizer's own policy — the attribute hook
// DOMPurify calls for every attribute, and the fail-closed path — with the
// payloads that matter: a `javascript:` link and an `<img onerror>` label.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  D2_PURIFY_CONFIG,
  d2AttributeHook,
  sanitizeD2Svg,
  d2ScopeClass,
  isScopedSelector,
  scopeD2Css,
  scopeD2Styles,
  scopeSelector,
  splitSelectorList,
  svgUrlAllowed,
  type CssNode,
} from '../src/modules/canvas/svgSanitize.ts';

function attr(attrName: string, attrValue: string) {
  const ev = { attrName, attrValue, keepAttr: true, allowedAttributes: {}, forceKeepAttr: undefined };
  d2AttributeHook({} as Element, ev);
  return ev.keepAttr;
}

test('javascript: links are dropped (href and xlink:href, obfuscated too)', () => {
  assert.equal(attr('href', 'javascript:alert(1)'), false);
  assert.equal(attr('xlink:href', 'JavaScript:alert(1)'), false);
  assert.equal(attr('href', 'java\tscript:alert(1)'), false);
  assert.equal(attr('href', ' \u0001javascript:alert(1)'), false);
  assert.equal(attr('href', 'data:text/html,<script>alert(1)</script>'), false);
  assert.equal(attr('href', 'vbscript:x'), false);
});

test('http(s) links and in-document fragments survive', () => {
  assert.equal(attr('href', 'https://example.com/x'), true);
  assert.equal(attr('xlink:href', 'http://example.com'), true);
  assert.equal(attr('href', '#marker-arrow'), true);
  assert.equal(svgUrlAllowed('mailto:a@b.c'), false);
});

test('event handlers are dropped regardless of element (img onerror payload)', () => {
  assert.equal(attr('onerror', 'alert(1)'), false);
  assert.equal(attr('ONLOAD', 'alert(1)'), false);
  assert.equal(attr('onclick', ''), false);
  // Ordinary presentation attributes are untouched.
  assert.equal(attr('fill', '#fff'), true);
  assert.equal(attr('class', 'shape'), true);
});

test('config keeps foreignObject labels but forbids active content', () => {
  assert.ok((D2_PURIFY_CONFIG.ADD_TAGS as string[]).includes('foreignObject'));
  for (const t of ['script', 'iframe', 'object', 'embed']) {
    assert.ok((D2_PURIFY_CONFIG.FORBID_TAGS as string[]).includes(t), t);
  }
});

test('fails closed when the purifier cannot run (no DOM)', () => {
  const payload =
    '<svg><a href="javascript:alert(1)"><text>x</text></a>' +
    '<foreignObject><img src="x" onerror="alert(1)"></foreignObject></svg>';
  const unsupported = {
    isSupported: false,
    addHook() { throw new Error('must not be reached'); },
    removeHook() {},
    sanitize() { throw new Error('must not be reached'); },
  };
  assert.equal(sanitizeD2Svg(payload, unsupported as never), null);
});

test('hook is installed only for the duration of one sanitize call', () => {
  const calls: string[] = [];
  const fake = {
    isSupported: true,
    addHook(name: string) { calls.push(`add:${name}`); },
    removeHook(name: string) { calls.push(`remove:${name}`); },
    sanitize(s: string) { calls.push('sanitize'); return s.replace(/ onerror="[^"]*"/, ''); },
  };
  const out = sanitizeD2Svg('<svg><img onerror="alert(1)"></svg>', fake as never);
  assert.equal(out, '<svg><img></svg>');
  assert.deepEqual(calls, ['add:uponSanitizeAttribute', 'sanitize', 'remove:uponSanitizeAttribute']);
  // Nothing SVG survived → caller gets an error, never raw markup.
  const empty = { ...fake, sanitize: () => '' };
  assert.equal(sanitizeD2Svg('<svg onload="x"></svg>', empty as never), null);
});

// S11-12 / S18-301 / S11-311: `<style>` survives purification, so its rules
// are re-scoped to the diagram's own `d2-<id>` class — a rule can no longer
// restyle the app. Production parses with the browser's CSSStyleSheet; node
// has none, so these tests drive the emitter through `cssom`, a small
// spec-shaped stand-in that (like the browser) honours strings, escapes and
// comments when finding rule boundaries.

type Decl = [string, string, boolean];

function cssom(css: string): CssNode[] {
  let i = 0;
  const src = css;
  const skipWs = () => {
    for (;;) {
      while (i < src.length && /\s/.test(src[i])) i++;
      if (src.startsWith('/*', i)) {
        const e = src.indexOf('*/', i + 2);
        i = e < 0 ? src.length : e + 2;
      } else return;
    }
  };
  // Read up to (not past) the first top-level char in `stops`, honouring
  // strings / escapes / comments / nested blocks like a CSS tokenizer.
  const until = (stops: string): string => {
    let out = '';
    let depth = 0;
    while (i < src.length) {
      const ch = src[i];
      if (ch === '"' || ch === "'") {
        let j = i + 1;
        while (j < src.length && src[j] !== ch && src[j] !== '\n') j += src[j] === '\\' ? 2 : 1;
        out += src.slice(i, j + 1);
        i = j + 1;
        continue;
      }
      if (ch === '\\') { out += src.slice(i, i + 2); i += 2; continue; }
      if (src.startsWith('/*', i)) { const e = src.indexOf('*/', i + 2); i = e < 0 ? src.length : e + 2; continue; }
      if (depth === 0 && stops.includes(ch)) break;
      if (ch === '(' || ch === '[' || ch === '{') depth++;
      if (ch === ')' || ch === ']' || ch === '}') depth--;
      out += ch;
      i++;
    }
    return out.trim();
  };
  const decls = (): Decl[] => {
    const out: Decl[] = [];
    while (i < src.length && src[i] !== '}') {
      const d = until(';}');
      if (src[i] === ';') i++;
      const c = d.indexOf(':');
      if (c < 0) continue;
      let v = d.slice(c + 1).trim();
      const imp = /!\s*important$/i.test(v);
      if (imp) v = v.replace(/!\s*important$/i, '').trim();
      out.push([d.slice(0, c).trim().toLowerCase(), v, imp]);
    }
    i++; // `}`
    return out;
  };
  const rules = (nested: boolean): CssNode[] => {
    const out: CssNode[] = [];
    for (;;) {
      skipWs();
      if (i >= src.length) return out;
      if (src[i] === '}') { if (nested) { i++; return out; } i++; continue; }
      const prelude = until('{;').replace(/\s+/g, ' ');
      if (src[i] === ';') { i++; out.push({ type: 'other' }); continue; }
      if (i >= src.length) return out;
      i++; // `{`
      const at = /^@([\w-]+)\s*(.*)$/.exec(prelude);
      if (!at) { out.push({ type: 'style', selector: prelude, decls: decls() }); continue; }
      const kw = at[1].toLowerCase();
      if (kw === 'media') out.push({ type: 'media', condition: at[2], rules: rules(true) });
      else if (kw === 'font-face') out.push({ type: 'font-face', decls: decls() });
      else if (kw.endsWith('keyframes')) {
        const frames: Array<{ key: string; decls: Decl[] }> = [];
        for (;;) {
          skipWs();
          if (i >= src.length || src[i] === '}') { i++; break; }
          const key = until('{');
          i++;
          frames.push({ key, decls: decls() });
        }
        out.push({ type: 'keyframes', name: at[2], frames });
      } else { rules(true); out.push({ type: 'other' }); }
    }
  };
  return rules(false);
}

/** Every top-level rule of `css` (as the stand-in parses it) is either a
 *  style rule anchored on the scope or an allowed at-rule. */
function allScoped(css: string, scope: string): boolean {
  const walk = (ns: CssNode[]): boolean =>
    ns.every((n) =>
      n.type === 'style'
        ? n.selector.split(',').every((s) => s.trim().startsWith(`.${scope}`))
        : n.type === 'media' ? walk(n.rules) : n.type !== 'other',
    );
  return walk(cssom(css));
}

test('D2 style rules are scoped to the diagram', () => {
  const css =
    '/* c */ @import url(https://evil.example/x.css); .d2-42 .fill-N1{fill:#000} ' +
    'body, input[value^="a"]{color:red} ' +
    '@font-face{font-family:d2-42-font-regular;src:url("data:font/woff;base64,AA")} ' +
    '@keyframes dashdraw{from{stroke-dashoffset:0}} @keyframes d2Transition-d2-42-0{0%{opacity:0}} ' +
    '@media (min-width:1px){a{color:red}} @supports (x:y){b{c:d}}';
  const out = scopeD2Css(css, 'd2-42', cssom);
  assert.ok(!out.includes('@import'), out);
  assert.ok(!out.includes('@supports'), out);
  assert.ok(out.includes('.d2-42 .fill-N1{fill:#000}'), out);
  assert.ok(out.includes('.d2-42 body,.d2-42 input[value^="a"]{color:red}'), out);
  assert.ok(out.includes('@font-face{font-family:d2-42-font-regular;src:url("data:font/woff;base64,AA")}'), out);
  assert.ok(out.includes('@keyframes dashdraw{'), out);
  assert.ok(out.includes('@keyframes d2Transition-d2-42-0{'), out);
  assert.ok(out.includes('@media (min-width:1px){.d2-42 a{color:red}}'), out);
  assert.ok(allScoped(out, 'd2-42'), out);
});

test('a string-embedded brace or comment cannot end a rule early (S18-301)', () => {
  for (const css of [
    'x{content:"}"; } body{display:none}',
    "x{content:'}'} body{display:none}",
    'x{content:"/*"} body{display:none} /* */',
    'x{content:"\\"}"} body{display:none}',
    'x{content:"a\\}"} body{display:none}',
  ]) {
    const out = scopeD2Css(css, 'd2-1', cssom);
    assert.ok(allScoped(out, 'd2-1'), `${css} → ${out}`);
    assert.ok(out.includes('.d2-1 body{display:none}'), `${css} → ${out}`);
    assert.ok(!/(^|})\s*body/.test(out), `${css} → ${out}`);
  }
});

test('a scope-anchored selector cannot hop to the app with ~ or + (S11-311)', () => {
  for (const sel of ['.d2-1 ~ *', '.d2-1~div', '.d2-1:not(.x) ~ div', '.d2-1.a + b', '.d2-1[title="x"] + p']) {
    assert.equal(scopeSelector(sel, 'd2-1'), null, sel);
    assert.ok(!isScopedSelector(sel, 'd2-1'), sel);
  }
  // Siblings INSIDE the diagram stay legal, as do `>` and descendants.
  assert.equal(scopeSelector('.d2-1 .a ~ .b', 'd2-1'), '.d2-1 .a ~ .b');
  assert.equal(scopeSelector('.d2-1 > g', 'd2-1'), '.d2-1 > g');
  assert.equal(scopeSelector('.d2-1:hover .x', 'd2-1'), '.d2-1:hover .x');
  assert.equal(scopeSelector(':nth-child(2n+1)', 'd2-1'), '.d2-1 :nth-child(2n+1)');
  assert.equal(scopeSelector('[class~="x"]', 'd2-1'), '.d2-1 [class~="x"]');
  assert.equal(scopeSelector('.d2-12 a', 'd2-1'), '.d2-1 .d2-12 a');
  // One escaping selector drops the whole rule (no half-scoped lists).
  assert.equal(scopeD2Css('.d2-1 ~ *, .x{color:red}', 'd2-1', cssom), '');
  // Commas inside :is() / strings don't split the list.
  assert.deepEqual(splitSelectorList(':is(a, b) c, [t="x,y"]'), [':is(a, b) c', '[t="x,y"]']);
});

test('foreign @font-face / @keyframes names and fetching values are dropped', () => {
  const out = scopeD2Css(
    '@font-face{font-family:system-ui;src:url(data:font/woff;base64,AA)} ' +
      '@font-face{font-family:d2-1-font-bold;src:url(https://evil.example/f.woff)} ' +
      '@keyframes spin{to{opacity:0}} @keyframes d2Transition-d2-2-0{to{opacity:0}} ' +
      '.a{background:url(https://evil.example/?a);color:red} ' +
      '.b{background-image:image-set("https://evil.example/i.png" 1x)} .c{filter:url(#shadow)}',
    'd2-1',
    cssom,
  );
  assert.ok(!out.includes('system-ui'), out);
  assert.ok(!out.includes('evil.example'), out);
  assert.ok(!out.includes('spin'), out);
  assert.ok(!out.includes('d2-2'), out);
  assert.ok(out.includes('.d2-1 .a{color:red}'), out);
  assert.ok(out.includes('.d2-1 .c{filter:url(#shadow)}'), out);
  assert.ok(!out.includes('@font-face{font-family:d2-1-font-bold'), out);
});

test('no browser CSS parser → styles dropped (fail closed)', () => {
  assert.equal(scopeD2Css('.x{color:red}', 'd2-1'), '');
  assert.equal(scopeD2Css('.x{color:red}', 'not-a-scope', cssom), '');
});

test('styles without a diagram scope class are dropped', () => {
  const svg = '<svg><style>body{display:none}</style><g/></svg>';
  assert.equal(scopeD2Styles(svg, cssom), '<svg><style></style><g/></svg>');
  const scoped = scopeD2Styles('<svg class="d2-7 d2-svg"><style>.x{a:b}</style></svg>', cssom);
  assert.equal(scoped, '<svg class="d2-7 d2-svg"><style>.d2-7 .x{a:b}</style></svg>');
});

test('scope class comes from D2\'s wrapper <svg> tags only, not a label element', () => {
  assert.equal(d2ScopeClass('<svg class="d2-svg d2-7"><g class="d2-99"/></svg>'), 'd2-7');
  // D2's real shape: an unclassed wrapper, then `<svg class="d2-N d2-svg">`.
  assert.equal(d2ScopeClass('<svg data-d2-version="v0.7"><svg class="d2-5 d2-svg"><g class="d2-99"/></svg></svg>'), 'd2-5');
  assert.equal(d2ScopeClass('<svg viewBox="0 0 1 1"><g class="d2-99"/></svg>'), null);
  assert.equal(d2ScopeClass('<svg><svg><g><svg class="d2-99"/></g></svg></svg>'), null);
  assert.equal(d2ScopeClass('<svg data-x=">" class="d2-3"></svg>'), 'd2-3');
});

test('serialized style text is entity-decoded for parsing and re-escaped after', () => {
  const out = scopeD2Styles(
    '<svg class="d2-7"><style>.a &gt; .b{content:"&lt;/style&gt;&lt;img src=x onerror=alert(1)&gt;"}</style></svg>',
    cssom,
  );
  assert.ok(out.includes('.d2-7 .a &gt; .b{content:"&lt;/style&gt;&lt;img'), out);
  assert.ok(!/<img|<\/style><img/.test(out), out);
});
