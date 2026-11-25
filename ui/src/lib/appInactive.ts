// Infinite CSS animations (a needs-you pulse, a working dot, spinners) keep
// the compositor drawing at the display rate even when nobody is looking:
// three pulsing dots alone held the GPU process at ~27% CPU in Chromium, and
// the app sat at 13-23% GPU all night behind other windows. While the window
// is hidden, or still unfocused 5 s after it lost focus, every INFINITE
// animation is paused (one that starts meanwhile too); a one-shot entrance
// (a toast, a modal, a menu) always plays to its end. Focus or visibility
// resumes them at once. A pane embedded in the main window (the side pane's
// iframe) loses focus whenever the user works next to it: it pauses only
// while hidden.
const BLUR_GRACE_MS = 5_000;

const infinite = (a: Animation): boolean => a.effect?.getComputedTiming().iterations === Infinity;

export function installAppInactive(doc: Document = document, win: Window = window): void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let inactive = false;
  let paused: Animation[] = [];
  const pause = (list: Animation[]): void => {
    for (const a of list) {
      if (a.playState !== 'running' || !infinite(a)) continue;
      a.pause();
      paused.push(a);
    }
  };
  const onStart = (e: Event): void => pause((e.target as Element).getAnimations?.() ?? []);
  const set = (on: boolean): void => {
    if (on === inactive) return;
    inactive = on;
    if (on) {
      pause(doc.getAnimations());
      doc.addEventListener('animationstart', onStart, true);
      return;
    }
    doc.removeEventListener('animationstart', onStart, true);
    // Only what is still paused: a dot removed meanwhile is gone for good.
    for (const a of paused) if (a.playState === 'paused') a.play();
    paused = [];
  };
  const embedded = win.parent !== win;
  const away = (): boolean => doc.hidden || (!embedded && !doc.hasFocus());
  const update = (): void => {
    clearTimeout(timer);
    if (doc.hidden) set(true);
    else if (!away()) set(false);
    else timer = setTimeout(() => set(away()), BLUR_GRACE_MS);
  };
  win.addEventListener('blur', update);
  win.addEventListener('focus', update);
  doc.addEventListener('visibilitychange', update);
  update(); // a window created hidden (the tray, the bar) starts paused
}
