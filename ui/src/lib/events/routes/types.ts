// The typed WebSocket event route table (architecture-2-04). Each feature file
// in this directory exports a partial table for the event types it owns; the
// dispatcher in `lib/events.svelte.ts` merges them. `unit/eventRoutes.test.ts`
// fails when an `OttoEvent` type is neither routed nor listed in
// `LIVE_ONLY_EVENTS` (handled through `liveEvents` subscribers only).
import type { OttoEvent } from '../../api/types';

export type EventType = OttoEvent['type'];
export type EventOf<K extends EventType> = Extract<OttoEvent, { type: K }>;
/** One handler per event type, typed to that type's payload. */
export type EventRoutes = { [K in EventType]?: (ev: EventOf<K>) => void };
