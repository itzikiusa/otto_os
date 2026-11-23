// Pure-TS unit tests for Git Phase 2 Deep Push deliverables:
//   - E-08: diffAiOverlay (line matching, deduplication, counts, file filtering, severity tones)
//   - E-09: pushPreflight (summary formatting, 404/405/501 graceful skip, ok pass-through, confirm block/bypass)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { Finding, ReviewComment } from '../src/lib/api/types.ts';
import {
  aiOverlayCountForFile,
  aiOverlayForFile,
  aiOverlayForLine,
  aiOverlaySeverityLabel,
  aiOverlaySeverityTone,
  buildAiOverlayIndex,
  normalizeOverlayPath,
  overlayLineKey,
} from '../src/modules/git/diffAiOverlay.ts';
import {
  formatPreflightBytes,
  formatPreflightSummary,
  hasPreflightIssues,
  isPreflightUnsupported,
  runPushPreflight,
  type PushPreflightConfirmOpts,
  type PushPreflightReq,
  type PushPreflightResp,
} from '../src/modules/git/pushPreflight.ts';

function makeComment(overrides: Partial<ReviewComment> = {}): ReviewComment {
  return {
    id: 'c1',
    review_id: 'rev1',
    path: 'src/auth.ts',
    line: 42,
    severity: 'bug',
    body: 'Token expiry check is inverted.',
    state: 'draft',
    posted: false,
    created_at: '2026-11-19T12:00:00Z',
    ...overrides,
  };
}

function makeFinding(overrides: Partial<Finding> = {}): Finding {
  return {
    id: 'f1',
    review_id: 'rev1',
    workspace_id: 'ws1',
    repo_id: 'repo1',
    pr_number: 12,
    fingerprint: 'fp1',
    severity: 'high',
    category: 'security',
    path: 'src/auth.ts',
    line: 42,
    line_end: 45,
    title: 'Missing signature check',
    body: 'Verify webhook HMAC before parsing JSON.',
    evidence: 'line 42',
    agent_reasoning_summary: 'Checked handler',
    suggested_fix: null,
    status: 'open',
    linked_commit: null,
    linked_test: null,
    reviewer: 'security-agent',
    state: 'open',
    regressed: false,
    requires_human_approval: false,
    approval_decision: null,
    approved_by: null,
    approved_at: null,
    jira_key: null,
    jira_url: null,
    produced_by_agent: 'security-agent',
    repo_rule_id: null,
    fix_session_id: null,
    occurrence_count: 1,
    created_at: '2026-11-19T12:00:00Z',
    updated_at: '2026-11-19T12:00:00Z',
    ...overrides,
  };
}

// ── E-08: diffAiOverlay ──────────────────────────────────────────────────────

test('normalizeOverlayPath strips leading ./, /, and normalizes backslashes', () => {
  assert.equal(normalizeOverlayPath('./src/a.ts'), 'src/a.ts');
  assert.equal(normalizeOverlayPath('/src/a.ts'), 'src/a.ts');
  assert.equal(normalizeOverlayPath('src\\nested\\b.ts'), 'src/nested/b.ts');
  assert.equal(normalizeOverlayPath(null), '');
  assert.equal(overlayLineKey('./src/a.ts', 10), 'src/a.ts\u000010');
});

test('buildAiOverlayIndex filters inactive comments/findings and computes total + per-file counts', () => {
  const comments: ReviewComment[] = [
    makeComment({ id: 'c-draft', path: './src/a.ts', line: 10, severity: 'warn', state: 'draft', posted: false }),
    makeComment({ id: 'c-retry', path: 'src/a.ts', line: 20, severity: 'bug', state: 'approved', posted: false }),
    makeComment({ id: 'c-posted', path: 'src/a.ts', line: 30, state: 'approved', posted: true }),
    makeComment({ id: 'c-declined', path: 'src/a.ts', line: 40, state: 'declined', posted: false }),
    makeComment({ id: 'c-general', path: null, line: null, state: 'draft', posted: false }),
  ];
  const findings: Finding[] = [
    makeFinding({ id: 'f-open', path: 'src/b.ts', line: 5, severity: 'critical', status: 'open' }),
    makeFinding({ id: 'f-accepted', path: 'src/b.ts', line: null, severity: 'low', status: 'accepted' }),
    makeFinding({ id: 'f-fp', path: 'src/b.ts', line: 9, status: 'false_positive' }),
    makeFinding({ id: 'f-waived', path: 'src/b.ts', line: 11, status: 'waived' }),
    makeFinding({ id: 'f-fixed', path: 'src/b.ts', line: 12, status: 'fixed' }),
    makeFinding({ id: 'f-verified', path: 'src/b.ts', line: 13, status: 'verified' }),
  ];

  const idx = buildAiOverlayIndex(comments, findings);
  assert.equal(idx.total, 4);
  assert.equal(aiOverlayCountForFile(idx, 'src/a.ts'), 2);
  assert.equal(aiOverlayCountForFile(idx, 'src/b.ts'), 2);
  assert.equal(aiOverlayCountForFile(idx, 'src/other.ts'), 0);

  assert.deepEqual(
    aiOverlayForLine(idx, 'src/a.ts', 10).map((x) => x.id),
    ['c-draft'],
  );
  assert.deepEqual(
    aiOverlayForLine(idx, 'src/a.ts', 20).map((x) => x.id),
    ['c-retry'],
  );
  assert.deepEqual(aiOverlayForLine(idx, 'src/a.ts', 30), []);
  assert.deepEqual(
    aiOverlayForLine(idx, 'src/b.ts', null, 5).map((x) => x.id),
    ['f-open'],
    'falls back to oldLine when newLine is null',
  );
  assert.deepEqual(
    aiOverlayForFile(idx, 'src/b.ts').map((x) => x.id),
    ['f-accepted'],
    'exposes file-level findings with line === null',
  );
});

test('buildAiOverlayIndex deduplicates a finding matching a draft comment on the same line and sorts by severity', () => {
  const comments: ReviewComment[] = [
    makeComment({
      id: 'c-dup',
      path: 'src/auth.ts',
      line: 42,
      severity: 'warn',
      body: 'Missing signature check: verify HMAC header.',
    }),
    makeComment({
      id: 'c-info',
      path: 'src/auth.ts',
      line: 42,
      severity: 'info',
      body: 'Consider extracting a helper.',
    }),
  ];
  const findings: Finding[] = [
    // Matches c-dup by title -> should be deduplicated
    makeFinding({
      id: 'f-dup',
      path: 'src/auth.ts',
      line: 42,
      severity: 'high',
      title: 'Missing signature check',
      body: 'Unrelated extra text',
    }),
    // Distinct finding on the same line -> kept and sorted before warn/info
    makeFinding({
      id: 'f-crit',
      path: 'src/auth.ts',
      line: 42,
      severity: 'critical',
      title: 'SQL injection in query builder',
      body: 'User input concatenated directly into SQL.',
    }),
  ];

  const idx = buildAiOverlayIndex(comments, findings);
  assert.equal(idx.total, 3);
  const lineItems = aiOverlayForLine(idx, 'src/auth.ts', 42);
  assert.deepEqual(
    lineItems.map((x) => x.id),
    ['f-crit', 'c-dup', 'c-info'],
    'sorts critical before warn before info and suppresses duplicate finding',
  );
});

test('aiOverlaySeverityTone and aiOverlaySeverityLabel map every severity onto valid Badge tones', () => {
  assert.equal(aiOverlaySeverityTone('critical'), 'bad');
  assert.equal(aiOverlaySeverityTone('high'), 'bad');
  assert.equal(aiOverlaySeverityTone('bug'), 'bad');
  assert.equal(aiOverlaySeverityTone('medium'), 'warn');
  assert.equal(aiOverlaySeverityTone('warn'), 'warn');
  assert.equal(aiOverlaySeverityTone('low'), 'neutral');
  assert.equal(aiOverlaySeverityTone('info'), 'info');

  assert.equal(aiOverlaySeverityLabel('bug'), 'Bug');
  assert.equal(aiOverlaySeverityLabel('warn'), 'Warning');
  assert.equal(aiOverlaySeverityLabel('critical'), 'Critical');
  assert.equal(aiOverlaySeverityLabel('info'), 'Info');
});

// ── E-09: pushPreflight ──────────────────────────────────────────────────────

test('formatPreflightBytes and formatPreflightSummary render secrets, control paths, and large blobs', () => {
  assert.equal(formatPreflightBytes(512), '512 B');
  assert.equal(formatPreflightBytes(2048), '2 KiB');
  assert.equal(formatPreflightBytes(6 * 1024 * 1024), '6.0 MiB');

  const resp: PushPreflightResp = {
    ok: false,
    commits_ahead: 3,
    secrets: [
      { kind: 'secret', path: '.env.local', line: 12, detail: 'AWS key [REDACTED]', commit: 'abcdef123456' },
    ],
    control_paths: [
      { kind: 'control_path', file_path: '.github/workflows/release.yml', message: 'GitHub Actions workflow definition' },
    ],
    large_blobs: [
      { kind: 'large_blob', path: 'assets/weights.bin', size_bytes: 8_388_608, detail: 'exceeds 5 MiB limit' },
    ],
  };

  assert.equal(hasPreflightIssues(resp), true);
  const summary = formatPreflightSummary(resp, { remote: 'origin', branch: 'main' });
  assert.match(summary, /origin\/main across 3 outgoing commits/);
  assert.match(summary, /Potential secrets \(1\):/);
  assert.match(summary, /\.env\.local:12 — AWS key \[REDACTED\] \[abcdef1\]/);
  assert.match(summary, /Sensitive control paths \(1\):/);
  assert.match(summary, /\.github\/workflows\/release\.yml — GitHub Actions workflow definition/);
  assert.match(summary, /Large files \(> 5 MiB\) \(1\):/);
  assert.match(summary, /assets\/weights\.bin \(8\.0 MiB\) — exceeds 5 MiB limit/);
});

test('formatPreflightSummary caps long buckets and handles !ok with empty buckets', () => {
  const many = Array.from({ length: 8 }, (_, i) => ({
    path: `keys/id_${i}.pem`,
    detail: 'private key',
  }));
  const capped = formatPreflightSummary({ ok: false, control_paths: many });
  assert.match(capped, /Sensitive control paths \(8\):/);
  assert.match(capped, /…and 2 more/);

  const bare = formatPreflightSummary({ ok: false });
  assert.match(bare, /safety check flagged an issue/);
});

test('runPushPreflight passes through when ok: true without prompting', async () => {
  let confirmCalled = false;
  let capturedPath = '';
  let capturedBody: PushPreflightReq | null = null;
  const res = await runPushPreflight(
    'repo-1',
    { remote: 'origin', branch: 'feat', force: false },
    {
      post: async (path, body) => {
        capturedPath = path;
        capturedBody = body;
        return { ok: true, commits_ahead: 1, secrets: [], control_paths: [], large_blobs: [] };
      },
      confirm: async () => {
        confirmCalled = true;
        return false;
      },
    },
  );
  assert.equal(capturedPath, '/repos/repo-1/push/preflight');
  assert.deepEqual(capturedBody, { remote: 'origin', branch: 'feat', force: false });
  assert.equal(confirmCalled, false);
  assert.equal(res.ok, true);
  assert.equal(res.skipped, undefined);
});

test('runPushPreflight gracefully skips on HTTP 404, 405, and 501 (older daemon)', async () => {
  for (const status of [404, 405, 501]) {
    assert.equal(isPreflightUnsupported({ status }), true);
    let confirmCalled = false;
    const res = await runPushPreflight('repo-1', { branch: 'main' }, {
      post: async () => {
        const err = new Error('Not found') as Error & { status: number };
        err.status = status;
        throw err;
      },
      confirm: async () => {
        confirmCalled = true;
        return false;
      },
    });
    assert.equal(confirmCalled, false);
    assert.deepEqual(res, { ok: true, skipped: true });
  }
  assert.equal(isPreflightUnsupported({ status: 500 }), false);
});

test('runPushPreflight blocks push when user declines danger confirmation and allows on bypass', async () => {
  const flagged: PushPreflightResp = {
    ok: false,
    commits_ahead: 1,
    secrets: [{ path: 'config.ts', line: 7, detail: 'token [REDACTED]' }],
    control_paths: [],
    large_blobs: [],
  };

  let seenOpts: PushPreflightConfirmOpts | null = null;
  let seenMessage = '';
  const blocked = await runPushPreflight('repo-1', { remote: 'origin', branch: 'feat' }, {
    post: async () => flagged,
    confirm: async (msg, opts) => {
      seenMessage = msg;
      seenOpts = opts;
      return false;
    },
  });
  assert.equal(blocked.ok, false);
  if (!blocked.ok) assert.equal(blocked.canceled, true);
  assert.deepEqual(seenOpts, {
    title: 'Push safety check found issues',
    confirmLabel: 'Push anyway',
    danger: true,
  });
  assert.match(seenMessage, /config\.ts:7/);

  const bypassed = await runPushPreflight('repo-1', { remote: 'origin', branch: 'feat' }, {
    post: async () => flagged,
    confirm: async () => true,
  });
  assert.equal(bypassed.ok, true);
  if (bypassed.ok) assert.equal(bypassed.bypassed, true);
});
