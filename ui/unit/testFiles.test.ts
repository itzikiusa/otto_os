// "Hide test files" heuristic (modules/git/testFiles.ts): many languages,
// whole-segment / suffix matches only, no substring false positives.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isTestPath } from '../src/modules/git/testFiles.ts';

const TESTS = [
  // JS / TS
  'apps/cbo/src/app/vip-program.routes.spec.ts',
  'src/foo.test.tsx',
  'src/foo.test.mjs',
  'lib/__tests__/util.ts',
  'lib/__mocks__/fs.ts',
  'src/__snapshots__/view.ts.snap',
  'ui/e2e/desktop-git.spec.ts',
  'ui/e2e/helpers.ts',
  'cypress/integration/login.js',
  'apps/web/playwright/smoke.ts',
  'apps/api/test/app.e2e-spec.ts',
  'src/button.cy.tsx',
  // Go
  'internal/wallet/service_test.go',
  'pkg/parser/testdata/input.json',
  // Python
  'tests/test_api.py',
  'pkg/test_models.py',
  'pkg/models_test.py',
  'conftest.py',
  // JVM
  'service/src/test/java/com/acme/FooService.java',
  'app/src/androidTest/java/Foo.kt',
  'core/src/main/java/com/acme/FooServiceTest.java',
  'core/FooServiceTests.java',
  'core/FooIT.java',
  'core/FooTest.kt',
  'core/FooSpec.scala',
  // .NET
  'src/Acme.Tests/WalletTests.cs',
  'src/Acme.UnitTests/Thing.cs',
  'src/Wallet/WalletTest.cs',
  // Ruby
  'spec/models/user_spec.rb',
  'lib/user_test.rb',
  // Rust
  'crates/otto-git/tests/providers.rs',
  'src/parser_tests.rs',
  // PHP / Swift / Dart / Elixir / C++
  'tests/Unit/UserTest.php',
  'src/UserTest.php',
  'App/AppTests/LoginTests.swift',
  'Sources/FooTests.swift',
  'lib/widget_test.dart',
  'test/my_app_test.exs',
  'src/codec_test.cc',
  'src/codec_unittest.cpp',
];

const NOT_TESTS = [
  'src/contest.ts',
  'cmd/latest.go',
  'pkg/attestation/verify.go',
  'src/testimonials/list.tsx',
  'src/components/Button.stories.tsx',
  'src/protest.py',
  'docs/testing-guide.md.txt/readme',
  'apps/cbo/src/app/modules/vip-program.routes.ts',
  'src/main/java/com/acme/Contest.java',
  'src/Latest.cs',
  'crates/otto-git/src/providers/github.rs',
  'README.md',
  'libs/data-models/facade-vip-program.ts',
  'src/attest.rs',
  '',
];

test('test files across languages are recognised', () => {
  for (const p of TESTS) assert.equal(isTestPath(p), true, `expected test: ${p}`);
});

test('ordinary source files are not', () => {
  for (const p of NOT_TESTS) assert.equal(isTestPath(p), false, `expected non-test: ${p}`);
});
