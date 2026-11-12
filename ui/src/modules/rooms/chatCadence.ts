// The room Chat view's transcript poll cadence (RoomConversation.svelte).
// Rooms have no owner event stream, so the chat polls `/room-transcript/…`.
// A quiet session backs off — 2.5 s, then 5 s, then 10 s while nothing
// changes — and any activity (a new turn, the member sending or stopping)
// drops it back to 2.5 s. `ms` is a getter: lib/poll.ts re-reads it on every
// schedule.

export const CHAT_POLL_STEPS = [2500, 5000, 10_000] as const;

export class ChatCadence {
  #step = 0;

  /** The delay before the next poll. */
  get ms(): number {
    return CHAT_POLL_STEPS[this.#step];
  }

  /** A poll found the transcript as it was: wait longer next time. */
  unchanged(): void {
    this.#step = Math.min(this.#step + 1, CHAT_POLL_STEPS.length - 1);
  }

  /** Something happened (a new turn, a send): poll at the fastest cadence. */
  active(): void {
    this.#step = 0;
  }
}
