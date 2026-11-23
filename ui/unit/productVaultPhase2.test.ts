import test from 'node:test';
import assert from 'node:assert/strict';
import { Marked } from 'marked';
import {
  activePredicates,
  clearPredicates,
  hasPredicate,
  insertOperatorPrefix,
  togglePredicate,
} from '../src/modules/vault/searchPredicates.ts';
import {
  buildStoryBodyFromNote,
  extractNoteTitle,
  vaultRefUri,
} from '../src/modules/vault/noteToStory.ts';
import {
  ottoRefMarkedExtension,
  parseOttoRef,
  parseTransclusionTarget,
} from '../src/modules/vault/ottoRefs.ts';

test('searchPredicates toggles audit filters, swaps exclusive status tokens, and clears cleanly', () => {
  assert.equal(hasPredicate('retry stale:true', 'stale:true'), true);
  assert.equal(hasPredicate('STALE:TRUE', 'stale:true'), true);
  assert.equal(hasPredicate('stale:false', 'stale:true'), false);

  let q = togglePredicate('checkout', 'stale:true');
  assert.equal(q, 'checkout stale:true');

  // Mutually exclusive status group swaps draft -> stable instead of stacking both.
  q = togglePredicate(q, 'status:draft', 'status:');
  assert.equal(q, 'checkout stale:true status:draft');
  q = togglePredicate(q, 'status:stable', 'status:');
  assert.equal(q, 'checkout stale:true status:stable');

  // Toggling an active predicate off removes only that token.
  q = togglePredicate(q, 'stale:true');
  assert.equal(q, 'checkout status:stable');

  // Operator prefix insertion adds clean spacing and avoids duplicating trailing prefix.
  assert.equal(insertOperatorPrefix('', 'links-to:'), 'links-to:');
  assert.equal(insertOperatorPrefix('auth', 'tag:'), 'auth tag:');
  assert.equal(insertOperatorPrefix('auth tag:', 'tag:'), 'auth tag:');

  // Active detection and clear-all preserve free-text terms.
  const mixed = 'payment status:stable orphan:true links-to:checkout/index type:rfc';
  assert.deepEqual(activePredicates(mixed), [
    'status:stable',
    'orphan:true',
    'links-to:checkout/index',
    'type:rfc',
  ]);
  assert.equal(clearPredicates(mixed), 'payment');
});

test('noteToStory extracts title, builds otto://vault back-link, and strips frontmatter', () => {
  const raw = '---\ntype: rfc\nstatus: draft\n---\n\n# Payment Retries\n\nUse idempotency keys.';
  assert.equal(extractNoteTitle('rfcs/pay.md', raw, 'Explicit Meta Title'), 'Explicit Meta Title');
  assert.equal(extractNoteTitle('rfcs/pay.md', raw, ''), 'Payment Retries');
  assert.equal(extractNoteTitle('rfcs/no-heading.md', 'Just body text.', null), 'no-heading');

  assert.equal(vaultRefUri(5, '/rfcs/pay.md'), 'otto://vault/5/rfcs/pay.md');

  const full = buildStoryBodyFromNote({ vaultId: 5, notePath: 'rfcs/pay.md', raw });
  assert.ok(full.includes('[rfcs/pay.md](otto://vault/5/rfcs/pay.md)'));
  assert.ok(full.includes('# Payment Retries'));
  assert.ok(full.includes('Use idempotency keys.'));
  assert.ok(!full.includes('type: rfc'));

  const linkOnly = buildStoryBodyFromNote({ vaultId: 5, notePath: 'rfcs/pay.md', raw, includeBody: false });
  assert.ok(linkOnly.includes('otto://vault/5/rfcs/pay.md'));
  assert.ok(!linkOnly.includes('Use idempotency keys.'));
});

test('ottoRefs supports otto://canvas pills and design/canvas transclusion target parsing', () => {
  const c = parseOttoRef('otto://canvas/scene-99');
  assert.equal(c?.kind, 'canvas');
  assert.equal(c?.id, 'scene-99');
  assert.equal(c?.route, 'canvas');
  assert.equal(parseOttoRef('otto://canvas/../bad'), null);

  const t1 = parseTransclusionTarget('otto://design/art-7@approved', 'otto://design/art-7@approved#hero|Hero Frame');
  assert.equal(t1?.ref.kind, 'design');
  assert.equal(t1?.ref.id, 'art-7');
  assert.equal(t1?.ref.selector, 'approved');
  assert.equal(t1?.ref.fragment, 'hero');
  assert.equal(t1?.alias, 'Hero Frame');

  const t2 = parseTransclusionTarget('otto://canvas/scene-99', 'otto://canvas/scene-99|System Flow');
  assert.equal(t2?.ref.kind, 'canvas');
  assert.equal(t2?.ref.id, 'scene-99');
  assert.equal(t2?.alias, 'System Flow');

  const marked = new Marked();
  marked.use(ottoRefMarkedExtension());
  const html = marked.parse('Diagram: [Auth Flow](otto://canvas/scene-99) and otto://canvas/scene-100', { async: false }) as string;
  assert.ok(html.includes('class="otto-ref otto-ref-canvas"'));
  assert.ok(html.includes('data-raw="otto://canvas/scene-99"'));
  assert.ok(html.includes('Auth Flow'));
  assert.ok(html.includes('data-raw="otto://canvas/scene-100"'));
});
