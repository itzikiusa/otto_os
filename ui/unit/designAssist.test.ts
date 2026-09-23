import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  applyAssistEvent,
  buildThread,
  candidateProgress,
  directionName,
  findingsOf,
  fixRequest,
  isTerminal,
  mergeTurns,
  parseBranch,
  promptRequest,
  provenanceChips,
  quickActionRequest,
  rejectSignal,
  salientTerms,
  statusInfo,
  variantCards,
  type AssistEventLike,
} from '../src/modules/design-hall/assist/model.ts';

// Wire-shape fixtures (docs/contracts/api.md § Design assist).
function turn(id: string, over: Record<string, unknown> = {}): any {
  return {
    turn_id: id,
    artifact_id: 'a1',
    workspace_id: 'w',
    mode: 'refine',
    status: 'running',
    branch: 'main',
    direction: null,
    provider: 'claude',
    session_id: null,
    base_version_id: 'v1',
    version_id: null,
    references: [],
    cited: [],
    unverified_citations: [],
    team_rules: [],
    findings: [],
    message: null,
    error: null,
    started_at: '2026-09-23T10:00:00Z',
    finished_at: null,
    ...over,
  };
}
function ev(turnId: string, status: string, over: Record<string, unknown> = {}): AssistEventLike {
  return {
    artifact_id: 'a1',
    workspace_id: 'w',
    turn_id: turnId,
    status: status as AssistEventLike['status'],
    mode: 'refine',
    branch: 'main',
    session_id: null,
    version_id: null,
    error: null,
    ...over,
  };
}
function version(id: string, seq: number, branch = 'main', over: Record<string, unknown> = {}): any {
  return {
    id,
    artifact_id: 'a1',
    seq,
    parent_version_id: null,
    branch,
    blob_sha256: 'x',
    size_bytes: 1,
    kind: 'agent',
    author_kind: 'agent',
    author_id: 'agent',
    session_id: null,
    message: '',
    provenance: {},
    created_at: '2026-09-23T10:05:00Z',
    ...over,
  };
}

// ── status reducer ─────────────────────────────────────────────────────────

test('status words: working states pulse, terminal states say what happened', () => {
  assert.equal(statusInfo('running').working, true);
  assert.equal(statusInfo('starting').label, 'Starting…');
  assert.equal(statusInfo('done').tone, 'ok');
  assert.equal(statusInfo('unchanged', 'critique').label, 'Review ready');
  assert.equal(statusInfo('conflict').label, 'Kept as a draft');
  assert.equal(statusInfo('failed').tone, 'bad');
  assert.equal(isTerminal('done'), true);
  assert.equal(isTerminal('running'), false);
});

test('an unknown turn from an event becomes a placeholder at the front', () => {
  const r = applyAssistEvent([turn('t0', { status: 'done' })], ev('t1', 'starting'), '2026-09-23T11:00:00Z');
  assert.equal(r.turns.length, 2);
  assert.equal(r.turns[0].turn_id, 't1');
  assert.equal(r.turns[0].status, 'starting');
  assert.equal(r.turns[0].started_at, '2026-09-23T11:00:00Z');
  assert.equal(r.refetch, false);
});

test('running → done merges ids, keeps a known session and asks for the full turn', () => {
  const start = [turn('t1', { status: 'running', session_id: 's1' })];
  const r = applyAssistEvent(start, ev('t1', 'done', { version_id: 'v2' }), 'now');
  assert.equal(r.turns[0].status, 'done');
  assert.equal(r.turns[0].session_id, 's1', 'a null id never erases a known one');
  assert.equal(r.turns[0].version_id, 'v2');
  assert.equal(r.turns[0].finished_at, 'now');
  assert.equal(r.refetch, true);
  // A second terminal event for the same turn doesn't refetch again.
  assert.equal(applyAssistEvent(r.turns, ev('t1', 'done', { version_id: 'v2' }), 'later').refetch, false);
});

test('a late "running" never regresses a finished turn', () => {
  const done = [turn('t1', { status: 'failed', error: 'invalid html' })];
  const r = applyAssistEvent(done, ev('t1', 'running', { session_id: 's9' }), 'now');
  assert.equal(r.turns, done);
  assert.equal(r.turns[0].status, 'failed');
});

test('conflict carries the side version; failed carries the error', () => {
  const r1 = applyAssistEvent([turn('t1')], ev('t1', 'conflict', { version_id: 'side', branch: 'variant/t1/1' }), 'now');
  assert.equal(r1.turns[0].version_id, 'side');
  assert.equal(r1.turns[0].branch, 'variant/t1/1');
  const r2 = applyAssistEvent([turn('t2')], ev('t2', 'failed', { error: 'boom' }), 'now');
  assert.equal(r2.turns[0].error, 'boom');
});

test('mergeTurns: the server copy wins, local-only and locally finished turns survive', () => {
  const local = [turn('a', { status: 'done', started_at: '2026-09-23T10:02:00Z' }), turn('b', { started_at: '2026-09-23T10:01:00Z' })];
  const fetched = [turn('a', { status: 'running', started_at: '2026-09-23T10:02:00Z' }), turn('c', { status: 'done', message: 'hi', started_at: '2026-09-23T10:03:00Z' })];
  const m = mergeTurns(local, fetched);
  assert.deepEqual(m.map((t) => t.turn_id), ['c', 'a', 'b']);
  assert.equal(m.find((t) => t.turn_id === 'a')!.status, 'done');
  assert.equal(m.find((t) => t.turn_id === 'c')!.message, 'hi');
});

// ── provenance chips ──────────────────────────────────────────────────────

test('verified citations open the reference; unverified ones are flagged and inert', () => {
  const t = turn('t1', {
    status: 'done',
    references: [
      { label: 'R1', artifact_id: 'ref1', version_id: 'rv1', seq: 12, title: 'Spring promo site', studio: 'site', format: 'html', status: 'shipped', source: 'search' },
      { label: 'R2', artifact_id: 'ref2', version_id: 'rv2', seq: 9, title: 'Tier ladder', studio: 'frames', format: 'html', status: 'approved', source: 'link' },
    ],
    cited: [
      { label: 'R1', artifact_id: 'ref1', version_id: 'rv1', seq: 12 },
      { label: 'R1', artifact_id: 'ref1', version_id: 'rv1', seq: 12 },
    ],
    unverified_citations: ['R7', ' '],
  });
  const chips = provenanceChips(t, { brand_kit: { artifact_id: 'bk', version_id: 'bkv', seq: 4 } });
  assert.deepEqual(chips.map((c) => [c.kind, c.label, c.verified, c.artifactId]), [
    ['ref', 'R1 Spring promo site', true, 'ref1'],
    ['brand', 'Brand Kit v4', true, 'bk'],
    ['unverified', 'R7', false, null],
  ]);
  assert.match(chips[0].title, /v12/);
  assert.match(chips[2].title, /not recorded/);
});

test('no provenance → only what the turn itself cites', () => {
  assert.deepEqual(provenanceChips(turn('t1')), []);
  const c = provenanceChips(turn('t1', { cited: [{ label: 'R3', artifact_id: 'x', version_id: null, seq: null }] }));
  assert.equal(c[0].label, 'R3 Reference');
});

// ── quick actions → requests ──────────────────────────────────────────────

test('quick actions map to modes; checks never edit, fixes do', () => {
  const sel = { node_id: 'hero', label: 'Hero' };
  const a11y = quickActionRequest('a11y_check', { selection: sel, provider: 'codex' });
  assert.equal(a11y.kind, 'assist');
  if (a11y.kind !== 'assist') return;
  assert.equal(a11y.body.mode, 'critique');
  assert.deepEqual(a11y.body.selection, { node_id: 'hero', label: 'Hero' });
  assert.equal(a11y.body.provider, 'codex');
  assert.match(a11y.prompt, /Do not edit/);
  assert.match(a11y.prompt, /“Hero”/);
  assert.equal(a11y.display, 'Check accessibility', 'the thread shows the label, not the recipe');

  const brand = quickActionRequest('brand_check');
  assert.equal(brand.kind === 'assist' && brand.body.mode, 'critique');
  for (const id of ['engaging', 'mobile', 'copy'] as const) {
    const r = quickActionRequest(id);
    assert.equal(r.kind === 'assist' && r.body.mode, 'refine', id);
    assert.equal(r.kind === 'assist' && 'selection' in r.body, false, 'no selection → none sent');
  }
});

test('"3 variants" is a variants run of 3 focused on the selection', () => {
  const r = quickActionRequest('variants', { selection: { node_id: 'hero', label: 'Hero' }, text: 'bolder', references: ['a@v2', ''] });
  assert.equal(r.kind, 'variants');
  if (r.kind !== 'variants') return;
  assert.equal(r.body.n, 3);
  assert.match(r.prompt, /Three distinct directions for “Hero”\. bolder/);
  assert.equal(r.display, '3 variants — bolder');
  assert.deepEqual(r.body.references, ['a@v2']);
  assert.deepEqual(r.body.selection, { node_id: 'hero', label: 'Hero' });
});

test('composer text → refine; n > 1 → variants (clamped to 4); generate intent kept', () => {
  const r = promptRequest('  tighten the subhead ', {});
  assert.equal(r.kind === 'assist' && r.body.mode, 'refine');
  assert.equal(r.prompt, 'tighten the subhead');
  const v = promptRequest('a launch page', { references: ['r1', 'r2'] }, 9, 'generate');
  assert.equal(v.kind, 'variants');
  assert.equal(v.kind === 'variants' && v.body.n, 4);
  assert.equal(v.intent, 'generate');
  const g = promptRequest('a launch page', {}, 1, 'generate');
  assert.equal(g.kind === 'assist' && g.body.mode, 'generate');
  const many = promptRequest('x', { references: Array.from({ length: 12 }, (_, i) => `r${i}`) });
  assert.equal(many.kind === 'assist' && many.body.references?.length, 8, 'at most 8 references');
});

test('Fix all: an accessibility review becomes an a11y turn quoting its findings', () => {
  const t = turn('t1', {
    mode: 'critique',
    findings: [
      { severity: 'error', rule: 'color-contrast', message: 'CTA text is 1.8:1', fix: 'use ink text' },
      { message: '' },
      { rule: 'image-alt', message: 'Hero embed lacks alt text' },
    ],
  });
  const fs = findingsOf(t);
  assert.equal(fs.length, 2);
  assert.deepEqual(fs.map((f) => f.index), [0, 2]);
  const r = fixRequest(fs, true);
  assert.equal(r.intent, 'fix_a11y');
  assert.equal(r.kind === 'assist' && r.body.mode, 'a11y');
  assert.match(r.prompt, /\[color-contrast\] CTA text is 1\.8:1 → use ink text/);
  const other = fixRequest(fs, false);
  assert.equal(other.kind === 'assist' && other.body.mode, 'refine');
});

// ── variants ──────────────────────────────────────────────────────────────

test('variant branches parse; direction names read well', () => {
  assert.deepEqual(parseBranch('variant/run1/2'), { run: 'run1', k: 2 });
  assert.equal(parseBranch('main'), null);
  assert.equal(directionName('defaults'), 'Team defaults');
  assert.equal(directionName('bold'), 'Bold');
  assert.equal(directionName(null), 'Variant');
});

test('variant cards: running, ready, failed, then accepted vs passed', () => {
  const run: any = {
    run_id: 'r1',
    artifact_id: 'a1',
    base_version_id: 'v1',
    status: 'running',
    versions: [version('vA', 5, 'variant/r1/1', { message: 'Bold tilt' })],
    accepted_version_id: null,
    turns: [
      turn('t1', { mode: 'variant', branch: 'variant/r1/1', direction: 'defaults', status: 'done', message: 'Big headline' }),
      turn('t2', { mode: 'variant', branch: 'variant/r1/2', direction: 'explore', status: 'running' }),
      turn('t3', { mode: 'variant', branch: 'variant/r1/3', direction: 'calm', status: 'failed', error: 'x' }),
    ],
  };
  const c = variantCards(run);
  assert.deepEqual(c.map((x) => [x.k, x.direction, x.state]), [
    [1, 'defaults', 'ready'],
    [2, 'explore', 'running'],
    [3, 'calm', 'failed'],
  ]);
  assert.equal(c[0].summary, 'Big headline');
  const accepted = variantCards({ ...run, status: 'accepted', accepted_version_id: 'vA', versions: [...run.versions, version('vB', 6, 'variant/r1/2')] });
  assert.deepEqual(accepted.slice(0, 2).map((x) => x.state), ['accepted', 'passed']);
});

test('after a daemon restart (no live turns) cards come from versions + provenance', () => {
  const run: any = {
    run_id: 'r1', artifact_id: 'a1', base_version_id: 'v1', status: 'ready', accepted_version_id: null, turns: [],
    versions: [version('vB', 6, 'variant/r1/2', { provenance: { assist: { direction: 'story' } } })],
  };
  const c = variantCards(run);
  assert.deepEqual(c.map((x) => [x.k, x.direction, x.state]), [[2, 'story', 'ready']]);
});

test('a 👎 posts variant_rejected with the reason chip and direction', () => {
  const card: any = { k: 2, branch: 'variant/r1/2', direction: 'calm', version: version('vB', 6, 'variant/r1/2'), turn: null, state: 'ready', summary: null };
  const s = rejectSignal({ artifactId: 'a1', card, runId: 'r1', reason: 'too_busy' });
  assert.equal(s.kind, 'variant_rejected');
  assert.equal(s.version_id, 'vB');
  assert.deepEqual(s.payload, { source: 'variant_tray', reason: 'too_busy', run_id: 'r1', k: 2, direction: 'calm', rejected_version_id: 'vB', rejected_seq: 6 });
});

// ── thread ────────────────────────────────────────────────────────────────

test('thread: main turns and runs interleave by time; variant turns fold into their run', () => {
  const turns = [
    turn('t2', { started_at: '2026-09-23T10:10:00Z', status: 'done' }),
    turn('tv', { mode: 'variant', branch: 'variant/r1/1', started_at: '2026-09-23T10:05:00Z' }),
    turn('t1', { started_at: '2026-09-23T10:00:00Z', status: 'done' }),
    // A conflict's side draft is listed by GET …/variants as a run named after the turn.
    turn('tc', { started_at: '2026-09-23T10:20:00Z', status: 'conflict', branch: 'variant/tc/1', version_id: 'side' }),
  ];
  const runs: any[] = [
    { run_id: 'r1', artifact_id: 'a1', base_version_id: 'v1', status: 'ready', accepted_version_id: null, versions: [], turns: [turns[1]] },
    { run_id: 'tc', artifact_id: 'a1', base_version_id: 'v1', status: 'ready', accepted_version_id: null, versions: [version('side', 9, 'variant/tc/1')], turns: [] },
  ];
  const th = buildThread(turns, runs);
  assert.deepEqual(
    th.map((i) => (i.kind === 'turn' ? `t:${i.turn.turn_id}` : `r:${i.run.run_id}`)),
    ['t:t1', 'r:r1', 't:t2', 't:tc'],
  );
});

// ── learning ──────────────────────────────────────────────────────────────

test('candidate progress counts toward ≥ 3 signals across ≥ 2 designs', () => {
  const c: any = { key: 'k', kind: 'reject_reason', rule: 'r', rationale: '', signal_ids: [], signal_count: 2, artifact_count: 1, ready: false };
  const p = candidateProgress(c);
  assert.equal(p.text, '2 of 3 signals · 1 of 2 designs');
  assert.equal(p.pct, 58);
  assert.equal(candidateProgress({ ...c, signal_count: 5, artifact_count: 3, ready: true }).pct, 100);
});

test('salient terms: no stop words, longest first, unique', () => {
  assert.deepEqual(salientTerms('A launch page for Rewards+ with a 3D card hero, a launch!'), ['rewards', 'launch', 'card']);
  assert.deepEqual(salientTerms('the a of'), []);
  assert.deepEqual(salientTerms('checkout flow diagram', 2), ['checkout', 'diagram']);
});
