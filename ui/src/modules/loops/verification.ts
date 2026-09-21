import type { AcceptanceCriterion, GoalLoopLedger } from '../../lib/api/types.ts';

/** A changed criterion cannot display an earlier human verification as current. */
export function humanVerification(criterion: AcceptanceCriterion, ledger?: GoalLoopLedger) {
  return ledger?.verifications.find((v) => {
    if (v.criterion_id !== criterion.id || !v.evidence.trim()) return false;
    try {
      const prior = JSON.parse(v.criterion_revision) as AcceptanceCriterion;
      return prior.id === criterion.id && prior.text === criterion.text &&
        prior.verify === criterion.verify && prior.verify_kind === criterion.verify_kind &&
        (prior.verify_cmd ?? null) === (criterion.verify_cmd ?? null);
    } catch { return false; }
  });
}
