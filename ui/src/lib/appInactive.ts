// Infinite CSS animations (a needs-you pulse, a working dot, spinners) keep
// the compositor drawing at the display rate even when nobody is looking:
// three pulsing dots alone held the GPU process at ~27% CPU in Chromium, and
// the app sat at 13-23% GPU all night behind other windows. While the window
// is hidden, or 5 s after it lost focus, `html.app-inactive` pauses every
// animation (app.css); focus or visibility brings them back at once.
const BLUR_GRACE_MS = 5_000;

export function installAppInactive(doc: Document = document, win: Window = window): void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const set = (on: boolean): void => void doc.documentElement.classList.toggle('app-inactive', on);
  const update = (): void => {
    clearTimeout(timer);
    if (doc.hidden) set(true);
    else if (doc.hasFocus()) set(false);
    else timer = setTimeout(() => set(true), BLUR_GRACE_MS);
  };
  win.addEventListener('blur', update);
  win.addEventListener('focus', update);
  doc.addEventListener('visibilitychange', update);
}
