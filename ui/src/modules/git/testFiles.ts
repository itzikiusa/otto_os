// "Is this a test file?" — powers the PR Code tab's "Hide test files" toggle.
// One pure, path-only heuristic across the languages and frameworks Otto's
// users review (JS/TS, Go, Python, JVM, .NET, Ruby, Rust, PHP, Swift, Dart,
// Elixir, C/C++). It errs toward precision: a path is a test only when a
// whole path SEGMENT or a well-known file-name suffix says so, so `contest.ts`,
// `latest.go` or `attestation/` never match. Pure so unit/testFiles.test.ts
// runs it under node --test.

/** Directory names that hold tests (matched as whole segments, any case). */
const TEST_DIRS = new Set([
  'test',
  'tests',
  '__tests__',
  '__mocks__',
  '__snapshots__',
  '__fixtures__',
  'spec',
  'specs',
  'e2e',
  'cypress',
  'playwright',
  'testdata',
  'test-data',
  'testing',
  'unittests',
  'integration-tests',
  'integration_tests',
]);

/** File-name patterns (applied to the basename). */
const TEST_FILE_RES: readonly RegExp[] = [
  // JS/TS & friends: foo.spec.ts, foo.test.jsx, foo.e2e-spec.ts, foo.cy.ts
  /\.(spec|test|e2e-spec|e2e|cy)\.[cm]?[jt]sx?$/i,
  // Any other language using the dotted convention: foo.test.py, bar.spec.rb …
  /\.(spec|test)\.[a-z0-9]+$/i,
  // Jest snapshots
  /\.snap$/i,
  // Go
  /_test\.go$/,
  // Python: test_foo.py, foo_test.py, conftest.py
  /^test_.+\.py$/i,
  /_tests?\.py$/i,
  /^conftest\.py$/i,
  // JVM: FooTest.java, FooTests.java, FooIT.java, FooTest.kt, FooSpec.scala/groovy/kt
  /[a-z0-9](Test|Tests|IT|TestCase)\.(java|kt|kts|scala|groovy)$/,
  /[a-z0-9]Spec\.(scala|groovy|kt)$/,
  // .NET: FooTests.cs, FooTest.cs, FooTests.fs, FooTests.vb
  /[a-z0-9](Test|Tests)\.(cs|fs|vb)$/,
  // Ruby
  /_spec\.rb$/i,
  /_test\.rb$/i,
  // Rust (unit tests in a sibling file)
  /_tests?\.rs$/i,
  // PHP
  /[a-z0-9]Test\.php$/,
  // Swift / Obj-C
  /[a-z0-9]Tests?\.(swift|m|mm)$/,
  // Dart / Elixir / Erlang
  /_test\.dart$/i,
  /_test\.exs$/i,
  /_SUITE\.erl$/,
  // C / C++
  /(_test|_unittest|_tests)\.(c|cc|cpp|cxx|h|hpp)$/i,
  // JS/TS conventions without a dot: fooSpec.js is too ambiguous — skipped.
];

/** `.NET` test projects: `Foo.Tests/`, `Foo.UnitTests/`, `Foo.IntegrationTests/`. */
const DOTNET_TEST_DIR = /\.(unit|integration|functional|acceptance)?tests?$/i;
/** Swift / Xcode test targets: `FooTests/`, `FooUITests/`. */
const XCODE_TEST_DIR = /^[A-Z][A-Za-z0-9]*(UI)?Tests$/;

/** True when `path` (repo-relative, `/`-separated) looks like a test file. */
export function isTestPath(path: string): boolean {
  if (!path) return false;
  const parts = path.split('/').filter((p) => p !== '');
  if (parts.length === 0) return false;
  const base = parts[parts.length - 1];
  for (const re of TEST_FILE_RES) if (re.test(base)) return true;
  const dirs = parts.slice(0, -1);
  for (let i = 0; i < dirs.length; i++) {
    const d = dirs[i];
    const low = d.toLowerCase();
    if (TEST_DIRS.has(low)) return true;
    // JVM / Maven / Gradle: src/test/…, src/androidTest/…, src/integrationTest/…
    if (i > 0 && dirs[i - 1].toLowerCase() === 'src' && /^[a-z]*test$/i.test(d)) return true;
    if (d.includes('.') && DOTNET_TEST_DIR.test(d)) return true;
    if (XCODE_TEST_DIR.test(d)) return true;
  }
  return false;
}
