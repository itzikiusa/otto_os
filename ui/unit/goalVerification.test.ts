import { test } from 'node:test';
import assert from 'node:assert/strict';
import { humanVerification } from '../src/modules/loops/verification.ts';
import type { AcceptanceCriterion, GoalLoopLedger } from '../src/lib/api/types.ts';
const criterion: AcceptanceCriterion = { id: 'h', text: 'Inspect output', verify: 'Open report', verify_kind: 'human' };
const ledger: GoalLoopLedger = { verifications: [{ criterion_id: 'h', criterion_revision: JSON.stringify({ ...criterion, verify_cmd: null }), verified_by: 'reviewer', evidence: 'Output verified', verified_at: '' }], questions: [], next_action: '', repeated_failures: 0, last_failure_signature: '', review_summary: '', review_passed: false };
test('human verification preserves identity and evidence after reload', () => {
  assert.equal(humanVerification(criterion, JSON.parse(JSON.stringify(ledger)))?.verified_by, 'reviewer');
});
test('changed criterion and malformed revisions do not display accepted evidence', () => {
  assert.equal(humanVerification({ ...criterion, verify: 'Inspect a different report' }, ledger), undefined);
  assert.equal(humanVerification(criterion, { ...ledger, verifications: [{ ...ledger.verifications[0], criterion_revision: 'broken' }] }), undefined);
});
