import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

async function flushAsync(): Promise<void> {
  await new Promise((r) => setTimeout(r, 0));
}

test('agentEmojiFor resets loading after a failed load so a subsequent render retries and succeeds', async () => {
  let attempts = 0;
  const mod = loadSource(
    new URL('../src/lib/agentEmojiRender.svelte.ts', import.meta.url),
    {
      './stores/agentEmoji.svelte': { agentEmoji: { on: true } },
      './emojiShortcodes': {
        loadEmojiTable: async () => {
          attempts++;
          if (attempts === 1) throw new Error('transient chunk fetch failure');
        },
        expandEmojiHtml: (html: string) => html.replace(':rocket:', '🚀'),
      },
    },
  );

  // First call kicks off the async table load, which rejects.
  assert.equal(mod.agentEmojiFor('ship :rocket:'), false);
  await flushAsync();
  assert.equal(attempts, 1);

  // Because .finally() reset `loading = false`, the next call retries and loads cleanly.
  assert.equal(mod.agentEmojiFor('ship :rocket:'), false);
  await flushAsync();
  assert.equal(attempts, 2);

  assert.equal(mod.agentEmojiFor('ship :rocket:'), true);
  assert.equal(mod.agentEmojiHtml('ship :rocket:', '<p>ship :rocket:</p>'), '<p>ship 🚀</p>');
});

test('agentEmojiFor stays idle when setting is off or text has no shortcodes', async () => {
  let attempts = 0;
  const mod = loadSource(
    new URL('../src/lib/agentEmojiRender.svelte.ts', import.meta.url),
    {
      './stores/agentEmoji.svelte': { agentEmoji: { on: false } },
      './emojiShortcodes': {
        loadEmojiTable: async () => {
          attempts++;
        },
        expandEmojiHtml: (html: string) => html,
      },
    },
  );

  assert.equal(mod.agentEmojiFor('ship :rocket:'), false);
  await flushAsync();
  assert.equal(attempts, 0);
});
