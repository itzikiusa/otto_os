/**
 * Web Audio for the games: Kenney CC0 one-shots from /room-games/sfx plus a
 * synthesized engine drone for karts. Starts on the first user gesture; a
 * missing audio device leaves the game silent but playable.
 */
import { ASSET_ROOT } from './assets';

export class GameAudio {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private buffers = new Map<string, AudioBuffer>();
  private loading: Promise<void> | null = null;
  private engine: { osc: OscillatorNode; osc2: OscillatorNode; gain: GainNode; filter: BiquadFilterNode } | null = null;
  private last = new Map<string, number>();
  private muted = false;
  private names: string[];

  constructor(names: string[]) { this.names = names; }

  async start(): Promise<void> {
    if (!this.ctx) {
      const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctor) return;
      this.ctx = new Ctor();
      this.master = this.ctx.createGain();
      this.master.gain.value = this.muted ? 0 : 0.55;
      this.master.connect(this.ctx.destination);
    }
    if (this.ctx.state === 'suspended') await this.ctx.resume();
    this.loading ??= Promise.all(this.names.map(async name => {
      try {
        const response = await fetch(`${ASSET_ROOT}/sfx/${name}.ogg`);
        if (!response.ok || !this.ctx) return;
        this.buffers.set(name, await this.ctx.decodeAudioData(await response.arrayBuffer()));
      } catch { /* A sound that cannot decode is skipped; the game stays playable. */ }
    })).then(() => undefined);
    await this.loading;
  }

  /** Plays a one-shot. `distance` attenuates (0 = at the listener), `pan` -1..1. */
  play(name: string, { volume = 1, distance = 0, pan = 0, rate = 1, throttle = 0.04 } = {}): void {
    const ctx = this.ctx, buffer = this.buffers.get(name);
    if (!ctx || !buffer || !this.master || this.muted) return;
    const now = ctx.currentTime;
    if (now - (this.last.get(name) ?? -1) < throttle) return;
    this.last.set(name, now);
    const gain = volume / (1 + distance * distance * 0.004);
    if (gain < 0.02) return;
    const src = ctx.createBufferSource();
    src.buffer = buffer;
    src.playbackRate.value = rate;
    const g = ctx.createGain();
    g.gain.value = gain;
    const p = ctx.createStereoPanner();
    p.pan.value = Math.max(-1, Math.min(1, pan));
    src.connect(g).connect(p).connect(this.master);
    src.start();
  }

  /** Continuous kart engine; speed01 0..1 sets pitch, boost adds grit. */
  engineLevel(speed01: number, boost: boolean, active: boolean): void {
    const ctx = this.ctx;
    if (!ctx || !this.master) return;
    if (!this.engine) {
      const osc = ctx.createOscillator(), osc2 = ctx.createOscillator();
      osc.type = 'sawtooth'; osc2.type = 'square';
      const filter = ctx.createBiquadFilter();
      filter.type = 'lowpass'; filter.frequency.value = 600;
      const gain = ctx.createGain();
      gain.gain.value = 0;
      osc.connect(filter); osc2.connect(filter); filter.connect(gain).connect(this.master);
      osc.start(); osc2.start();
      this.engine = { osc, osc2, gain, filter };
    }
    const t = ctx.currentTime, e = this.engine;
    const s = Math.max(0, Math.min(1.4, speed01));
    e.osc.frequency.setTargetAtTime(48 + s * 95 + (boost ? 25 : 0), t, 0.08);
    e.osc2.frequency.setTargetAtTime(24 + s * 48, t, 0.08);
    e.filter.frequency.setTargetAtTime(380 + s * 900 + (boost ? 500 : 0), t, 0.1);
    e.gain.gain.setTargetAtTime(active && !this.muted ? 0.05 + s * 0.05 : 0, t, 0.12);
  }

  mute(muted: boolean): void {
    this.muted = muted;
    if (this.master && this.ctx) this.master.gain.setTargetAtTime(muted ? 0 : 0.55, this.ctx.currentTime, 0.05);
  }

  dispose(): void {
    if (this.engine) { try { this.engine.osc.stop(); this.engine.osc2.stop(); } catch { /* already stopped */ } }
    this.engine = null;
    void this.ctx?.close().catch(() => {});
    this.ctx = null;
    this.buffers.clear();
  }
}
