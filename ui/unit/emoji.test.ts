import assert from 'node:assert/strict';
import { register } from 'node:module';
import { before, describe, it } from 'node:test';
import { Marked } from 'marked';
import { applyEmojiEdit, EmoticonConverter, emoticonBefore, inCode, shortcodeQuery } from '../src/lib/emojiInput.ts';
import {
  expandEmojiHtml,
  loadEmojiTable,
  replaceEmojiShortcodes,
  resolveEmojiShortcode,
  searchEmojiShortcodes,
} from '../src/lib/emojiShortcodes.ts';

const tsResolveHook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register(`data:text/javascript,${encodeURIComponent(tsResolveHook)}`);
const { chatMarkedExtension, chatEmojiExtension } = await import('../src/modules/agents/conversation/chatMarkdown.ts');

/** Type `keys` into a textarea model the way EmojiAssist does: an edit from
 *  the converter replaces the key's default. */
function typeInto(conv: EmoticonConverter, start: string, keys: string[]): { text: string; caret: number } {
  let text = start;
  let caret = text.length;
  for (const key of keys) {
    const edit = conv.key(key, text, caret, caret);
    if (edit) {
      text = applyEmojiEdit(text, edit);
      caret = edit.from + edit.insert.length;
    } else if (key === 'Backspace') {
      text = text.slice(0, caret - 1) + text.slice(caret);
      caret -= 1;
    } else if (key.length === 1) {
      text = text.slice(0, caret) + key + text.slice(caret);
      caret += 1;
    }
  }
  return { text, caret };
}
const chars = (s: string): string[] => [...s];

describe('emoticon converter (composer)', () => {
  it('converts only when a space follows the emoticon', () => {
    const c = new EmoticonConverter();
    assert.equal(typeInto(c, '', chars('hi :)')).text, 'hi :)');
    assert.equal(typeInto(c, '', chars('hi :) ')).text, 'hi 🙂 ');
    assert.equal(typeInto(new EmoticonConverter(), '', chars('<3 ')).text, '❤️ ');
    assert.equal(typeInto(new EmoticonConverter(), '', chars(":'( ")).text, '😢 ');
    assert.equal(typeInto(new EmoticonConverter(), '', chars('ok :-D ')).text, 'ok 😄 ');
  });

  it('Backspace deletes the space, then restores the typed text', () => {
    const c = new EmoticonConverter();
    assert.equal(typeInto(c, '', [...chars('hi :) '), 'Backspace']).text, 'hi 🙂');
    const c2 = new EmoticonConverter();
    assert.equal(typeInto(c2, '', [...chars('hi :) '), 'Backspace', 'Backspace']).text, 'hi :)');
  });

  it('a restored emoticon stays until another space is typed after it', () => {
    const c = new EmoticonConverter();
    const undone = typeInto(c, '', [...chars('hi :) '), 'Backspace', 'Backspace', ...chars('x')]);
    assert.equal(undone.text, 'hi :)x');
    const c2 = new EmoticonConverter();
    assert.equal(typeInto(c2, '', [...chars('hi :) '), 'Backspace', 'Backspace', ' ']).text, 'hi 🙂 ');
  });

  it('only a FRESH conversion is undone', () => {
    const c = new EmoticonConverter();
    // Typing after the emoji, then deleting back to it, is ordinary editing.
    const back = typeInto(c, '', [...chars(':) a'), 'Backspace', 'Backspace']);
    assert.equal(back.text, '🙂');
    assert.equal(c.key('Backspace', back.text, back.caret, back.caret), null);
    const c2 = new EmoticonConverter();
    assert.equal(c2.key('Backspace', '🙂', 2, 2), null);
  });

  it('never converts inside code, a word or a URL', () => {
    for (const s of ['`x :)', '```\nlet a = :)', 'a:)', 'http://x.com/:P', 'see(:)']) {
      assert.equal(typeInto(new EmoticonConverter(), s, [' ']).text, `${s} `, s);
    }
    // After the inline code span closes, it converts again.
    assert.equal(typeInto(new EmoticonConverter(), '`code` :)', [' ']).text, '`code` 🙂 ');
    // After a closed fence too.
    assert.equal(typeInto(new EmoticonConverter(), '```\nx\n```\n:)', [' ']).text, '```\nx\n```\n🙂 ');
  });

  it('a selection or a non-emoticon does nothing', () => {
    const c = new EmoticonConverter();
    assert.equal(c.key(' ', 'a :)', 2, 4), null);
    assert.equal(c.key(' ', 'hello', 5, 5), null);
    assert.equal(emoticonBefore(':)', 2)?.emoji, '🙂');
    assert.equal(emoticonBefore('x:)', 3), null);
  });

  it('inCode tracks fences and inline spans', () => {
    assert.equal(inCode('a `b', 4), true);
    assert.equal(inCode('a `b` c', 7), false);
    assert.equal(inCode('```\nfoo', 7), true);
    assert.equal(inCode('```\nfoo\n```\nbar', 15), false);
  });
});

describe(':shortcode completion query', () => {
  it('needs a word-start colon and 2+ name characters', () => {
    assert.deepEqual(shortcodeQuery(':smi', 4), { start: 0, query: 'smi' });
    assert.deepEqual(shortcodeQuery('hi :Rock', 8), { start: 3, query: 'rock' });
    assert.equal(shortcodeQuery(':s', 2), null);
    assert.equal(shortcodeQuery('a:smi', 5), null);
    assert.equal(shortcodeQuery('http://x:smi', 12), null);
    assert.equal(shortcodeQuery('`:smi', 5), null);
    assert.equal(shortcodeQuery('```\n:smi', 8), null);
  });
});

describe('GitHub emoji table', () => {
  before(() => loadEmojiTable());

  it('resolves GitHub aliases exactly', () => {
    assert.equal(resolveEmojiShortcode('rocket'), '🚀');
    assert.equal(resolveEmojiShortcode('+1'), '👍');
    assert.equal(resolveEmojiShortcode('thumbsup'), '👍');
    assert.equal(resolveEmojiShortcode('Rocket'), null);
    assert.equal(resolveEmojiShortcode('not_a_real_shortcode'), null);
  });

  it('searches prefix hits first, one row per emoji', () => {
    const hits = searchEmojiShortcodes('smi', 8);
    assert.ok(hits.length > 0 && hits.length <= 8);
    assert.ok(hits.every((h) => h.shortcode.startsWith('smi')));
    assert.equal(new Set(hits.map((h) => h.emoji)).size, hits.length);
    assert.ok(searchEmojiShortcodes('ocke').some((h) => h.shortcode === 'rocket'));
    // A popular shortcode leads its prefix: `:roc` + Enter is 🚀, not 🪨 (`:rock:`).
    assert.equal(searchEmojiShortcodes('roc')[0]?.shortcode, 'rocket');
  });

  it('expands plain text but keeps URLs, ports, escapes and unknown names', () => {
    assert.equal(
      replaceEmojiShortcodes('Ship :rocket::+1: via https://localhost:7700/:fire: at 12:30:45, \\:tada: and :nope_x:'),
      'Ship 🚀👍 via https://localhost:7700/:fire: at 12:30:45, \\:tada: and :nope_x:',
    );
  });

  it('expands rendered HTML text only — never tags, code or pre', () => {
    const html = '<p title=":fire:">go :rocket: <code>:fire:</code> <a href="/x">:tada:</a></p><pre><code>:bug:\n</code></pre>';
    assert.equal(
      expandEmojiHtml(html),
      '<p title=":fire:">go 🚀 <code>:fire:</code> <a href="/x">🎉</a></p><pre><code>:bug:\n</code></pre>',
    );
  });

  it('chat markdown expands prose only when the emoji extension is on', () => {
    const md = 'Deployed :sparkles: `:rocket:` \\:tada: https://x.dev/:fire: :unknown_x: :)\n\n```ts\nconst s = ":bug:";\n```';
    const on = new Marked({ gfm: true, breaks: true }).use(chatMarkedExtension, chatEmojiExtension).parse(md) as string;
    assert.ok(on.includes('✨'), 'prose shortcode');
    assert.ok(on.includes('<code>:rocket:</code>'), 'inline code stays literal');
    assert.ok(on.includes(':tada:') && !on.includes('🎉'), 'escaped stays literal');
    assert.ok(on.includes('https://x.dev/:fire') && !on.includes('🔥'), 'URL stays literal');
    assert.ok(on.includes(':unknown_x:'), 'unknown stays literal');
    assert.ok(on.includes(':bug:') && !on.includes('🐛'), 'fence stays literal');
    assert.ok(on.includes(':)') && !on.includes('🙂'), 'emoticons are never converted in agent output');
    const off = new Marked({ gfm: true, breaks: true }).use(chatMarkedExtension).parse(md) as string;
    assert.ok(off.includes(':sparkles:') && !off.includes('✨'), 'setting off shows shortcodes as written');
  });
});
