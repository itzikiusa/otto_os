import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MERMAID_PURIFY_CONFIG,
  MERMAID_SECURE_KEYS,
  mermaidAttributeHook,
  mermaidFragmentAllowed,
  mermaidScopeId,
  mermaidUrlAllowed,
  safeMermaidCssDecl,
  safeMermaidCssValue,
  sanitizeMermaidInlineStyle,
  sanitizeMermaidSvg,
  scopeMermaidCss,
  scopeMermaidStyles,
  type CssNode,
} from '../src/lib/render/mermaidSanitize.ts';

function attrEvent(attrName: string, attrValue: string, tagName = 'a') {
  const ev = { attrName, attrValue, keepAttr: true, allowedAttributes: {}, forceKeepAttr: undefined };
  mermaidAttributeHook({ tagName } as Element, ev);
  return ev;
}

function attr(attrName: string, attrValue: string, tagName = 'a'): boolean {
  return attrEvent(attrName, attrValue, tagName).keepAttr;
}

/** Lightweight CSSOM parser stand-in for Node (production uses CSSStyleSheet). */
function mockParseCss(css: string): CssNode[] {
  const nodes: CssNode[] = [];
  const src = css.trim();
  if (!src) return nodes;
  let i = 0;
  const skipWs = () => { while (i < src.length && /\s/.test(src[i])) i++; };
  const parseDecls = (body: string): [string, string, boolean][] =>
    body
      .split(';')
      .map((s) => s.trim())
      .filter(Boolean)
      .map((d) => {
        const c = d.indexOf(':');
        if (c < 0) return null;
        const name = d.slice(0, c).trim();
        let val = d.slice(c + 1).trim();
        const imp = /\s*!important$/i.test(val);
        if (imp) val = val.replace(/\s*!important$/i, '').trim();
        return [name, val, imp] as [string, string, boolean];
      })
      .filter((x): x is [string, string, boolean] => x !== null);
  const readBlock = (): string => {
    let depth = 0;
    const start = i;
    while (i < src.length) {
      if (src[i] === '{') depth++;
      else if (src[i] === '}') {
        depth--;
        if (depth === 0) {
          const inner = src.slice(start + 1, i);
          i++;
          return inner;
        }
      }
      i++;
    }
    return '';
  };
  while (i < src.length) {
    skipWs();
    if (i >= src.length) break;
    if (src[i] === '@') {
      const semi = src.indexOf(';', i);
      const brace = src.indexOf('{', i);
      if (semi >= 0 && (brace < 0 || semi < brace)) {
        i = semi + 1;
        nodes.push({ type: 'other' });
        continue;
      }
      const head = src.slice(i, brace).trim();
      i = brace;
      const body = readBlock();
      if (head.startsWith('@media ')) {
        nodes.push({ type: 'media', condition: head.slice(7).trim(), rules: mockParseCss(body) });
      } else if (head.startsWith('@keyframes ')) {
        const frames: Array<{ key: string; decls: [string, string, boolean][] }> = [];
        const re = /([^{}]+)\{([^{}]*)\}/g;
        let m: RegExpExecArray | null;
        while ((m = re.exec(body)) !== null) {
          frames.push({ key: m[1].trim(), decls: parseDecls(m[2]) });
        }
        nodes.push({ type: 'keyframes', name: head.slice(11).trim(), frames });
      } else {
        nodes.push({ type: 'other' });
      }
      continue;
    }
    const brace = src.indexOf('{', i);
    if (brace < 0) break;
    const selector = src.slice(i, brace).trim();
    i = brace;
    const body = readBlock();
    nodes.push({ type: 'style', selector, decls: parseDecls(body) });
  }
  return nodes;
}

test('MERMAID_SECURE_KEYS locks themeCSS and typography directives', () => {
  for (const key of ['securityLevel', 'themeCSS', 'fontFamily', 'altFontFamily', 'themeVariables']) {
    assert.ok((MERMAID_SECURE_KEYS as readonly string[]).includes(key), `missing ${key}`);
  }
});

test('attribute hook drops event handlers, restricts external URLs to <a>, and forbids image tags', () => {
  assert.equal(attr('href', 'javascript:alert(1)', 'a'), false);
  assert.equal(attr('xlink:href', 'java\tscript:alert(1)', 'a'), false);
  assert.equal(attr('href', 'data:text/html,<script>1</script>', 'a'), false);
  assert.equal(attr('onerror', 'alert(1)', 'g'), false);
  assert.equal(attr('ONCLICK', 'x()', 'g'), false);
  assert.equal(attr('href', 'https://example.com', 'a'), true);
  assert.equal(attr('href', '#arrow-marker', 'use'), true);
  assert.equal(attr('xlink:href', '#clip-1', 'use'), true);
  // External URLs on non-<a> elements are dropped even if a tag slips through.
  assert.equal(attr('src', 'https://evil.example/beacon', 'img'), false);
  assert.equal(attr('href', 'https://evil.example/beacon', 'image'), false);
  assert.equal(attr('xlink:href', 'https://evil.example/beacon', 'feImage'), false);
  assert.equal(attr('href', 'https://evil.example/defs.svg#x', 'use'), false);
  assert.equal(attr('src', 'https://evil.example/beacon', 'a'), false);
  assert.equal(attr('href', 'https://evil.example/beacon', ''), false);
  assert.equal(mermaidFragmentAllowed('#marker'), true);
  assert.equal(mermaidFragmentAllowed('https://evil.example'), false);
  assert.equal(mermaidUrlAllowed('file:///etc/passwd'), false);

  const forbidden = (MERMAID_PURIFY_CONFIG.FORBID_TAGS as string[]).map((t) => t.toLowerCase());
  for (const tag of ['script', 'img', 'image', 'feimage']) {
    assert.ok(forbidden.includes(tag), `FORBID_TAGS must include ${tag}`);
  }
});

test('inline style attribute sanitization strips external beacons and layout-escape overlays', () => {
  const safe = attrEvent(
    'style',
    'max-width: 600px; fill: #fff; stroke: url(#grad1); display: table-cell;',
    'div',
  );
  assert.equal(safe.keepAttr, true);
  assert.equal(safe.attrValue, 'max-width:600px;fill:#fff;stroke:url(#grad1);display:table-cell');

  // Attack payload: beacon + full-viewport overlay.
  const attack = attrEvent(
    'style',
    'background-image:url(https://evil.example/beacon);position:fixed;z-index:999999;color:red',
    'div',
  );
  assert.equal(attack.keepAttr, true);
  assert.equal(attack.attrValue, 'color:red');

  // Purely malicious inline style drops the style attribute entirely.
  const pureAttack = attrEvent(
    'style',
    'background: \\75rl(https://evil.example/x); position: f\\69xed; z-index: 9999;',
    'span',
  );
  assert.equal(pureAttack.keepAttr, false);

  // Sticky / absolute / var() positions are blocked; static / relative stay allowed.
  assert.equal(safeMermaidCssDecl('position', 'fixed'), false);
  assert.equal(safeMermaidCssDecl('position', 'sticky'), false);
  assert.equal(safeMermaidCssDecl('position', 'absolute'), false);
  assert.equal(safeMermaidCssDecl('position', 'var(--pos)'), false);
  assert.equal(safeMermaidCssDecl('position', 'relative'), true);
  assert.equal(safeMermaidCssDecl('position', 'static'), true);
  assert.equal(safeMermaidCssDecl('z-index', '100'), false);
  assert.equal(sanitizeMermaidInlineStyle('color: red; unclosed: "oops'), '');
});

test('scopeMermaidCss confines rules, blocks sideways hops and drops external fetches', () => {
  const input = [
    '#m1 .node rect { fill: #fff; stroke: url(#grad1); }',
    'body { display: none !important; background: url(https://evil.example/beacon); }',
    '#m1 ~ .sidebar { visibility: hidden; }',
    '@import url(https://evil.example/x.css);',
    '@font-face { font-family: evil; src: url(https://evil.example/f.woff2); }',
    '@keyframes dash { from { stroke-dashoffset: 0; } to { stroke-dashoffset: 10; } }',
  ].join('\n');
  const out = scopeMermaidCss(input, 'm1', mockParseCss);
  assert.match(out, /#m1 \.node rect\{fill:#fff;stroke:url\(#grad1\)\}/);
  assert.match(out, /#m1 body\{display:none !important\}/);
  assert.ok(!out.includes('evil.example'), 'external URLs must be stripped');
  assert.ok(!out.includes('.sidebar'), 'sideways ~ combinator must be dropped');
  assert.ok(!out.includes('@import'), '@import must be dropped');
  assert.ok(!out.includes('@font-face'), '@font-face must be dropped');
  assert.match(out, /@keyframes dash\{/);
  assert.equal(safeMermaidCssValue('url(https://evil.example/x)'), false);
  assert.equal(safeMermaidCssValue('image-set(url(#a) 1x)'), false);
  assert.equal(safeMermaidCssValue('url(#arrowhead)'), true);
});

test('scopeMermaidStyles fails closed without a safe root svg id', () => {
  assert.equal(mermaidScopeId('<svg id="canvas-1" viewBox="0 0 10 10"></svg>'), 'canvas-1');
  assert.equal(mermaidScopeId('<svg viewBox="0 0 10 10"></svg>'), null);
  assert.equal(
    scopeMermaidStyles('<svg viewBox="0 0 10 10"><style>body{color:red}</style></svg>', mockParseCss),
    '<svg viewBox="0 0 10 10"><style></style></svg>',
  );
});

test('sanitizeMermaidSvg fails closed without DOMPurify support and cleans hooks', () => {
  const unsupported = {
    isSupported: false,
    addHook() { throw new Error('unexpected'); },
    removeHook() {},
    sanitize() { throw new Error('unexpected'); },
  };
  assert.equal(sanitizeMermaidSvg('<svg id="m1"></svg>', unsupported as never, mockParseCss), null);

  const calls: string[] = [];
  const fake = {
    isSupported: true,
    addHook(n: string) { calls.push(`+${n}`); },
    removeHook(n: string) { calls.push(`-${n}`); },
    sanitize(s: string) { calls.push('sanitize'); return s; },
  };
  const out = sanitizeMermaidSvg(
    '<svg id="m1"><style>body{background:url(https://evil.test/x);color:red}</style><g/></svg>',
    fake as never,
    mockParseCss,
  );
  assert.equal(out, '<svg id="m1"><style>#m1 body{color:red}</style><g/></svg>');
  assert.deepEqual(calls, ['+uponSanitizeAttribute', 'sanitize', '-uponSanitizeAttribute']);
});
