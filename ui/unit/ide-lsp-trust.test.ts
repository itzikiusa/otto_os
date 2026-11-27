import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  IDE_TRUST_PROJECT_ENV,
  dismissIdeTrustNotice,
  ideLspTrustConfirm,
  ideLspTrustNotice,
  ideProjectTrustOn,
  ideProjectTrusted,
  ideTrustErrorText,
  ideTrustNoticeDismissed,
  showIdeTrustNotice,
  withProjectTrust,
} from '../src/modules/ide/editor/ideLspTrust.ts';
import { ideLspItem, type IdeLspRecord } from '../src/modules/ide/editor/ideStatusModel.ts';
import type { IdeEnvVar } from '../src/modules/ide/store/ideTypes.ts';

const refused = (lang: string): IdeLspRecord => ({ lang, rootPath: '/p', phase: 'unavailable', reason: 'trust_or_unavailable' });

test('a trust refusal becomes one plain notice per language, only while the project has no opt-in', () => {
  assert.equal(ideLspTrustNotice('zig', refused('zig'), false), 'Code completion for Zig is off: zls needs to run this project’s build.zig.');
  assert.match(ideLspTrustNotice('kotlin', refused('kotlin'), false) ?? '', /^Code completion for Kotlin is off: kotlin-language-server needs to import this project’s Gradle or Maven build\.$/);
  assert.match(ideLspTrustNotice('vue', refused('vue'), false) ?? '', /vue-language-server needs to load the plugins this project’s tsconfig\.json names/);
  assert.match(ideLspTrustNotice('csharp', refused('csharp'), false) ?? '', /^Code completion for C# is off: csharp-ls needs to load this project’s MSBuild project/);
  assert.match(ideLspTrustNotice('fsharp', refused('fsharp'), false) ?? '', /^Code completion for F# is off: fsautocomplete /);
  // Trusted (a server that still fails is not a trust problem), env not loaded yet (no guess),
  // not refused, or a language that runs no project code: no notice.
  assert.equal(ideLspTrustNotice('zig', refused('zig'), true), null);
  assert.equal(ideLspTrustNotice('zig', refused('zig'), null), null);
  assert.equal(ideLspTrustNotice('zig', { ...refused('zig'), phase: 'ready' }, false), null);
  assert.equal(ideLspTrustNotice('zig', null, false), null);
  assert.equal(ideLspTrustNotice('go', { ...refused('go'), reason: 'no_session' }, false), null);
  // The status item says the same short state and its click trusts; no env-variable recipe.
  const item = ideLspItem('kotlin', refused('kotlin'), false);
  assert.deepEqual([item?.label, item?.action], ['kotlin-language-server needs trust', 'trust']);
  assert.doesNotMatch(item?.title ?? '', /OTTO_IDE_TRUST/);
  assert.equal(ideLspItem('kotlin', refused('kotlin'), true)?.action, 'settings');
});

test('the confirm names what runs and where: this Mac, or the SSH host', () => {
  const local = ideLspTrustConfirm('zig', { kind: 'local' });
  assert.match(local, /run this project’s own build code \(its build\.zig\) on this Mac\./);
  assert.match(local, /Only do this for code you trust\. You can turn it off in Project environment\./);
  assert.match(ideLspTrustConfirm('csharp', { kind: 'ssh', host: 'build-box' }), /\(its MSBuild targets\) on build-box\./);
  assert.match(ideLspTrustConfirm('vue', { kind: 'ssh', host: null }), /on the SSH host\./);
});

test('trust is read from non-secret "1" values of the project key or the language key', () => {
  const v = (key: string, value: string, secret = false): IdeEnvVar => ({ key, value, secret });
  assert.equal(ideProjectTrusted(null, 'zig'), null);
  assert.equal(ideProjectTrusted([], 'zig'), false);
  assert.equal(ideProjectTrusted([v(IDE_TRUST_PROJECT_ENV, '1')], 'vue'), true);
  assert.equal(ideProjectTrusted([v('OTTO_IDE_TRUST_ZIG_BUILD', ' 1 ')], 'zig'), true);
  assert.equal(ideProjectTrusted([v('OTTO_IDE_TRUST_ZIG_BUILD', '1')], 'kotlin'), false);
  assert.equal(ideProjectTrusted([v('OTTO_IDE_TRUST_DOTNET_LSP', '1')], 'fsharp'), true);
  assert.equal(ideProjectTrusted([v(IDE_TRUST_PROJECT_ENV, '1', true)], 'zig'), false);
  assert.equal(ideProjectTrusted([v(IDE_TRUST_PROJECT_ENV, 'true')], 'zig'), false);
  assert.equal(ideProjectTrustOn([v('OTTO_IDE_TRUST_ZIG_BUILD', '1')]), false);
  assert.equal(ideProjectTrustOn([v(IDE_TRUST_PROJECT_ENV, '1')]), true);
});

test('the environment merge adds or removes exactly the one variable and keeps the rest in order', () => {
  const rest: IdeEnvVar[] = [
    { key: 'A', value: 'x', secret: false },
    { key: 'TOKEN', secret: true, has_secret: true },
    { key: 'OTTO_IDE_TRUST_ZIG_BUILD', value: '1', secret: false },
  ];
  const on = withProjectTrust(rest, true);
  assert.deepEqual(on, [...rest, { key: IDE_TRUST_PROJECT_ENV, value: '1', secret: false }]);
  assert.equal(on[1], rest[1], 'the secret entry is passed through as is');
  assert.deepEqual(withProjectTrust(on, false), rest);
  // Already present (any value): set to "1" in place, never duplicated or moved.
  const mid: IdeEnvVar[] = [rest[0], { key: IDE_TRUST_PROJECT_ENV, value: '0', secret: false }, rest[1]];
  assert.deepEqual(withProjectTrust(mid, true), [rest[0], { key: IDE_TRUST_PROJECT_ENV, value: '1', secret: false }, rest[1]]);
  assert.deepEqual(withProjectTrust(rest, false), rest);
  assert.deepEqual(rest.length, 3, 'the input is not mutated');
});

test('Hide is per project and lasts until shown again (memory only)', () => {
  assert.equal(ideTrustNoticeDismissed('local::/a'), false);
  dismissIdeTrustNotice('local::/a');
  assert.equal(ideTrustNoticeDismissed('local::/a'), true);
  assert.equal(ideTrustNoticeDismissed('local::/b'), false);
  assert.equal(ideTrustNoticeDismissed('ssh:c1:/a'), false);
  showIdeTrustNotice('local::/a');
  assert.equal(ideTrustNoticeDismissed('local::/a'), false);
});

test('a refused trust save says what to do, in words', () => {
  const err = (status: number, message: string) => Object.assign(new Error(message), { status });
  assert.match(ideTrustErrorText(err(403, 'requires root')), /Only the root account can trust a local project/);
  assert.match(ideTrustErrorText(err(403, 'an agent session’s credential cannot use root authority here')), /agent session can’t trust/);
  assert.match(ideTrustErrorText(err(403, "an agent session's credential cannot use root authority here")), /agent session can’t trust/);
  assert.equal(ideTrustErrorText(new TypeError('Failed to fetch')), 'Couldn’t trust this project: Otto can’t reach the daemon.');
});
