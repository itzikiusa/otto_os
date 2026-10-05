import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runEvidenceLinks } from '../src/modules/run-with-otto/runEvidence.ts';

const base = { findings_total: 0 } as const;

test('a run with every artifact links to proof, findings, loop and branch (S20-02)', () => {
  const links = runEvidenceLinks({
    ...base,
    proof_pack_id: 'pp 1',
    review_id: 'rv1',
    goal_loop_id: 'gl1',
    repo_id: 'r1',
    branch: 'otto/run-1',
    findings_total: 3,
  });
  assert.deepEqual(
    links.map((l) => [l.key, l.route]),
    [
      ['proof', 'proof/pp%201'],
      ['findings', 'git/r1/review'],
      ['loop', 'loops/gl1'],
      ['branch', 'git/r1/graph'],
    ],
  );
});

test('a run with nothing to inspect yields no links', () => {
  assert.deepEqual(runEvidenceLinks({ ...base }), []);
});

test('findings and branch need a repo to resolve', () => {
  const links = runEvidenceLinks({ ...base, review_id: 'rv1', branch: 'b', findings_total: 2 });
  assert.deepEqual(links, []);
});
