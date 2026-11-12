// Debounced autosave of the artifact view's working copy. The view calls
// `schedule()` on every edit; ~1.5 s after the last one, `run` saves (a
// `PUT …/content` with the base version — the daemon labels it `autosave` and
// squashes runs of them). A save already in flight is never doubled: an edit
// during it re-arms the timer once it lands. `state` feeds the quiet
// "Saved / Saving… / Couldn’t save · Retry" indicator.
import { loadErrorText } from '../../lib/loadError';

export const AUTOSAVE_DELAY_MS = 1500;

export type AutosaveState = 'idle' | 'pending' | 'saving' | 'saved' | 'error';

export class Autosaver {
  state = $state<AutosaveState>('idle');
  error = $state<string | null>(null);
  #timer: ReturnType<typeof setTimeout> | null = null;
  #again = false;
  #run: () => Promise<void>;
  #delay: number;

  /** `run` throws on failure (its message is shown beside Retry). */
  constructor(run: () => Promise<void>, delay = AUTOSAVE_DELAY_MS) {
    this.#run = run;
    this.#delay = delay;
  }

  schedule(): void {
    this.#clear();
    if (this.state === 'saving') {
      this.#again = true;
      return;
    }
    this.state = 'pending';
    this.#timer = setTimeout(() => void this.flush(), this.#delay);
  }

  /** Drop a pending save (discard, a version switch, leaving the design). */
  cancel(): void {
    this.#clear();
    this.#again = false;
    if (this.state === 'pending') this.state = 'idle';
  }

  /** Save now (the debounce fired, Retry, or ⌘S). */
  async flush(): Promise<void> {
    this.#clear();
    if (this.state === 'saving') {
      this.#again = true;
      return;
    }
    this.state = 'saving';
    try {
      await this.#run();
      this.state = 'saved';
      this.error = null;
    } catch (e) {
      this.state = 'error';
      this.error = loadErrorText(e);
    }
    if (this.#again) {
      this.#again = false;
      this.schedule();
    }
  }

  #clear(): void {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
  }
}
