// The phone composer's on-device trace (the "screen jumps" / "first letter
// jumps" reports from a real phone, which no emulator reproduced). Off unless
// THIS device turned it on: ⌘K (phone: More ▸ Commands) ▸ "Turn on the
// composer trace" sets localStorage `otto.debug.composerTrace` = '1' and
// reloads. Only then does main.ts load the recorder
// (./composerTraceRecorder.ts: the ring buffer, its listeners and the
// Send / Copy / Clear bar); until then `trace()` is a no-op.
// Callers pass positions, sizes, lengths, short reason tags and elements
// (described by tag and class) — never typed text.

export const COMPOSER_TRACE_FLAG = 'otto.debug.composerTrace';

type Sink = (kind: string, data?: Record<string, unknown>) => void;
let sink: Sink | null = null;

/** Record one event while the trace is on. */
export function trace(kind: string, data?: Record<string, unknown>): void {
  sink?.(kind, data);
}

export function setTraceSink(f: Sink | null): void {
  sink = f;
}

/** The trace is switched on for this device. */
export function composerTraceOn(): boolean {
  try {
    return globalThis.localStorage?.getItem(COMPOSER_TRACE_FLAG) === '1';
  } catch {
    return false;
  }
}

/** Flip the switch and reload, so the listeners start (or are gone) from a clean page. */
export function toggleComposerTrace(): void {
  if (composerTraceOn()) localStorage.removeItem(COMPOSER_TRACE_FLAG);
  else localStorage.setItem(COMPOSER_TRACE_FLAG, '1');
  location.reload();
}
