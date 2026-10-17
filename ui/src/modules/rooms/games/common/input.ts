/**
 * Keyboard / mouse / touch state for one canvas. Held keys are released when
 * the window blurs or the game pauses so nobody keeps driving while away.
 */
export class InputState {
  private held = new Set<string>();
  private pressed = new Set<string>();
  /** Accumulated pointer movement since the last `consumeLook()`. */
  private lookX = 0; private lookY = 0;
  wheel = 0;
  mouse = { left: false, right: false };
  private disposers: (() => void)[] = [];
  private canvas: HTMLElement;
  locked = false;

  constructor(canvas: HTMLElement, opts: { pointerLock: boolean; onLockChange?: (locked: boolean) => void }) {
    this.canvas = canvas;
    const on = <K extends keyof WindowEventMap>(target: Window | Document | HTMLElement, type: K | string, fn: (e: never) => void, options?: AddEventListenerOptions) => {
      target.addEventListener(type, fn as EventListener, options);
      this.disposers.push(() => target.removeEventListener(type, fn as EventListener, options));
    };
    on(window, 'keydown', (e: KeyboardEvent) => {
      if (!this.focused() || e.metaKey || e.ctrlKey && e.code !== 'ControlLeft') return;
      if (['Space', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Tab'].includes(e.code)) e.preventDefault();
      if (!this.held.has(e.code)) this.pressed.add(e.code);
      this.held.add(e.code);
    });
    on(window, 'keyup', (e: KeyboardEvent) => { this.held.delete(e.code); });
    on(window, 'blur', () => this.release());
    on(canvas, 'mousedown', (e: MouseEvent) => {
      canvas.focus();
      if (opts.pointerLock && !this.locked && e.button === 0) {
        try { void (canvas.requestPointerLock() as unknown as Promise<void> | undefined)?.catch?.(() => {}); } catch { /* drag-to-look still works */ }
      }
      if (e.button === 0) { this.mouse.left = true; this.pressed.add('Mouse0'); }
      if (e.button === 2) { this.mouse.right = true; this.pressed.add('Mouse2'); }
    });
    on(window, 'mouseup', (e: MouseEvent) => {
      if (e.button === 0) this.mouse.left = false;
      if (e.button === 2) this.mouse.right = false;
    });
    on(canvas, 'contextmenu', (e: MouseEvent) => e.preventDefault());
    on(window, 'mousemove', (e: MouseEvent) => {
      if (this.locked || (this.mouse.left && document.activeElement === canvas) || (this.mouse.right && document.activeElement === canvas)) {
        this.lookX += e.movementX; this.lookY += e.movementY;
      }
    });
    on(canvas, 'wheel', (e: WheelEvent) => { e.preventDefault(); this.wheel += Math.sign(e.deltaY); }, { passive: false });
    on(document, 'pointerlockchange', () => {
      this.locked = document.pointerLockElement === canvas;
      opts.onLockChange?.(this.locked);
      if (!this.locked) this.mouse.left = this.mouse.right = false;
    });
  }

  private focused(): boolean {
    const active = document.activeElement;
    if (this.locked || !active || active === document.body) return true;
    if (['INPUT', 'TEXTAREA', 'SELECT'].includes(active.tagName)) return false;
    return !!this.canvas.closest('.game-play')?.contains(active);
  }

  down(...codes: string[]): boolean { return codes.some(c => this.held.has(c)); }
  /** True once per key press (edge); cleared by `endFrame()`. */
  hit(...codes: string[]): boolean { return codes.some(c => this.pressed.has(c)); }
  /** Touch controls feed the same key codes. */
  touch(code: string, isDown: boolean): void {
    if (isDown) { if (!this.held.has(code)) this.pressed.add(code); this.held.add(code); }
    else this.held.delete(code);
  }
  look(dx: number, dy: number): void { this.lookX += dx; this.lookY += dy; }
  consumeLook(): [number, number] { const v: [number, number] = [this.lookX, this.lookY]; this.lookX = this.lookY = 0; return v; }
  endFrame(): void { this.pressed.clear(); this.wheel = 0; }
  release(): void {
    this.held.clear(); this.pressed.clear(); this.mouse.left = this.mouse.right = false; this.lookX = this.lookY = 0;
    if (this.locked) document.exitPointerLock?.();
  }
  dispose(): void { this.release(); for (const d of this.disposers) d(); this.disposers = []; }
}
