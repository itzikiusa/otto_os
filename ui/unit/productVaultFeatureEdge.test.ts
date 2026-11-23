import test from 'node:test';
import assert from 'node:assert/strict';
import { Marked } from 'marked';
import {
  buildOkfStoryNote,
  defaultVaultNotePath,
  normalizeVaultNotePath,
} from '../src/modules/product/vaultPromote.ts';
import {
  collectOpenThreads,
  formatPinsMarkdown,
  MAX_PINS_MARKDOWN_CHARS,
  pickTargetAgentSession,
} from '../src/modules/design-hall/comments/pinsDispatch.ts';
import type { Thread } from '../src/modules/design-hall/comments/pins.ts';
import {
  consumeVaultDeepLink,
  ottoRefMarkedExtension,
  parseOttoRef,
  parseVaultDeepLink,
} from '../src/modules/vault/ottoRefs.ts';
import type { DesignComment } from '../src/lib/api/types.ts';

function fakeComment(overrides: Partial<DesignComment> = {}): DesignComment {
  return {
    id: 'c-1',
    artifact_id: 'art-1',
    workspace_id: 'ws-1',
    version_id: 'v-3',
    version_seq: 3,
    parent_id: null,
    author_kind: 'user',
    author_id: 'u-1',
    author_name: 'Maya',
    session_id: null,
    body: 'Increase button contrast',
    pin: null,
    resolved: false,
    resolved_by: null,
    resolved_by_name: null,
    resolved_at: null,
    edited_at: null,
    created_at: '2026-11-19T12:00:00Z',
    updated_at: '2026-11-19T12:00:00Z',
    ...overrides,
  };
}

test('vaultPromote builds safe OKF RFC/ADR paths and frontmatter back-linked to the story', () => {
  const story = {
    id: 'story-42',
    source_key: 'PAY-128',
    title: 'Checkout: Retry "Idempotency" Flow',
    stage: 'ready',
    url: 'https://jira.example/browse/PAY-128',
  };

  assert.equal(defaultVaultNotePath(story, 'rfc'), 'rfcs/pay-128-checkout-retry-idempotency-flow.md');
  assert.equal(defaultVaultNotePath(story, 'adr'), 'adrs/pay-128-checkout-retry-idempotency-flow.md');

  assert.equal(normalizeVaultNotePath('/rfcs/my-spec'), 'rfcs/my-spec.md');
  assert.equal(normalizeVaultNotePath('adrs\\001-choice.md'), 'adrs/001-choice.md');
  assert.equal(normalizeVaultNotePath('../escape.md'), null);
  assert.equal(normalizeVaultNotePath('   '), null);

  const md = buildOkfStoryNote({
    story,
    kind: 'rfc',
    bodyMd: 'Retrying timed-out payment intents safely.',
    planMd: '- [ ] Add idempotency key header',
    nowIso: '2026-11-19T10:00:00Z',
  });

  assert.match(md, /^---\ntype: rfc\nstatus: draft\n/);
  assert.ok(md.includes('title: "RFC: Checkout: Retry \\"Idempotency\\" Flow"'));
  assert.ok(md.includes('story_id: "story-42"'));
  assert.ok(md.includes('story_ref: "otto://story/story-42"'));
  assert.ok(md.includes('created: 2026-11-19'));
  assert.ok(md.includes('[PAY-128](otto://story/story-42)'));
  assert.ok(md.includes('## Context & Specification'));
  assert.ok(md.includes('## Implementation Plan'));
  assert.ok(md.includes('- [ ] Add idempotency key header'));
});

test('pinsDispatch orders open pinned threads first, formats coordinates/replies, and caps size', () => {
  const threads: Thread[] = [
    {
      n: 1,
      root: fakeComment({ id: 'c1', body: 'Already fixed', resolved: true, pin: { x: 0.1, y: 0.1 } }),
      replies: [],
    },
    {
      n: 2,
      root: fakeComment({ id: 'c2', body: 'General hierarchy note', pin: null }),
      replies: [],
    },
    {
      n: 3,
      root: fakeComment({
        id: 'c3',
        body: 'Align CTA with input',
        pin: { x: 0.42, y: 0.18, frame: 'Checkout Mobile' },
      }),
      replies: [fakeComment({ id: 'c3-r1', parent_id: 'c3', author_name: 'Avi', body: 'Use 8px gap' })],
    },
  ];

  const open = collectOpenThreads(threads);
  assert.deepEqual(open.map((t) => t.n), [3, 2]);

  const md = formatPinsMarkdown({
    artifactId: 'art-9',
    artifactTitle: 'Checkout Screen',
    versionSeq: 4,
    threads,
  });
  assert.ok(md.includes('`otto://design/art-9@v4`'));
  assert.ok(md.includes('**Pin #3** (frame: Checkout Mobile · 42% across, 18% down · v3) — Maya: Align CTA with input'));
  assert.ok(md.includes('  - Reply (Avi): Use 8px gap'));
  assert.ok(md.includes('**Comment #2** (v3) — Maya: General hierarchy note'));
  assert.ok(!md.includes('Already fixed'));

  // Size-cap check on a large batch of threads.
  const many: Thread[] = Array.from({ length: 40 }, (_, i) => ({
    n: i + 1,
    root: fakeComment({ id: `m-${i}`, body: 'x'.repeat(350), pin: { x: 0.5, y: 0.5 } }),
    replies: [],
  }));
  const capped = formatPinsMarkdown({ artifactId: 'art-big', artifactTitle: 'Big', threads: many });
  assert.ok(capped.length <= MAX_PINS_MARKDOWN_CHARS + 128);
  assert.match(capped, /truncated to fit/);
});

test('pickTargetAgentSession prefers active live agent and ignores connection/archived/exited sessions', () => {
  const sessions = [
    { id: 'sh-1', title: 'SSH Prod', kind: 'connection' as const, archived: false, status: 'running' as const },
    { id: 'ag-old', title: 'Archived', kind: 'agent' as const, archived: true, status: 'running' as const },
    { id: 'ag-dead', title: 'Exited', kind: 'agent' as const, archived: false, status: 'exited' as const },
    { id: 'ag-1', title: 'Claude 1', kind: 'agent' as const, archived: false, status: 'running' as const },
    { id: 'ag-2', title: 'Claude 2', kind: 'agent' as const, archived: false, status: 'running' as const },
  ];

  assert.equal(pickTargetAgentSession(sessions, 'ag-2')?.id, 'ag-2');
  assert.equal(pickTargetAgentSession(sessions, 'sh-1')?.id, 'ag-1');
  assert.equal(pickTargetAgentSession(sessions.slice(0, 3), 'sh-1'), null);
});

test('ottoRefs parses story/design URIs, renders safe clickable pills, and resolves vault deep links', async () => {
  const s = parseOttoRef('otto://story/story-42');
  assert.equal(s?.kind, 'story');
  assert.equal(s?.route, 'product/story-42');

  const d = parseOttoRef('otto://design/art-7@approved#hero');
  assert.equal(d?.kind, 'design');
  assert.equal(d?.selector, 'approved');
  assert.equal(d?.fragment, 'hero');
  assert.equal(d?.route, 'design/a/art-7');

  assert.equal(parseOttoRef('otto://story/../secret'), null);
  assert.equal(parseOttoRef('javascript:alert(1)'), null);

  const marked = new Marked();
  marked.use(ottoRefMarkedExtension());
  const html = marked.parse(
    'See [PAY-128](otto://story/story-42) and bare otto://design/art-7@approved.',
    { async: false },
  ) as string;
  assert.ok(html.includes('class="otto-ref otto-ref-story"'));
  assert.ok(html.includes('href="#/product/story-42"'));
  assert.ok(html.includes('data-raw="otto://story/story-42"'));
  assert.ok(html.includes('PAY-128'));
  assert.ok(html.includes('class="otto-ref otto-ref-design"'));
  assert.ok(html.includes('href="#/design/a/art-7"'));
  assert.ok(html.includes('art-7@approved'));

  assert.deepEqual(parseVaultDeepLink(['vault', '3', 'rfcs%2Fpay-128.md']), {
    vaultId: 3,
    path: 'rfcs/pay-128.md',
  });
  assert.equal(parseVaultDeepLink(['vault', '3', '..%2Fsecret.md']), null);
  assert.equal(parseVaultDeepLink(['vault', 'bad', 'a.md']), null);

  const calls: string[] = [];
  const consumed = await consumeVaultDeepLink(
    ['vault', '3', 'rfcs%2Fpay-128.md'],
    {
      vaults: [{ id: 1 }, { id: 3 }],
      current: { id: 1 },
      select: async (id) => { calls.push(`select:${id}`); },
      open: async (path) => { calls.push(`open:${path}`); },
    },
    { replace: (r) => calls.push(`replace:${r}`) },
  );
  assert.equal(consumed, true);
  assert.deepEqual(calls, ['replace:vault', 'select:3', 'open:rfcs/pay-128.md']);
});
