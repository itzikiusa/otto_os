import {test} from 'node:test';
import assert from 'node:assert/strict';
import {inputFromKeys, clampPitch} from '../src/modules/rooms/games/input.ts';
test('diagonal movement has unit length and release produces neutral input', () => {
  const input = inputFromKeys(new Set(['KeyW', 'KeyD']), 1, 0, false);
  assert.ok(Math.abs(Math.hypot(input.moveX, input.moveZ) - 1) < 1e-6);
  assert.equal(input.yaw, 1);
  const neutral = inputFromKeys(new Set(), 1, 0, false);
  assert.equal(neutral.moveX, 0); assert.equal(neutral.moveZ, 0); assert.equal(neutral.fire, false);
});
test('camera pitch is finite and cannot flip over', () => {
  assert.equal(clampPitch(Infinity), 0);
  assert.ok(clampPitch(10) < Math.PI / 2);
  assert.ok(clampPitch(-10) > -Math.PI / 2);
});

test('releasing controls exits pointer lock so match overlays can receive clicks',async context=>{
 const {GameControls}=await import('../src/modules/rooms/games/input.ts');
 const canvas=Object.assign(new EventTarget(),{focus(){}}) as unknown as HTMLCanvasElement;
 const fakeDocument=Object.assign(new EventTarget(),{pointerLockElement:canvas,hidden:false,exitPointerLock(){this.pointerLockElement=null as unknown as HTMLCanvasElement;}});
 const priorDocument=Object.getOwnPropertyDescriptor(globalThis,'document'),priorWindow=Object.getOwnPropertyDescriptor(globalThis,'window');
 Object.defineProperty(globalThis,'document',{configurable:true,value:fakeDocument});Object.defineProperty(globalThis,'window',{configurable:true,value:new EventTarget()});
 context.after(()=>{if(priorDocument)Object.defineProperty(globalThis,'document',priorDocument);else Reflect.deleteProperty(globalThis,'document');if(priorWindow)Object.defineProperty(globalThis,'window',priorWindow);else Reflect.deleteProperty(globalThis,'window');});
 const controls=new GameControls(canvas,true,()=>{});controls.touch('Fire',true);controls.release();
 assert.equal(fakeDocument.pointerLockElement,null);assert.equal(controls.read().fire,false);controls.dispose();
});
