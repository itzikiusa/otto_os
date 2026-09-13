import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
const grid = () => loadSource(new URL('../src/lib/components/folderGrid.ts', import.meta.url), {});

test('keyboard navigation reaches all entries independently of rendered window', () => {
  const { moveGrid }=grid();
  assert.deepEqual({...moveGrid({index:0,action:0},'End',10000,10)}, {index:9999,action:0});
  assert.deepEqual({...moveGrid({index:9999,action:1},'PageUp',10000,10)}, {index:9989,action:1});
  assert.deepEqual({...moveGrid({index:3,action:1},'Home',10000,10)}, {index:0,action:1});
  assert.deepEqual({...moveGrid({index:0,action:0},'ArrowUp',10000,10)}, {index:0,action:0});
  assert.equal(moveGrid({index:0,action:0},'Tab',10000,10),null);
});

test('selection action respects files and git-only eligibility', () => {
  const { gridAction }=grid();
  assert.equal(gridAction({is_dir:true,is_git_repo:false},1,true),null);
  assert.equal(gridAction({is_dir:true,is_git_repo:true},1,true),'pick');
  assert.equal(gridAction({is_dir:true,is_git_repo:false},0,true),'open');
  assert.equal(gridAction({is_dir:false,is_git_repo:false},0,false),'pick');
  assert.equal(gridAction({is_dir:false,is_git_repo:false},1,false),null);
});
