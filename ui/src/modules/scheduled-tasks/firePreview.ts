// The "Next fires" preview of the schedule form: one fire time's label, and
// the sentence a screen reader hears for a preview (the visible line is
// inserted with its text, which VoiceOver doesn't speak — the page passes
// this to announce()).

export interface FirePreview {
  times: string[];
  tz: string;
  error: string;
}

/** A fire time in the schedule's own timezone ("Mon, Oct 27, 09:00"). */
export function fireLabel(at: string, tz: string): string {
  try {
    return new Date(at).toLocaleString(undefined, { timeZone: tz, weekday: 'short', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  } catch {
    return new Date(at).toLocaleString();
  }
}

/** What the preview says, as one sentence: the error, the next three fires, or none. */
export function previewSpeech(p: FirePreview): string {
  if (p.error) return p.error;
  if (!p.times.length) return 'This schedule has no upcoming run.';
  return `Next fires: ${p.times.slice(0, 3).map((at) => fireLabel(at, p.tz)).join(', ')}`;
}
