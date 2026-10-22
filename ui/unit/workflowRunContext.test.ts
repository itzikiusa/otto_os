import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runContextPath } from '../src/modules/workflows/runContext.ts';

test('the context tree maps its absolute root and children onto run-relative paths', () => {
  const dir = '/Users/me/Library/Application Support/Otto/workflow-context/01RUN';
  assert.equal(runContextPath(dir, dir), '', 'the context dir itself is the route root');
  assert.equal(runContextPath(`${dir}/`, dir), '');
  assert.equal(runContextPath(dir, `${dir}/steps/brief.md`), 'steps/brief.md');
  assert.equal(runContextPath(dir, 'steps/brief.md'), 'steps/brief.md', 'route entries are already relative');
  assert.equal(runContextPath(dir, '/etc/passwd'), '/etc/passwd', 'anything else is passed as-is for the route to refuse');
});
