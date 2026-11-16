// A thread's model / effort / access change as the timeline's quiet row: the
// daemon records each accepted (or refused) switch as an ordinary
// `system_notice` whose id starts `selection:` — `title` names the setting
// ("Model" | "Effort" | "Access"), `message` is the line to show ("Model:
// opus → sonnet", "Model: bad was refused — still on sonnet"; `default` = the
// provider's own default). This file is the only place that knows the shape;
// any other notice returns null and renders as before. Pure
// (unit/threadsTaskStop.test.ts).

export interface SettingsChange {
  /** Which setting: "Model", "Effort", "Access" (or what the daemon names). */
  setting: string;
  /** The daemon's line, shown as is. */
  text: string;
  /** The switch was refused: the thread stayed on its old setting. */
  refused: boolean;
}

const SELECTION = 'selection:';

/** The change a timeline item records, or null when it records none. */
export function settingsChangeOf(item: unknown): SettingsChange | null {
  if (!item || typeof item !== 'object') return null;
  const o = item as { type?: unknown; id?: unknown; title?: unknown; message?: unknown };
  if (o.type !== 'system_notice' || typeof o.id !== 'string' || !o.id.startsWith(SELECTION)) return null;
  const text = typeof o.message === 'string' ? o.message.trim() : '';
  if (!text) return null;
  const setting = typeof o.title === 'string' && o.title.trim() ? o.title.trim() : (text.split(':')[0] ?? '').trim();
  return { setting, text, refused: /\bwas refused\b/i.test(text) };
}
