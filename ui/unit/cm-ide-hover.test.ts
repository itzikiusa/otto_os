import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
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
