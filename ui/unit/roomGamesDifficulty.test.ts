import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createGame, defaultInput, stepGame } from '../src/modules/rooms/games/simulation.ts';
import type { Difficulty } from '../src/modules/rooms/games/types.ts';

function measure(difficulty:Difficulty,scene:string,seed:number,seconds=30) {
 const state=createGame({kind:'shooter',map:['open','moving','close'].includes(scene)?'station':scene,difficulty,vsComputer:true,seed});
 state.phase='playing';
 if(['open','moving','close'].includes(scene)) {
  Object.assign(state.players[0],{x:20,z:2,invulnerable:0});
  Object.assign(state.players[1],{x:20,z:scene==='close'?0:-6,yaw:0,invulnerable:0});
 }
 let firstDamage=Infinity,firstKill=Infinity,damage=0,damage10=0,shots10=0,shots=0;
 for(let tick=0;tick<seconds*60;tick++) {
  const input=defaultInput();
  if(scene==='moving')input.moveX=Math.floor(tick/45)%2===0?1:-1;
  stepGame(state,[input,defaultInput()],1/60);
  for(const event of state.events)if(event.player===1) {
   if(event.type==='hit'){firstDamage=Math.min(firstDamage,state.elapsed);damage+=25;if(tick<600)damage10+=25;}
   if(event.type==='kill')firstKill=Math.min(firstKill,state.elapsed);
   if(event.type==='shot'){shots++;if(tick<600)shots10++;}
  }
 }
 return {firstDamage,firstKill,damage,damage10,shots10,shots};
}

test('Easy gives acquisition time, short bursts and long breaks even against a stationary close target',()=>{
 for(const seed of [2121,72813,123]) {
  const m=measure('easy','open',seed);
  assert.ok(m.firstDamage>=2,`first damage after only ${m.firstDamage}s`);
  assert.ok(m.firstKill>=10,`first elimination after only ${m.firstKill}s`);
  assert.ok(m.shots10<=6,`Easy fired ${m.shots10} shots in 10 seconds`);
  assert.ok(m.damage10<=75,`Easy dealt ${m.damage10} damage in 10 seconds`);
 }
});
test('moving players survive the first ten seconds against Easy',()=>{
 for(const seed of [2121,72813,123])assert.ok(measure('easy','moving',seed).firstKill>=10);
});
test('Easy, Normal and Hard have increasing sustained damage across every arena and open line of sight',()=>{
 for(const scene of ['open','station','foundry','dunes']) {
  const totals=(difficulty:Difficulty)=>[2121,72813,123].map(seed=>measure(difficulty,scene,seed));
  const samples={easy:totals('easy'),normal:totals('normal'),hard:totals('hard')};
  for(const window of ['damage10','damage'] as const) {
   const easy=samples.easy.reduce((sum,m)=>sum+m[window],0),normal=samples.normal.reduce((sum,m)=>sum+m[window],0),hard=samples.hard.reduce((sum,m)=>sum+m[window],0);
   assert.ok(easy<normal,`${scene}/${window}: Easy ${easy} >= Normal ${normal}`);
   assert.ok(normal<hard,`${scene}/${window}: Normal ${normal} >= Hard ${hard}`);
  }
 }
});

test('Easy aim misses remain meaningful at two metres',()=>{
 const samples=[2121,72813,123].map(seed=>measure('easy','close',seed));
 const shots=samples.reduce((sum,m)=>sum+m.shots,0),hits=samples.reduce((sum,m)=>sum+m.damage/25,0);
 assert.ok(shots>=12,'bot should attempt shots rather than becoming inert');
 assert.ok(hits/shots<0.65,`close-range accuracy ${hits}/${shots} is too high for Easy`);
});
test('losing line of sight makes Easy reacquire instead of instantly firing when the player returns',()=>{
 const state=createGame({kind:'shooter',map:'station',difficulty:'easy',vsComputer:true,seed:2121});state.phase='playing';
 Object.assign(state.players[0],{x:20,z:2,invulnerable:0});Object.assign(state.players[1],{x:20,z:-6,yaw:0,invulnerable:0});
 for(let tick=0;tick<180;tick++)stepGame(state,[defaultInput(),defaultInput()],1/60);
 // Put cover between the combatants for longer than Easy's decision interval.
 Object.assign(state.players[0],{x:0,z:8});Object.assign(state.players[1],{x:0,z:-8});
 for(let tick=0;tick<60;tick++)stepGame(state,[defaultInput(),defaultInput()],1/60);
 Object.assign(state.players[0],{x:20,z:2,hp:100});Object.assign(state.players[1],{x:20,z:-6,yaw:0});
 for(let tick=0;tick<120;tick++) {
  stepGame(state,[defaultInput(),defaultInput()],1/60);
  assert.equal(state.players[0].hp,100,'returning player must get a fresh acquisition delay');
 }
});
