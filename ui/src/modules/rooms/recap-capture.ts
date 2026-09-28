import type {RoomSnapshot, RoomPresentation} from '../../lib/api/room-types';
import type {RoomRecapAudioSource} from './room-media';
import {CaptureUploadQueue, changedFrame, encodeMonoWav} from './recap-capture-policy.ts';
import {recapRequest} from './recap-client';
interface AudioSlot {source: RoomRecapAudioSource; input: MediaStreamAudioSourceNode; processor: AudioWorkletNode; offset: number; flushed?: () => void; closing?: Promise<void>}
interface ScreenSlot {generation: number; video: HTMLVideoElement; fingerprint: Uint8Array | null; sampled: number; busy: boolean; created: number}
export interface RecapCaptureStatus {audioSources: number; screenSources: number; gaps: string[]}
export class RoomRecapCapture {
  private room: RoomSnapshot | null = null;
  private epoch = -1;
  private context: AudioContext | null = null;
  private preparedContext: AudioContext | null = null;
  private finishingEpoch = -1;
  private preparing = false;
  private workletReady = false;
  private speechEnabled = true;
  private audio = new Map<string, AudioSlot>();
  private screens = new Map<string, ScreenSlot>();
  private sequences = new Map<string, number>();
  private timer: ReturnType<typeof setInterval> | undefined;
  private queue: CaptureUploadQueue;
  private status: RecapCaptureStatus = {audioSources: 0, screenSources: 0, gaps: []};
  private reported = new Set<string>();
  private sources: () => ReadonlyMap<string, RoomRecapAudioSource>;
  private streams: () => ReadonlyMap<string, MediaStream>;
  private onStatus: (status: RecapCaptureStatus) => void;
  constructor(sources: () => ReadonlyMap<string, RoomRecapAudioSource>, streams: () => ReadonlyMap<string, MediaStream>, onStatus: (status: RecapCaptureStatus) => void) {
    this.sources = sources; this.streams = streams; this.onStatus = onStatus;
    this.queue = new CaptureUploadQueue(4, reason => this.gap('capture', reason));
  }
  update(room: RoomSnapshot, connected: boolean, speechEnabled = true): void {
    const speechBecameAvailable = speechEnabled && !this.speechEnabled;
    this.speechEnabled = speechEnabled;
    this.room = room;
    const next = connected && room.member_id === room.host_member_id && room.recap?.state === 'capturing' ? room.recap.epoch : -1;
    if (next === this.finishingEpoch && next >= 0) return;
    if (next !== this.epoch) {
      this.finishingEpoch = -1;
      this.reset(); this.epoch = next; this.queue.reset(next); this.sequences.clear(); this.reported.clear();
      if (next >= 0) { void this.resumeAudio(); this.timer = setInterval(() => this.sync(), 500); }
    }
    if (next >= 0) { if (speechBecameAvailable) void this.resumeAudio(); this.sync(); }
  }
  /** Unlock audio from the host's click, without tapping any tracks before consent. */
  prepareAudio(): void {
    if (!this.speechEnabled || this.preparedContext) return;
    this.preparedContext = new AudioContext();
    void this.preparedContext.resume().catch(() => {});
  }
  async resumeAudio(): Promise<void> {
    if (this.epoch < 0 || this.preparing || !this.speechEnabled) return;
    if (this.context?.state === 'running' && this.workletReady) { this.sync(); return; }
    const epoch = this.epoch; this.preparing = true;
    try {
      const context = this.context ?? this.preparedContext ?? new AudioContext(); this.preparedContext = null; this.context = context;
      if (!context.audioWorklet) throw new Error('This browser cannot capture a spoken transcript.');
      await context.audioWorklet.addModule(new URL('/room-recap-worklet.js', location.href).href);
      await Promise.race([context.resume(), new Promise<never>((_, reject) => setTimeout(() => reject(new Error('Choose Retry speech capture to allow local audio processing.')), 2000))]);
      if (this.epoch !== epoch) { await context.close(); return; }
      this.workletReady = true; this.sync();
    } catch (e) { if (this.epoch === epoch) this.gap('audio', e instanceof Error ? e.message : 'Speech capture is unavailable.'); }
    finally { this.preparing = false; }
  }
  private startTime(): number { return Date.parse(this.room?.recap?.started_at ?? '') || Date.now(); }
  private next(key: string): number { const seq = (this.sequences.get(key) ?? 0) + 1; this.sequences.set(key, seq); return seq; }
  private sync(): void {
    if (this.epoch < 0 || this.epoch === this.finishingEpoch || !this.room) return;
    const available = this.speechEnabled ? this.sources() : new Map<string, RoomRecapAudioSource>();
    for (const [id, slot] of this.audio) if (available.get(id)?.stream !== slot.source.stream || available.get(id)?.generation !== slot.source.generation) {
      const member = this.room.members?.find(member => member.id === id);
      if (member?.connected && member.admission === 'admitted' && member.generation === slot.source.generation) void this.flushAudio(id, slot);
      else this.stopAudio(id);
    }
    if (this.context?.state === 'running' && this.workletReady) for (const [id, source] of available) if (!this.audio.has(id)) {
      const processor = new AudioWorkletNode(this.context, 'room-recap-pcm');
      const input = this.context.createMediaStreamSource(source.stream);
      const slot: AudioSlot = {source, input, processor, offset: Math.max(0, Date.now() - this.startTime())};
      const epoch = this.epoch; this.audio.set(id, slot);
      processor.port.onmessage = event => {
        if (event.data.flushed) { slot.flushed?.(); return; }
        if (this.epoch !== epoch || this.audio.get(id) !== slot || !(event.data.samples instanceof Float32Array)) return;
        const wav = encodeMonoWav(event.data.samples, event.data.sampleRate as number);
        const duration = Math.round((wav.length - 44) / 32); if (!duration) return;
        const offset = slot.offset; slot.offset += duration;
        this.upload('audio', {capture_epoch: epoch, member_id: id, member_generation: source.generation, sequence: this.next(`audio:${id}`), offset_ms: Math.round(offset), duration_ms: duration, wav_base64: base64(wav)}, epoch);
      };
      input.connect(processor); processor.connect(this.context.destination);
    }
    const streams = this.streams();
    const sources = this.room.presentations ?? [];
    for (const [id, slot] of this.screens) if (!sources.some(s => s.id === id && s.generation === slot.generation) || !streams.has(id)) { slot.video.pause(); slot.video.srcObject = null; this.screens.delete(id); }
    for (const source of sources) {
      const stream = streams.get(source.id);
      if (!stream) { this.gap('screen', `No frame available for ${source.title}; hidden or disconnected sources may not be sampled.`, source.id); continue; }
      let slot = this.screens.get(source.id);
      if (!slot) {
        const video = document.createElement('video'); video.muted = true; video.playsInline = true; video.srcObject = stream; void video.play().catch(() => {});
        slot = {generation: source.generation, video, fingerprint: null, sampled: 0, busy: false, created: Date.now()}; this.screens.set(source.id, slot);
      }
      if (slot.video.readyState < 2 && Date.now() - slot.created > 5000) this.gap('screen', `No decoded frame is available for ${source.title}.`, source.id);
      if (!slot.busy && Date.now() - slot.sampled >= 15000 && slot.video.readyState >= 2) void this.sample(source, slot);
    }
    this.status.audioSources = this.audio.size; this.status.screenSources = this.screens.size; this.emit();
  }
  private async sample(source: RoomPresentation, slot: ScreenSlot): Promise<void> {
    const epoch = this.epoch; slot.busy = true; slot.sampled = Date.now();
    try {
      const video = slot.video;
      const factor = Math.min(1, 1280 / video.videoWidth, 720 / video.videoHeight);
      const canvas = document.createElement('canvas'); canvas.width = Math.max(1, Math.round(video.videoWidth * factor)); canvas.height = Math.max(1, Math.round(video.videoHeight * factor));
      const ctx = canvas.getContext('2d')!; ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      const tiny = document.createElement('canvas'); tiny.width = 32; tiny.height = 18;
      const tc = tiny.getContext('2d')!; tc.drawImage(canvas, 0, 0, 32, 18);
      const pixels = tc.getImageData(0, 0, 32, 18).data; const fingerprint = new Uint8Array(32 * 18);
      for (let i = 0; i < fingerprint.length; i++) fingerprint[i] = Math.round((pixels[i * 4] + pixels[i * 4 + 1] + pixels[i * 4 + 2]) / 3);
      if (!changedFrame(slot.fingerprint, fingerprint)) return;
      let blob: Blob | null = null;
      for (const quality of [.65, .45, .25]) { blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', quality)); if (blob && blob.size <= 256 * 1024) break; }
      if (!blob || blob.size > 256 * 1024) { this.gap('screen', 'A screen sample exceeded the capture size limit.', source.id); return; }
      const bytes = new Uint8Array(await blob.arrayBuffer());
      if (this.epoch !== epoch || this.screens.get(source.id) !== slot) return;
      this.upload('screen', {capture_epoch: epoch, source_id: source.id, source_generation: source.generation, sequence: this.next(`screen:${source.id}`), offset_ms: Math.max(0, slot.sampled - this.startTime()), jpeg_base64: base64(bytes)}, epoch, () => { if (this.epoch === epoch && this.screens.get(source.id) === slot) slot.fingerprint = fingerprint; });
    } catch { if (this.epoch === epoch) this.gap('screen', 'A shared-screen sample could not be captured.', source.id); }
    finally { slot.busy = false; }
  }
  private upload(kind: 'audio' | 'screen', body: unknown, epoch: number, accepted?: () => void): void {
    const id = this.room?.recap?.id; if (!id) return;
    this.queue.push(epoch, async signal => { await recapRequest(`/room-recaps/${id}/${kind}`, body, signal); if (!signal.aborted) accepted?.(); });
  }
  private gap(kind: string, reason: string, sourceId?: string): void {
    const key = `${kind}:${sourceId ?? ''}:${reason}`; if (this.reported.has(key)) return; this.reported.add(key);
    this.status.gaps = [...this.status.gaps, reason].slice(-10); this.emit();
    const recap = this.room?.recap;
    if (this.epoch >= 0 && recap) void recapRequest(`/room-recaps/${recap.id}/gap`, {capture_epoch: this.epoch, kind, source_id: sourceId ?? null, member_id: null, reason}).catch(() => {});
  }
  private emit(): void { this.onStatus({...this.status, gaps: [...this.status.gaps]}); }
  private stopAudio(id: string): void { const slot = this.audio.get(id); if (!slot) return; slot.input.disconnect(); slot.processor.disconnect(); slot.processor.port.close(); this.audio.delete(id); slot.flushed?.(); }
  private flushAudio(id: string, slot: AudioSlot): Promise<void> {
    if (slot.closing) return slot.closing;
    slot.input.disconnect();
    slot.closing = new Promise<void>(resolve => {
      const timer = setTimeout(() => { this.gap('audio', 'A final microphone chunk could not be flushed.'); this.stopAudio(id); resolve(); }, 1000);
      slot.flushed = () => { clearTimeout(timer); slot.flushed = undefined; if (this.audio.get(id) === slot) this.stopAudio(id); resolve(); };
      slot.processor.port.postMessage('flush');
    });
    return slot.closing;
  }
  cancelCapture(): void {
    this.finishingEpoch = this.epoch;
    this.epoch = -1; this.queue.reset(-1); this.reset();
  }
  async finish(): Promise<void> {
    clearInterval(this.timer);
    this.finishingEpoch = this.epoch;
    const flushes = [...this.audio].map(([id, slot]) => this.flushAudio(id, slot));
    await Promise.race([Promise.all(flushes).then(() => this.queue.drain()), new Promise(resolve => setTimeout(resolve, 2500))]);
    this.reset(); this.epoch = -1; this.queue.reset(-1);
  }
  private reset(): void {
    clearInterval(this.timer); this.workletReady = false;
    for (const id of this.audio.keys()) this.stopAudio(id);
    for (const slot of this.screens.values()) { slot.video.pause(); slot.video.srcObject = null; }
    this.screens.clear();
    if (this.context) { void this.context.close().catch(() => {}); this.context = null; }
    this.status.audioSources = 0; this.status.screenSources = 0; this.emit();
  }
  dispose(): void { if (this.preparedContext) void this.preparedContext.close().catch(() => {}); this.preparedContext = null; this.epoch = -1; this.queue.dispose(); this.reset(); }
}
function base64(bytes: Uint8Array): string { let value = ''; for (let i = 0; i < bytes.length; i += 8192) value += String.fromCharCode(...bytes.subarray(i, i + 8192)); return btoa(value); }
