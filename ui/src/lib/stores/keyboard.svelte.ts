// The on-screen keyboard, reactive. lib/viewportFit.ts measures it (touch
// devices only — main.ts installs it on a coarse pointer); this store mirrors
// its state for components. CSS can use `html[data-kb='open']` and `--kb-h`
// instead. On a desktop it stays closed.
import { onKeyboard, type KeyboardState } from '../viewportFit';

class KeyboardStore {
  /** The on-screen keyboard is up. */
  open = $state(false);
  /** The visible area's height in CSS px (0 until measured / on a desktop). */
  visibleHeight = $state(0);
  /** The keyboard's height in CSS px; 0 when closed. */
  inset = $state(0);

  constructor() {
    onKeyboard((s: KeyboardState) => {
      this.open = s.open;
      this.visibleHeight = s.visibleHeight;
      this.inset = s.inset;
    });
  }
}

export const keyboard = new KeyboardStore();
