// Display helpers for the Assistant module (dates, delivery targets). Pure —
// Intl only — so they stay node-testable next to model.ts.
import type { AssistantOrigin } from '../../lib/api/types';

/** "Thu 25 Sep, 09:00" (locale-aware); today → "Today, 09:00"; tomorrow → "Tomorrow, 09:00". */
export function whenLabel(iso: string, now: Date = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const time = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  const day = Math.round((startOfDay(d) - startOfDay(now)) / 86_400_000);
  if (day === 0) return `Today, ${time}`;
  if (day === 1) return `Tomorrow, ${time}`;
  const date = d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
  return `${date}, ${time}`;
}

/** Clock time only ("14:00"), for limit notices and message headers. */
export function clock(iso: string | null | undefined): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

const ORIGIN: Record<AssistantOrigin, string> = {
  app: 'here',
  thread: 'here',
  bar: 'to the floating bar',
  phone: 'to your phone',
  channel: 'to the chat it came from',
};

/** Reminders go back where they were asked for, plus a notification. */
export function deliverLabel(origin: AssistantOrigin | string): string {
  const where = ORIGIN[origin as AssistantOrigin] ?? 'here';
  return `Delivered ${where} + a notification`;
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}
