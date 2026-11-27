import assert from 'node:assert/strict';
import { test } from 'node:test';
import { runOnlyDebugRefusal } from '../src/modules/ide/store/ideDebugLaunchHelpers.ts';

// BUG-30: Playwright and Angular Karma debug through js-debug; plain Karma stays Run-only.
test('only plain Karma run configurations refuse Debug', () => {
  const ngKarma = { runner: 'karma' as const, args: ['--no-install', 'ng', 'test', '--watch=false'] };
  const plainKarma = { runner: 'karma' as const, args: ['--no-install', 'karma', 'start', '--single-run'] };
  const playwright = { runner: 'playwright' as const, args: ['--no-install', 'playwright', 'test'] };
  assert.equal(runOnlyDebugRefusal(ngKarma), null);
  assert.equal(runOnlyDebugRefusal(playwright), null);
  assert.equal(runOnlyDebugRefusal(undefined), null);
  assert.match(runOnlyDebugRefusal(plainKarma) ?? '', /^Debug supports Karma in Angular CLI projects\./);
});
