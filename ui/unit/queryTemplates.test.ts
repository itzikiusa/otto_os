import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CQL_TEMPLATES, JQL_TEMPLATES, LINEAR_TEMPLATES, fillLinearTemplate, fillTemplate } from '../src/modules/product/queryTemplates.ts';

test('templates fill the selected key, or a KEY placeholder', () => {
  assert.equal(fillTemplate('project = {P} AND parent = {P}-1', 'CS3'), 'project = CS3 AND parent = CS3-1');
  assert.equal(fillTemplate('project = {P}', ''), 'project = KEY');
  assert.equal(fillTemplate('space = {P}', 'my "space"'), 'space = "my \\"space\\""');
  assert.equal(fillTemplate('space = {P}', 'a\\b'), 'space = "a\\\\b"');
});

test('every template is non-empty and uniquely labelled', () => {
  for (const list of [JQL_TEMPLATES, CQL_TEMPLATES, LINEAR_TEMPLATES]) {
    assert.equal(new Set(list.map((t) => t.label)).size, list.length);
    for (const t of list) assert.ok(t.query.trim().length > 0, t.label);
  }
});

test('Linear templates are IssueFilter JSON objects once filled', () => {
  for (const t of LINEAR_TEMPLATES) {
    for (const key of ['ENG', '', 'we"ird']) {
      const v = JSON.parse(fillLinearTemplate(t.query, key));
      assert.equal(typeof v, 'object', t.label);
      assert.ok(v && !Array.isArray(v), t.label);
    }
  }
  assert.equal(fillLinearTemplate('{"team":{"key":{"eq":"{P}"}}}', 'ENG'), '{"team":{"key":{"eq":"ENG"}}}');
  assert.equal(fillLinearTemplate('{"team":{"key":{"eq":"{P}"}}}', ''), '{"team":{"key":{"eq":"KEY"}}}');
});
