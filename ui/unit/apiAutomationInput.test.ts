import {test} from 'node:test';
import assert from 'node:assert/strict';
import {editableSteps} from '../src/modules/api/automationInput.ts';

test('API-created minimal automation is editable without assertion/extraction arrays', () => {
  const steps=editableSteps([{request_id:'request'} as any]);
  assert.deepEqual(steps,[{request_id:'request',assertions:[],extract:[]}]);
});
test('editing an assertion does not mutate the persisted automation snapshot', () => {
  const source=[{request_id:'request',assertions:[{kind:'status',op:'eq',value:'200'}],extract:[]}];
  const steps=editableSteps(source as any);steps[0].assertions[0].value='201';
  assert.equal(source[0].assertions[0].value,'200');
});
