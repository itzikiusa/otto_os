import {test} from 'node:test';
import assert from 'node:assert/strict';
import {FinishDelivery} from '../src/modules/rooms/games/finish-delivery.ts';
import type {GameRoomState} from '../src/lib/api/game-room-types';
const room:GameRoomState={id:'r',config:{game:'kart',map:'coast'},round:1,generation:2,phase:'playing',paused:false,members:[]};
test('unacknowledged finish retries under new connection generation and stops after acknowledgement',()=>{
 const delivery=new FinishDelivery();delivery.queue({winner:0,scores:[0,0],elapsed:99});
 assert.equal(delivery.command(room,0)?.generation,2);
 assert.equal(delivery.command({...room,generation:3,paused:true},1000),null);
 assert.equal(delivery.command({...room,generation:4},1100)?.generation,4);
 delivery.observe({...room,phase:'finished'});
 assert.equal(delivery.command(room,3000),null);
});
test('finish retry is bounded and a new round never receives old results',()=>{
 const delivery=new FinishDelivery();delivery.queue({winner:1,scores:[2,7],elapsed:20});
 assert.ok(delivery.command(room,0));assert.equal(delivery.command(room,100),null);
 delivery.observe({...room,round:2});assert.equal(delivery.command({...room,round:2},2000),null);
});
