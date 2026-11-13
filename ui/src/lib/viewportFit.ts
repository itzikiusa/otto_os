// Touch devices: keep the app inside the VISIBLE viewport. iOS (Safari and
// Chrome alike) does not shrink the layout viewport for the on-screen
// keyboard; it pans the page inside it, so focusing a bottom composer pushed
// the page's top (the Threads header) above the screen — and the pan often
// stayed after the keyboard closed. `#app` follows `visualViewport.height`
// (app.css `--app-h`) and any pan is undone, so the header stays put and the
// composer sits just above the keyboard. Pinch-zoom (scale ≠ 1) is left alone.

type VV = Pick<VisualViewport, 'height' | 'scale' | 'offsetTop' | 'addEventListener'>;

export function fitToVisualViewport(win: { visualViewport: VV | null; scrollY: number; scrollTo(x: number, y: number): void }, root: { style: { setProperty(k: string, v: string): void } }): void {
  const vv = win.visualViewport;
  if (!vv) return;
  const sync = (): void => {
    if (Math.abs(vv.scale - 1) > 0.01) return;
    root.style.setProperty('--app-h', `${Math.round(vv.height)}px`);
    if (win.scrollY !== 0 || vv.offsetTop !== 0) win.scrollTo(0, 0);
  };
  sync();
  vv.addEventListener('resize', sync);
  vv.addEventListener('scroll', sync);
}
