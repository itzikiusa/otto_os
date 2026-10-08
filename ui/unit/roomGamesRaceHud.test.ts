import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createGame} from '../src/modules/rooms/games/simulation.ts';
import {racePosition,raceClock,courseOutline} from '../src/modules/rooms/games/race-hud.ts';
import {trackFor} from '../src/modules/rooms/games/maps.ts';
test('race position follows earned laps and ordered checkpoints',()=>{
 const state=createGame({kind:'kart',map:'coast',difficulty:'easy',vsComputer:true});
 state.players[1].lap=1;assert.equal(racePosition(state,0),2);
 state.players[0].lap=2;assert.equal(racePosition(state,0),1);
 state.players[1].lap=2;state.players[1].checkpoint=5;assert.equal(racePosition(state,0),2);
});
test('clock formats a race duration and outline includes every sampled height-independent point',()=>{
 assert.equal(raceClock(65.87),'1:05.87');assert.equal(raceClock(0),'0:00.00');
 const course=trackFor('coast');assert.equal(courseOutline(course).split(' ').length,course.route.length+1);
});
