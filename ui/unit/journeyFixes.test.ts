// S20 journey fixes pinned statically (the components need a DOM):
// onboarding Enter submits (S20-08), resumes after a reload (S20-11), an
// optional workspace failure doesn't block Finish (S20-12), a dead daemon
// reads as such (S20-13), password length counts code points like the server
// (S20-14); the palette ignores IME composition keys (S20-16); History's ⌘K
// resume uses the button's predicate (S20-15).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const read = (p: string): string => readFileSync(join(import.meta.dirname, '..', p), 'utf8');
const ob = read('src/modules/settings/Onboarding.svelte');

test('onboarding steps are forms: Enter submits, Back/Skip/Browse are type=button', () => {
  assert.equal((ob.match(/<form\s/g) ?? []).length, 2);
  assert.match(ob, /if \(pwValid && !busy && !rootTaken\) void confirmPassword\(\)/);
  assert.match(ob, /void confirmWorkspace\(\)/);
  assert.equal((ob.match(/type="submit"/g) ?? []).length, 2);
  assert.doesNotMatch(ob, /<button class="btn"[^>]*onclick=\{\(\) => \(step = /, 'Back buttons must not submit');
});

test('password length counts code points (server: chars().count())', () => {
  assert.match(ob, /const pwLen = \$derived\(\[\.\.\.password\]\.length\)/);
  assert.match(ob, /const pwValid = \$derived\(pwLen >= 10/);
  assert.doesNotMatch(ob, /password\.length/);
  // The rule itself: an emoji is one character.
  assert.equal([...'😀😀😀😀😀abcd'].length, 9);
  assert.equal('😀😀😀😀😀abcd'.length, 14);
});

test('a dead daemon reads as "isn’t responding", not the browser’s raw error', () => {
  assert.match(ob, /if \(!\(e instanceof ApiError\)\) return 'Otto’s background service isn’t responding/);
});

test('a failed optional workspace still finishes setup and toasts the error', () => {
  const fin = ob.slice(ob.indexOf('async function finish()'));
  assert.match(fin, /wsFailed = e;/);
  assert.ok(fin.indexOf('acceptLogin') < fin.indexOf("toasts.error('Setup finished, but the workspace wasn’t created'"));
});

test('a reload after the root exists resumes the wizard (tab-scoped step)', () => {
  const auth = read('src/lib/stores/auth.svelte.ts');
  assert.match(auth, /ssGet\(ONBOARDING_RESUME_KEY\)/);
  assert.match(auth, /this\.phase = 'onboarding';\s*return;/);
  assert.match(ob, /ssSet\(ONBOARDING_RESUME_KEY, String\(step\)\)/);
  assert.match(ob, /ssRemove\(ONBOARDING_RESUME_KEY\)/);
});

test('palette handlers ignore IME composition keys', () => {
  const pal = read('src/shell/Palette.svelte');
  for (const fn of ['onCommandsKey', 'onEnglishKey']) {
    const body = pal.slice(pal.indexOf(`function ${fn}(`), pal.indexOf(`function ${fn}(`) + 300);
    assert.match(body, /if \(e\.isComposing \|\| e\.keyCode === 229\) return;/, fn);
  }
});

test('History ⌘K resume is registered only when the button would allow it', () => {
  const h = read('src/modules/agents/history/HistoryPage.svelte');
  assert.match(h, /sel && canEdit && canResume\(sel\) \? \[\{ id: 'history\.resume'/);
  assert.match(h, /if \(e\.archived\) return false;/);
});
