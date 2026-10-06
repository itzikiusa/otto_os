import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runEvidenceLinks } from '../src/modules/run-with-otto/runEvidence.ts';
import { routeGraphRef, routeReviewId } from '../src/modules/git/deepLink.ts';

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
      ['findings', 'git/r1/review/rv1'],
      ['loop', 'loops/gl1'],
      ['branch', 'git/r1/graph/otto%2Frun-1'],
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

test('the evidence routes resolve to THIS run’s review and branch (S20-301)', () => {
  const links = runEvidenceLinks({ ...base, repo_id: 'r 1', review_id: 'rv/1', branch: 'otto/run-x', findings_total: 2 });
  // The router splits on "/" and decodes each segment — mirror that.
  const parts = (route: string) => route.split('/').map(decodeURIComponent);
  const findings = parts(links.find((l) => l.key === 'findings')!.route);
  const branch = parts(links.find((l) => l.key === 'branch')!.route);
  assert.equal(routeReviewId(findings, 'r 1'), 'rv/1');
  assert.equal(routeGraphRef(branch, 'r 1'), 'otto/run-x');
  // Another repo's tab, or the bare tab, targets nothing.
  assert.equal(routeReviewId(findings, 'r2'), null);
  assert.equal(routeReviewId(['git', 'r 1', 'review'], 'r 1'), null);
  assert.equal(routeGraphRef(['git', 'r 1', 'graph'], 'r 1'), null);
  // An unencoded slash still resolves to the whole branch name.
  assert.equal(routeGraphRef(['git', 'r 1', 'graph', 'feat', 'x'], 'r 1'), 'feat/x');
});

test('without a review id there is no findings link (nothing specific to open)', () => {
  const links = runEvidenceLinks({ ...base, repo_id: 'r1', findings_total: 2 });
  assert.equal(links.some((l) => l.key === 'findings'), false);
});
