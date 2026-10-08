import type {GameRoomCommand,GameRoomState} from '../../../lib/api/game-room-types';
export interface GameResult {winner:number|null;scores:number[];elapsed:number}
/** A successful send is not an acknowledgement; reconnect can invalidate its epoch. */
export class FinishDelivery {
 private result:GameResult|null=null;
 private round:number|null=null;
 private generation=0;
 private last=-Infinity;
 queue(result:GameResult):void {this.result??=result;}
 observe(room:GameRoomState):void {
  if(room.phase==='finished'||room.phase==='closed'||(this.round!==null&&room.round!==this.round)){this.result=null;this.round=null;this.last=-Infinity;}
 }
 command(room:GameRoomState,now:number):Extract<GameRoomCommand,{type:'finish'}>|null {
  this.observe(room);
  if(!this.result||room.phase!=='playing'||room.paused)return null;
  if(this.generation===room.generation&&now-this.last<1000)return null;
  this.round=room.round;this.generation=room.generation;this.last=now;
  return {type:'finish',round:room.round,generation:room.generation,result:this.result};
 }
}
