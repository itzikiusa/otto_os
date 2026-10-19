// The incomplete-history banner is presentation. It reads the store accessors
// E08 owns and does not reimplement replay detection. The Svelte compiler is
// the syntax gate; source text only checks the banner contract.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { compile } from 'svelte/compiler';

const path = new URL('../src/modules/api/RequestBuilder.svelte', import.meta.url);
const source = readFileSync(path, 'utf8');

test('incomplete history banner compiles and keeps the saved-request contract', () => {
  const compiled = compile(source, { filename: path.pathname, generate: 'client' });
  assert.ok(compiled.js.code.length > 0);
  assert.equal(compiled.warnings.length, 0);
  assert.match(compiled.js.code, /incomplete-history/);
  assert.match(compiled.js.code, /replayIssue\(\)/);
  assert.match(compiled.js.code, /openHistorySavedRequest\(\)/);

  const start = source.indexOf('data-testid="incomplete-history"');
  assert.ok(start > 0, 'incomplete-history banner is present');
  const banner = source.slice(Math.max(0, start - 240), start + 700);
  assert.doesNotMatch(banner, /\{#if\s+[^\n}]*\bas\b/);
  assert.match(banner, /role="status"/);
  assert.match(banner, /class="tab-help"/);
  assert.match(banner, /apiClient\.replayIssue\(\)/);
  assert.match(banner, /draft\.historyReplay\?\.requestId/);
  assert.match(banner, /apiClient\.openHistorySavedRequest\(\)/);
  assert.match(banner, />Open current saved request</);
  assert.doesNotMatch(source, /history-gap/);

  assert.throws(() =>
    compile('{#if apiClient.replayIssue() as issue}<p>{issue}</p>{/if}', {
      filename: 'IncompleteHistoryBanner.svelte',
      generate: 'client',
    }),
  );
});


