import { test } from 'node:test';
import assert from 'node:assert/strict';
import { neighbourAfterDelete } from '../src/modules/workflows/canvasFocus.ts';

test('a deleted step hands focus to the next step, else the previous, else the canvas', () => {
  const nodes = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
  assert.equal(neighbourAfterDelete(nodes, 'a'), 'b');
  assert.equal(neighbourAfterDelete(nodes, 'b'), 'c');
  assert.equal(neighbourAfterDelete(nodes, 'c'), 'b', 'the last step falls back to the one before it');
  assert.equal(neighbourAfterDelete([{ id: 'solo' }], 'solo'), null);
  assert.equal(neighbourAfterDelete(nodes, 'gone'), null);
});
