import { test } from 'node:test';
import assert from 'node:assert/strict';
import { patchProperties } from '../src/modules/vault/properties.ts';

test('structured properties preserve unknown YAML fields, comments and the exact body', () => {
  const body = '# Body\r\n\r\nKeep [links](other.md) and code.\r\n';
  const raw = '---\n# Custom setting\ncustom:\n  count: 3\ntitle: Old\n---\n' + body;
  const updated = patchProperties(raw, {title: 'New: title', tags: 'one, two/nested'});
  assert.ok(updated.endsWith(body)); assert.match(updated, /# Custom setting/);
  assert.match(updated, /count: 3/); assert.match(updated, /title: ['"]New: title['"]/);
  assert.match(updated, /- two\/nested/);
});
test('blank values remove a property and malformed frontmatter is refused', () => {
  assert.doesNotMatch(patchProperties('---\ntitle: Old\ncustom: yes\n---\nbody', {title: ''}), /title:/);
  assert.throws(() => patchProperties('---\ntitle: [broken\n---\nbody', {title: 'new'}));
  assert.throws(() => patchProperties('---\ntitle: no closing fence', {title: 'new'}));
});
