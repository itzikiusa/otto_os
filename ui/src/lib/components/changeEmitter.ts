// Coalesced `onchange` for the shared CodeEditor (perf SB-08 / DB5-06).
//
// Emitting the full document on every edit costs a `doc.toString()` (a copy of
// the whole buffer) plus whatever the caller does with it (a new draft object,
// variable scans, persistence) PER KEYSTROKE. That is noise below a few
// hundred KB, but on a multi-MB body it is the typing cost. So: small docs
// emit synchronously (unchanged contract — every binding sees every edit);
// docs at/above `threshold` chars mark the change pending and emit ONCE after
// `delayMs` of quiet. The text is read lazily at emit time, so a burst costs a
// single copy.
//
// Callers that must never see a stale value flush first: the editor flushes on
// blur, on a ⌘/Ctrl-chord keydown (Send / Run / Save shortcuts), before its
// own submit handler, before switching documents and on teardown.

export interface ChangeEmitterOptions {
  /** Doc length (chars) at/above which emission is deferred. */
  threshold: number;
  /** Quiet period before a deferred emit. */
  delayMs: number;
  /** Read the current text; `null` when there is nothing to read any more. */
  read: () => string | null;
  /** Deliver the text to the caller. */
  emit: (value: string) => void;
  /** Timer hooks (tests inject fakes). */
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
}

export interface ChangeEmitter {
  /** The doc changed; `length` is its new length. */
  changed(length: number): void;
  /** Emit a pending change now (no-op when nothing is pending). */
  flush(): void;
  /** Drop a pending change without emitting it. */
  cancel(): void;
  readonly pending: boolean;
}

export function createChangeEmitter(o: ChangeEmitterOptions): ChangeEmitter {
  const setTimer = o.setTimer ?? ((fn: () => void, ms: number) => setTimeout(fn, ms));
  const clearTimer = o.clearTimer ?? ((h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>));
  let pending = false;
  let timer: unknown = null;

  function stopTimer(): void {
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
  }

  function emitNow(): void {
    stopTimer();
    pending = false;
    const value = o.read();
    if (value !== null) o.emit(value);
  }

  return {
    changed(length: number): void {
      if (length < o.threshold) {
        // Small doc: synchronous, exactly as before (also supersedes any
        // pending deferred emit — the text read now includes it).
        emitNow();
        return;
      }
      pending = true;
      stopTimer();
      timer = setTimer(() => {
        timer = null;
        if (pending) emitNow();
      }, o.delayMs);
    },
    flush(): void {
      if (pending) emitNow();
      else stopTimer();
    },
    cancel(): void {
      stopTimer();
      pending = false;
    },
    get pending() {
      return pending;
    },
  };
}
