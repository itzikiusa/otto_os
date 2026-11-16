// A3: raw `<…>` in agent prose. `Vec<String>` and `<name>` are text the
// sanitizer would unwrap into nothing; only tags it keeps stay HTML.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { Marked } from 'marked';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const { chatMarkedExtension } = await import('../src/modules/agents/conversation/chatMarkdown.ts');

const render = (md: string): string => new Marked(chatMarkedExtension).parse(md, { async: false }) as string;

test('generic types and placeholders render as written', () => {
  const html = render('Use Vec<String> or Option<T> and fill <routine> in the <name> slot');
  assert.match(html, /Vec&lt;String&gt;/);
  assert.match(html, /Option&lt;T&gt;/);
  assert.match(html, /&lt;routine&gt;/);
  assert.match(html, /&lt;name&gt;/);
});

test('tags the sanitizer keeps stay HTML', () => {
  assert.match(render('a<br>b'), /a<br>b/);
  assert.match(render('<details>\n<summary>More</summary>\n\nbody\n</details>'), /<details>/);
});
