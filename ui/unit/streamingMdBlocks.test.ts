// P1-007: a streaming answer renders each settled block once. It used to
// re-render the whole settled prefix on every new paragraph (O(P²) marked +
// hljs + sanitize) and hand Markdown.svelte one html string, so every 32 ms
// flush replaced the message's whole DOM — Mermaid diagrams re-drawn and
// workspace images re-fetched through /fs/raw on every delta.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { splitStreamingMarkdown, streamingBlocksRenderer } from '../src/modules/agents/conversation/streamingMd.ts';

const para = (i: number): string => `Paragraph ${i}: ${'prose that keeps going for a while. '.repeat(6).trim()}\n\n`;
const doc =
  '# Title\n\n' +
  Array.from({ length: 40 }, (_, i) => para(i) + (i % 5 === 0 ? `\`\`\`ts\nconst a = ${i};\n\nconst b = 2;\n\`\`\`\n\n` : '')).join('') +
  '```mermaid\ngraph TD\n  A --> B\n```\n\nThe end.';

test('streamed in 40-char deltas, every settled block renders exactly once and keeps its html', () => {
  // Settled-block renders only (the open tail renders on every delta).
  const renders = new Map<string, number>();
  let tailNow = '';
  const render = (md: string): string => {
    if (md !== tailNow) renders.set(md, (renders.get(md) ?? 0) + 1);
    return `<x>${md.length}</x>`;
  };
  const next = streamingBlocksRenderer(render);
  const seen: string[] = [];
  for (let n = 40; n < doc.length + 40; n += 40) {
    const md = doc.slice(0, n);
    const { blocks, tail } = splitStreamingMarkdown(md);
    tailNow = tail;
    const parts = next(md);
    assert.equal(parts.length, blocks.length + (tail ? 1 : 0));
    // A settled block's html is the very string it had on the delta before.
    for (let i = 0; i < Math.min(seen.length, blocks.length); i++) assert.equal(parts[i], seen[i], `block ${i} re-rendered`);
    seen.splice(0, seen.length, ...parts.slice(0, blocks.length));
  }
  const { blocks } = splitStreamingMarkdown(doc);
  assert.ok(blocks.length > 40, `${blocks.length} blocks`);
  for (const b of blocks) assert.equal(renders.get(b), 1, `rendered ${renders.get(b)}× : ${b.slice(0, 30)}`);
});

test('cuts only between whole blocks: fences, loose lists and tables stay in one piece', () => {
  const long = 'x'.repeat(400);
  const md = `${long}\n\n- a\n\n- b\n\n1. one\n\n2. two\n\n| h |\n|---|\n\n| r |\n\n\`\`\`js\nlet a;\n\nlet b;\n\`\`\`\n\nTail`;
  const { blocks, tail } = splitStreamingMarkdown(md);
  assert.equal(blocks.join('') + tail, md);
  // A list never starts a block: the paragraph before it is merged in.
  assert.deepEqual(blocks, [`${long}\n\n- a\n\n- b\n\n1. one\n\n2. two\n\n| h |\n|---|\n\n| r |\n\n`, '```js\nlet a;\n\nlet b;\n```\n\n']);
  assert.equal(tail, 'Tail');
});

// P2-009: what CommonMark reads as no fence, or as one HTML block, never
// changes where the stream is cut.
const lead = `${'x'.repeat(400)}\n\n`;

test('an indented code block or an inline ```span``` opens no fence: the blocks after it still settle', () => {
  for (const odd of ['    ```not a fence\n    still code\n\n', 'Use ```foo``` inline.\n\n']) {
    const md = `${lead}${odd}Next paragraph.\n\nThird paragraph.\n\nTail`;
    const { blocks, tail } = splitStreamingMarkdown(md);
    assert.equal(blocks.join('') + tail, md);
    assert.equal(tail, 'Tail', JSON.stringify(odd));
  }
});

test('a fence indented up to three spaces still holds its blank lines', () => {
  const md = `${lead}   \`\`\`js\n   let a;\n\nlet b;\n   \`\`\`\n\nTail`;
  // (An indented line never starts a block, so the paragraph before it is merged in.)
  assert.deepEqual(splitStreamingMarkdown(md).blocks, [`${lead}   \`\`\`js\n   let a;\n\nlet b;\n   \`\`\`\n\n`]);
});

test('blank lines inside a <details> / <div> block are not cuts', () => {
  const html = '<details>\n<summary>More</summary>\n\nBody text.\n\n<div>\n\ninner\n\n</div>\n\n</details>\n\n';
  const md = `${lead}${html}After.\n\nTail`;
  const { blocks, tail } = splitStreamingMarkdown(md);
  assert.deepEqual(blocks, [lead, html, 'After.\n\n']);
  assert.equal(tail, 'Tail');
});
