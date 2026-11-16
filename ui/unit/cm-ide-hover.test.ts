import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  buildIdeHoverRequest,
  extractCleanSymbolHoverFromSource,
  sanitizeAndSplitHoverDoc,
} from '../src/lib/components/cm-ide-hover.ts';

test('sanitizeAndSplitHoverDoc strips fenced backticks, --- rules, and domainless pkg.go.dev links while keeping external links', () => {
  const rawGoplsMarkdown = [
    '```go',
    'func InitApp(name string, version string)',
    '```',
    '---',
    'InitApp initializes telemetry, HTTP routes, and graceful shutdown hooks for the service. ' +
      'It validates required environment variables before binding listeners. ' +
      'Callers should invoke InitApp once from main before starting background workers. ' +
      'Additional lifecycle hooks are flushed automatically when SIGTERM arrives in production.',
    '',
    '[`app.InitApp` on pkg.go.dev](https://pkg.go.dev/promotions/internal/app#InitApp)',
    '[`decimal.Decimal` on pkg.go.dev](https://pkg.go.dev/github.com/shopspring/decimal#Decimal)',
  ].join('\n');

  const doc = sanitizeAndSplitHoverDoc({
    symbol: 'InitApp',
    packageName: 'app',
    rawDoc: rawGoplsMarkdown,
  });

  assert.ok(doc);
  assert.equal(doc.kind, 'function');
  assert.equal(doc.signature, 'func InitApp(name string, version string)');
  assert.ok(!doc.signature.includes('```'), 'signature must never contain raw triple backticks');
  assert.ok(!doc.summary.includes('---'), 'summary must strip horizontal rules');
  assert.ok(!doc.summary.includes('pkg.go.dev'), 'summary must strip pkg.go.dev link noise');
  assert.ok(doc.summary.length <= 240, `summary must be <= 240 chars, got ${doc.summary.length}`);
  assert.ok(doc.moreParagraphs.length >= 1, 'overflow sentences should move to expandable full docs');

  // Domainless local module `promotions/internal/app` was stripped; external `github.com/shopspring/decimal` kept!
  assert.equal(doc.externalLinks.length, 1);
  assert.equal(doc.externalLinks[0].url, 'https://pkg.go.dev/github.com/shopspring/decimal#Decimal');
});

test('sanitizeAndSplitHoverDoc isolates single symbol from Go var (...) group block without leaking siblings', () => {
  const dumpedGroupSig = [
    'var (',
    '\t// appName is the service identifier.',
    '\tappName = "promotions"',
    '\t// BuildTime is injected by ldflags at release build.',
    '\tBuildTime = "unknown"',
    '\t// Commit is the git revision.',
    '\tCommit = "HEAD"',
    ')',
  ].join('\n');

  const parsed = sanitizeAndSplitHoverDoc({
    symbol: 'BuildTime',
    signature: dumpedGroupSig,
  });

  assert.ok(parsed);
  assert.equal(parsed.signature, 'var BuildTime = "unknown"');
  assert.ok(!parsed.signature.includes('appName'));
  assert.ok(!parsed.signature.includes('Commit'));
  assert.equal(parsed.summary, 'BuildTime is injected by ldflags at release build.');
});

test('extractCleanSymbolHoverFromSource extracts exact item comments across Go var groups, Java Javadoc, and Rust /// docs', () => {
  const goSrc = [
    'package main',
    '',
    '// Group header comment that belongs to the block, not Commit.',
    'var (',
    '\t// appName identifies the binary.',
    '\tappName = "otto"',
    '',
    '\t// Version is the semantic version tag.',
    '\tVersion = "0.2.2"',
    '\tCommit  = "ea68519"',
    ')',
  ].join('\n');

  const verHover = extractCleanSymbolHoverFromSource(goSrc, 'Version', 'go');
  assert.ok(verHover);
  assert.equal(verHover.packageName, 'main');
  assert.equal(verHover.signature, 'var Version = "0.2.2"');
  assert.equal(verHover.summary, 'Version is the semantic version tag.');

  // `Commit` has no comment immediately above it (preceded by `Version = ...`), so `Version`'s doc must NOT leak!
  const commitHover = extractCleanSymbolHoverFromSource(goSrc, 'Commit', 'go');
  assert.ok(commitHover);
  assert.equal(commitHover.signature, 'var Commit  = "ea68519"');
  assert.equal(commitHover.summary, '');

  const rustSrc = [
    '/// Parses an incoming frame from the wire.',
    '/// Returns `None` when the buffer is incomplete.',
    'pub fn parse_frame(buf: &[u8]) -> Option<Frame> {',
    '    None',
    '}',
  ].join('\n');
  const rsHover = extractCleanSymbolHoverFromSource(rustSrc, 'parse_frame', 'rust');
  assert.ok(rsHover);
  assert.equal(rsHover.kind, 'function');
  assert.ok(rsHover.summary.includes('Parses an incoming frame'));
});

test('buildIdeHoverRequest matches backend IdeHoverReq without duplicate column field and blocks secret/invalid paths', () => {
  const localReq = buildIdeHoverRequest({
    root: '/ws/app',
    filePath: 'cmd/server/main.go',
    content: 'package main\n',
    line: 14,
    character: 6,
  });
  assert.ok(localReq);
  assert.deepEqual(localReq, {
    target: {
      kind: 'local',
      root: '/ws/app',
      connection_id: null,
    },
    path: 'cmd/server/main.go',
    content: 'package main\n',
    line: 14,
    character: 6,
  });
  assert.equal('column' in (localReq as unknown as Record<string, unknown>), false);

  const sshReq = buildIdeHoverRequest({
    root: '/srv/mono',
    filePath: 'svc/handler.go',
    content: 'package svc\n',
    line: 3,
    character: 0,
    connectionId: 'conn_ssh_1',
    projectFolders: ['/srv/mono/svc', '/srv/mono/pkg'],
  });
  assert.ok(sshReq);
  assert.equal(sshReq.target.kind, 'ssh');
  assert.equal(sshReq.target.connection_id, 'conn_ssh_1');
  assert.deepEqual(sshReq.target.additional_roots, ['/srv/mono/svc', '/srv/mono/pkg']);
  assert.deepEqual(sshReq.project_folders, ['/srv/mono/svc', '/srv/mono/pkg']);
  assert.equal('column' in (sshReq as unknown as Record<string, unknown>), false);

  assert.equal(buildIdeHoverRequest({ root: '/ws/app', filePath: 'main.go', content: '', line: 0, character: 0 }), null);
  assert.equal(buildIdeHoverRequest({ root: '/ws/app', filePath: 'main.go', content: '', line: 1, character: -1 }), null);
  assert.equal(buildIdeHoverRequest({ root: '', filePath: 'main.go', content: '', line: 1, character: 0 }), null);
  assert.equal(buildIdeHoverRequest({ root: '/ws/app', filePath: 'untitled:1', content: '', line: 1, character: 0 }), null);
  assert.equal(buildIdeHoverRequest({ root: '/ws/app', filePath: '.env.production', content: '', line: 1, character: 0 }), null);
  assert.equal(buildIdeHoverRequest({ root: '/ws/app', filePath: '~/.ssh/id_ed25519', content: '', line: 1, character: 0 }), null);
});

test('extractCleanSymbolHoverFromSource disambiguates selector calls (g.pool.Submit) vs enclosing method and external receivers', () => {
  // 1. Same-file Pool struct + GoRoutineConcurrencyHelper struct both defining `Submit`:
  // Hovering `g.pool.Submit(f)` MUST resolve to `func (p *Pool) Submit`, NEVER `GoRoutineConcurrencyHelper.Submit`!
  const sameFilePoolSrc = [
    'package concurrency',
    '',
    'type Pool struct{}',
    '',
    '// Submit enqueues work onto the underlying goroutine pool.',
    'func (p *Pool) Submit(task func()) error {',
    '\treturn nil',
    '}',
    '',
    'type GoRoutineConcurrencyHelper struct {',
    '\tpool *Pool',
    '}',
    '',
    '// Submit submits a task to the goroutine pool for execution.',
    'func (g *GoRoutineConcurrencyHelper) Submit(ctx context.Context, f func()) {',
    '\terr := g.pool.Submit(f)',
    '\t_ = err',
    '}',
  ].join('\n');

  const callOffsetSameFile = sameFilePoolSrc.indexOf('g.pool.Submit(f)') + 'g.pool.'.length;
  const sameFileHover = extractCleanSymbolHoverFromSource(
    sameFilePoolSrc,
    'Submit',
    'go',
    true,
    callOffsetSameFile,
  );
  assert.ok(sameFileHover, 'same-file target receiver Pool.Submit should resolve cleanly');
  assert.equal(sameFileHover.signature, 'func (p *Pool) Submit(task func()) error');
  assert.equal(sameFileHover.summary, 'Submit enqueues work onto the underlying goroutine pool.');
  assert.ok(!sameFileHover.signature.includes('GoRoutineConcurrencyHelper'));

  // 2. External package receiver (`pool *ants.Pool`) alongside an unrelated same-file `Worker.Submit`:
  // Hovering `g.pool.Submit(f)` MUST return `null` from same-file fallback so neither enclosing
  // `GoRoutineConcurrencyHelper.Submit` nor unrelated `Worker.Submit` hijacks the hover!
  const externalPoolSrc = [
    'package concurrency',
    '',
    'type Worker struct{}',
    'func (w *Worker) Submit(job int) {}',
    '',
    'type GoRoutineConcurrencyHelper struct {',
    '\tpool *ants.Pool',
    '}',
    '',
    '// Submit submits a task to the goroutine pool for execution.',
    'func (g *GoRoutineConcurrencyHelper) Submit(ctx context.Context, f func()) {',
    '\terr := g.pool.Submit(f)',
    '\t_ = err',
    '}',
  ].join('\n');

  const callOffsetExternal = externalPoolSrc.indexOf('g.pool.Submit(f)') + 'g.pool.'.length;
  const externalHover = extractCleanSymbolHoverFromSource(
    externalPoolSrc,
    'Submit',
    'go',
    true,
    callOffsetExternal,
  );
  assert.equal(
    externalHover,
    null,
    'external *ants.Pool selector call must never match enclosing GoRoutineConcurrencyHelper.Submit or unrelated Worker.Submit',
  );
});
