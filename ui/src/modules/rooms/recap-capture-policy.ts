/** Bounded, deterministic capture helpers independent of browser permissions. */
export function encodeMonoWav(input: Float32Array, sourceRate: number): Uint8Array {
  if (!Number.isFinite(sourceRate) || sourceRate < 8000 || sourceRate > 192000 || input.length > sourceRate * 30) throw new Error('Audio chunks must contain at most 30 seconds of supported PCM.');
  const rate = 16000, length = Math.floor(input.length * rate / sourceRate);
  const output = new Uint8Array(44 + length * 2), view = new DataView(output.buffer);
  const text = (offset: number, value: string) => { for (let i = 0; i < value.length; i++) output[offset + i] = value.charCodeAt(i); };
  text(0, 'RIFF'); view.setUint32(4, output.length - 8, true); text(8, 'WAVE'); text(12, 'fmt ');
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, rate, true); view.setUint32(28, rate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, 'data'); view.setUint32(40, length * 2, true);
  // Weighted box filtering prevents selecting just one sample out of each 48k→16k group.
  for (let i = 0; i < length; i++) {
    const start = i * sourceRate / rate, end = (i + 1) * sourceRate / rate;
    let sum = 0;
    for (let index = Math.floor(start); index < Math.ceil(end) && index < input.length; index++) {
      const weight = Math.min(end, index + 1) - Math.max(start, index);
      sum += (Number.isFinite(input[index]) ? input[index] : 0) * weight;
    }
    const value = Math.max(-1, Math.min(1, sum / (end - start)));
    view.setInt16(44 + i * 2, Math.round(value * (value < 0 ? 32768 : 32767)), true);
  }
  return output;
}
export function changedFrame(previous: Uint8Array | null, next: Uint8Array): boolean {
  if (!previous || previous.length !== next.length) return true;
  let difference = 0;
  for (let i = 0; i < next.length; i++) difference += Math.abs(previous[i] - next[i]);
  return difference / Math.max(1, next.length) >= 3;
}
interface Upload { epoch: number; run: (signal: AbortSignal) => Promise<void> }
export class CaptureUploadQueue {
  private epoch = -1;
  private waiting: Upload[] = [];
  private active: AbortController | null = null;
  private work: Promise<void> = Promise.resolve();
  private capacity: number;
  private gap: (reason: string) => void;
  constructor(capacity: number, gap: (reason: string) => void) { this.capacity = capacity; this.gap = gap; }
  reset(epoch: number): void { this.epoch = epoch; this.waiting = []; this.active?.abort(); }
  push(epoch: number, run: Upload['run']): boolean {
    if (epoch !== this.epoch || epoch < 0) return false;
    if (this.waiting.length >= this.capacity) { this.gap('Capture upload queue is full.'); return false; }
    this.waiting.push({epoch, run}); this.pump(); return true;
  }
  private pump(): void {
    if (this.active) return;
    const task = this.waiting.shift(); if (!task) return;
    const controller = new AbortController(); this.active = controller;
    this.work = task.run(controller.signal).catch(() => {
      if (!controller.signal.aborted && task.epoch === this.epoch) this.gap('A capture upload failed.');
    }).finally(() => { this.active = null; this.pump(); });
  }
  async drain(): Promise<void> { while (this.active || this.waiting.length) await this.work; }
  dispose(): void { this.reset(-1); }
}
