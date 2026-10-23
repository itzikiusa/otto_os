import {test} from 'node:test';
import assert from 'node:assert/strict';
import {editableSteps, movedArrow, savedSteps} from '../src/modules/api/automationInput.ts';

test('API-created minimal automation is editable without assertion/extraction arrays', () => {
  const steps=editableSteps([{request_id:'request'} as any]);
  assert.deepEqual(savedSteps(steps),[{request_id:'request',assertions:[],extract:[]}]);
  assert.match(steps[0].key,/^r\d+$/);
});
test('editing an assertion does not mutate the persisted automation snapshot', () => {
  const source=[{request_id:'request',assertions:[{kind:'status',op:'eq',value:'200'}],extract:[]}];
  const steps=editableSteps(source as any);steps[0].assertions[0].value='201';
  assert.equal(source[0].assertions[0].value,'200');
});

// a11y-6-03: rows key by a stable client id, so a moved step keeps its key
// (and its focused arrow); the key never reaches the daemon.
test('steps, checks and saved values get distinct keys that Save strips', () => {
  const steps=editableSteps([
    {request_id:'a',assertions:[{kind:'status',op:'eq',value:'200'},{kind:'status',op:'lt',value:'500'}],extract:[{path:'$.id',var:'id'}]},
    {request_id:'b',assertions:[],extract:[{path:'',var:''}]},
  ] as any);
  const keys=[steps[0].key,steps[1].key,...steps[0].assertions.map(a=>a.key),steps[0].extract[0].key];
  assert.equal(new Set(keys).size,keys.length,'every row has its own key');
  const moved=[steps[1],steps[0]];
  assert.deepEqual(moved.map(s=>s.key),[steps[1].key,steps[0].key],'a move carries the keys with the rows');
  const wire=savedSteps(steps);
  assert.ok(!JSON.stringify(wire).includes('"key"'),'no client key on the wire');
  assert.deepEqual(wire[1].extract,[],'an incomplete saved value is dropped');
});

test('after a move, focus goes to the same arrow unless the edge disabled it', () => {
  assert.deepEqual(movedArrow(2,-1,4),{index:1,arrow:'up'});
  assert.deepEqual(movedArrow(1,-1,4),{index:0,arrow:'down'},'top edge: Up is disabled, so Down');
  assert.deepEqual(movedArrow(1,1,4),{index:2,arrow:'down'});
  assert.deepEqual(movedArrow(2,1,4),{index:3,arrow:'up'},'bottom edge: Down is disabled, so Up');
});
