import type { GameInput } from './types.ts';

export function clampPitch(value: number): number {
  return Number.isFinite(value) ? Math.max(-1.35, Math.min(1.35, value)) : 0;
}
export function inputFromKeys(keys: ReadonlySet<string>, yaw: number, pitch: number, fire: boolean): GameInput {
  let moveX = Number(keys.has('KeyD') || keys.has('ArrowRight')) - Number(keys.has('KeyA') || keys.has('ArrowLeft'));
  let moveZ = Number(keys.has('KeyW') || keys.has('ArrowUp')) - Number(keys.has('KeyS') || keys.has('ArrowDown'));
  const length = Math.max(1, Math.hypot(moveX, moveZ)); moveX /= length; moveZ /= length;
  return {moveX, moveZ, yaw, pitch: clampPitch(pitch), fire, weapon: keys.has('Digit1')?1:keys.has('Digit2')?2:keys.has('Digit3')?3:0, reload: keys.has('KeyR'), jump: keys.has('Space'), sprint: keys.has('ShiftLeft') || keys.has('ShiftRight'), drift: keys.has('Space'), item: keys.has('KeyE'), reset: keys.has('KeyR')};
}

/** Canvas-scoped controls. Losing focus releases every held input immediately. */
export class GameControls {
  private readonly keys = new Set<string>();
  private fire = false;
  private dragging = false;
  private active = false;
  private aim = false;
  private abort = new AbortController();
  yaw = 0;
  pitch = 0;
  private canvas: HTMLCanvasElement;
  private shooter: boolean;
  private changed: (active: boolean) => void;
  constructor(canvas: HTMLCanvasElement, shooter: boolean, changed: (active: boolean) => void, toggleCamera?: () => void) {
    this.canvas = canvas; this.shooter = shooter; this.changed = changed;
    const signal = this.abort.signal;
    canvas.addEventListener('contextmenu', e => e.preventDefault(), {signal});
    canvas.addEventListener('pointerdown', e => {
      if (e.pointerType === 'touch') return;
      canvas.focus(); this.active = true; this.changed(true);
      if (e.button === 2) this.aim = true;
      if (e.button === 0) { this.fire = true; this.dragging = true; }
      if (this.shooter && document.pointerLockElement !== canvas) {
        try { const locked = canvas.requestPointerLock?.(); if (locked) void locked.catch(() => {}); } catch { /* Drag-to-look is available in webviews without pointer lock. */ }
      }
    }, {signal});
    window.addEventListener('pointerup', e => { if (e.button === 0) { this.fire = false; this.dragging = false; } if (e.button === 2) this.aim = false; }, {signal});
    document.addEventListener('pointerlockchange', () => {
      if (document.pointerLockElement !== canvas) this.release();
    }, {signal});
    window.addEventListener('pointermove', e => {
      if (!this.shooter || !this.active || (document.pointerLockElement !== canvas && !this.dragging)) return;
      this.yaw -= e.movementX * (this.aim ? .0013 : .0024);
      this.pitch = clampPitch(this.pitch - e.movementY * (this.aim ? .0013 : .0024));
    }, {signal});
    canvas.addEventListener('keydown', e => {
      if (e.code === 'KeyV' && !e.repeat) { e.preventDefault(); toggleCamera?.(); return; }
      if (e.code === 'Escape') { this.release(); return; }
      if (!['KeyW','KeyA','KeyS','KeyD','ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Space','ShiftLeft','ShiftRight','KeyE','KeyR','Digit1','Digit2','Digit3'].includes(e.code)) return;
      e.preventDefault(); this.active = true; this.changed(true); this.keys.add(e.code);
    }, {signal});
    window.addEventListener('keyup', e => this.keys.delete(e.code), {signal});
    canvas.addEventListener('blur', () => this.release(), {signal});
    window.addEventListener('blur', () => this.release(), {signal});
    document.addEventListener('visibilitychange', () => { if (document.hidden) this.release(); }, {signal});
  }
  read(): GameInput { return inputFromKeys(this.keys, this.yaw, this.pitch, this.fire); }
  aiming(): boolean { return this.aim; }
  touch(code: string, pressed: boolean): void { this.active = true; this.changed(true); if (code === 'Fire') this.fire = pressed; else if (pressed) this.keys.add(code); else this.keys.delete(code); }
  look(dx: number, dy: number): void { this.yaw -= dx * .007; this.pitch = clampPitch(this.pitch - dy * .007); }
  release(): void { this.keys.clear(); this.fire = false; this.aim = false; this.dragging = false; this.active = false; this.changed(false); if (document.pointerLockElement === this.canvas) document.exitPointerLock(); }
  dispose(): void { this.release(); this.abort.abort(); if (document.pointerLockElement === this.canvas) document.exitPointerLock(); }
}
