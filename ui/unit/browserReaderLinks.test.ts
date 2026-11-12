// Reader view: a click on a page link navigates THIS tab through the browser's
// guarded navigation, and only to a web page — a link must never become a way
// to run script or open a local file.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readerLinkAction } from '../src/modules/browser/readerLinks.ts';

const PAGE = 'https://docs.example/guide/intro.html';

test('web links open in this tab, resolved against the page', () => {
  assert.deepEqual(readerLinkAction('https://other.example/a?b=1', PAGE), { kind: 'open', url: 'https://other.example/a?b=1' });
  assert.deepEqual(readerLinkAction('../api/', PAGE), { kind: 'open', url: 'https://docs.example/api/' });
  assert.deepEqual(readerLinkAction('//cdn.example/x', PAGE), { kind: 'open', url: 'https://cdn.example/x' });
  assert.deepEqual(readerLinkAction('http://plain.example/', PAGE), { kind: 'open', url: 'http://plain.example/' });
});

test('in-page anchors are left to the page', () => {
  assert.deepEqual(readerLinkAction('#install', PAGE), { kind: 'anchor' });
  assert.deepEqual(readerLinkAction('#', PAGE), { kind: 'anchor' });
});

test('script, local-file and app schemes are blocked', () => {
  for (const href of ['javascript:alert(1)', ' JavaScript:alert(1)', 'file:///etc/passwd', 'data:text/html,<b>x</b>',
    'tauri://localhost/', 'vscode://file/x', 'mailto:a@b.example', '', null]) {
    assert.deepEqual(readerLinkAction(href, PAGE), { kind: 'block' }, String(href));
  }
});
