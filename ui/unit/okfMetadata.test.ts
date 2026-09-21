import { test } from 'node:test';
import assert from 'node:assert/strict';
import { okfMetadata } from '../src/modules/vault/okfMetadata.ts';
import { parse } from 'yaml';
import { okfConceptTemplate } from '../src/modules/vault/okfTemplate.ts';
import { patchProperties } from '../src/modules/vault/properties.ts';

test('OKF verifier mapping and list derive the same advisory trust tier', () => {
  const human = { by: 'human:alice', at: '2026-09-19T12:00:00Z' };
  assert.equal(okfMetadata({ verified: human }).tier, 'Declared human review');
  assert.equal(okfMetadata({ verified: [human, { by: 'process:test', at: '2026-09-20T12:00:00Z' }] }).tier, 'Declared human review');
  assert.equal(okfMetadata({ verified: { by: 'process:test', at: '2026-09-20T12:00:00Z' } }).tier, 'Declared machine verification');
  assert.equal(okfMetadata({ verified: { by: 'human:alice', at: 'yesterday' } }).tier, 'Unverified');
  assert.equal(okfMetadata({ verified: true }).tier, 'Unverified');
  assert.equal(okfMetadata({ verified: { by: 'human:alice', at: '2026-02-30T12:00:00Z' } }).tier, 'Unverified');
  assert.equal(okfMetadata({ stale_after: '2026-09-20T24:00:00Z' }).invalidDeadline, true);
});
test('staleness uses an absolute inclusive deadline and verification is independent', () => {
  const value = { stale_after: '2026-09-20T12:00:00Z', generated: { by: 'process:test', at: '2026-09-20T12:00:00Z' }, verified: { by: 'human:a', at: '2026-09-19T12:00:00Z' } };
  const now = Date.parse(value.stale_after);
  assert.equal(okfMetadata(value, now - 1).stale, false);
  assert.equal(okfMetadata(value, now).stale, true);
  assert.equal(okfMetadata(value, now).changedSinceVerification, true);
  assert.equal(okfMetadata({ stale_after: '3 days' }).invalidDeadline, true);
});
test('legacy and absent metadata remain readable, unknown types are accepted', () => {
  assert.equal(okfMetadata({ timestamp: '2026-09-20T12:00:00Z' }).generatedAt, '2026-09-20T12:00:00Z');
  assert.equal(okfMetadata(null).status, 'stable');
  for (const generated of [null, true, {}, 'malformed']) {
    assert.equal(okfMetadata({ generated, timestamp: '2026-09-20T12:00:00Z' }).generatedAt, '');
  }
  assert.equal(okfMetadata({ type: 'Future', sources: [{ resource: 'all queries in project X' }, 4] }).sources.length, 1);
  assert.equal(okfMetadata({ type: 'Attested Computation' }).attestedComputation, true);
});
test('editing lifecycle fields preserves nested provenance and body', () => {
  const raw = '---\ntype: Service\ngenerated: {by: agent/v1, at: 2026-09-20T12:00:00Z}\nsources: [{resource: /reference.md}]\nverified: {by: human:a, at: 2026-09-20T13:00:00Z}\n---\nExact body\n';
  const updated = patchProperties(raw, { status: 'deprecated', stale_after: '2026-09-21T00:00:00Z' });
  assert.match(updated, /generated:/); assert.match(updated, /sources:/); assert.match(updated, /verified:/);
  assert.match(updated, /status: deprecated/); assert.ok(updated.endsWith('Exact body\n'));
});

test('new concept template is v0.2, draft and unverified, with YAML-safe titles', () => {
  const raw = okfConceptTemplate('Reference', 'Title: one\nstatus: stable', 'human:a', new Date('2026-09-20T00:00:00Z'));
  const fm = parse(raw.split('---')[1]);
  assert.equal(fm.title, 'Title: one\nstatus: stable'); assert.equal(fm.status, 'draft');
  assert.equal(fm.generated.by, 'human:a'); assert.equal(fm.verified, undefined);
  assert.deepEqual(fm.sources, []); assert.equal(fm.timestamp, undefined);
});
