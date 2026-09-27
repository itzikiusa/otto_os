import {test} from 'node:test';
import assert from 'node:assert/strict';
import {encodeMonoWav, changedFrame, CaptureUploadQueue} from '../src/modules/rooms/recap-capture-policy.ts';

test('encodes bounded mono16k PCM WAV with clamping and duration-preserving downsampling', () => {
  const wav = encodeMonoWav(new Float32Array([1, 1, 1, -1, -1, -1]), 48000);
  const view = new DataView(wav.buffer);
  assert.equal(new TextDecoder().decode(wav.subarray(0, 4)), 'RIFF');
  assert.equal(view.getUint32(24, true), 16000);
  assert.equal(view.getUint16(22, true), 1);
  assert.equal(view.getUint32(40, true), 4);
  assert.equal(view.getInt16(44, true), 32767);
  assert.equal(view.getInt16(46, true), -32768);
  assert.throws(() => encodeMonoWav(new Float32Array(16000 * 31), 16000));
  assert.throws(() => encodeMonoWav(new Float32Array(1), 0));
});
test('screen difference retains first/changed frames and skips small noise', () => {
  assert.equal(changedFrame(null, new Uint8Array([10, 10])), true);
  assert.equal(changedFrame(new Uint8Array([10, 10]), new Uint8Array([11, 11])), false);
  assert.equal(changedFrame(new Uint8Array([10, 10]), new Uint8Array([10, 100])), true);
});
test('bounded upload queue drops overflow and invalidates queued work when consent epoch changes', async () => {
  let release!: () => void;
  const seen: number[] = [], gaps: string[] = [];
  const queue = new CaptureUploadQueue(1, reason => gaps.push(reason));
  queue.reset(1);
  queue.push(1, async signal => { seen.push(1); await new Promise<void>(resolve => { release = resolve; signal.addEventListener('abort', () => resolve(), {once: true}); }); });
  queue.push(1, async () => { seen.push(2); });
  assert.equal(queue.push(1, async () => { seen.push(3); }), false);
  queue.reset(2);
  assert.equal(queue.push(1, async () => { seen.push(4); }), false);
  queue.push(2, async () => { seen.push(5); });
  release(); await queue.drain();
  assert.deepEqual(seen, [1, 5]);
  assert.equal(gaps.includes('Capture upload queue is full.'), true);
  queue.dispose();
});
