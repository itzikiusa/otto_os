// Play the whole scene: its spoken lines in the conversation's order over
// the place's sounds (looping) and the music (low, lower still under speech).
// The order is pure (unit-tested); ScenePlayer plays it with plain audio.
import type { ProductionItem } from '../../../lib/api/types';

export interface PlayLine {
  id: string;
  clipId: string;
  /** The character who says it (none for a narrator). */
  speaker?: string;
  text?: string;
}

export interface PlayBed {
  id: string;
  clipId: string;
  music: boolean;
  featured: boolean;
  loop: boolean;
}

/** Seconds of quiet before a line: a breath for the same speaker, longer when another speaks. */
export const gapBefore = (prev: PlayLine | undefined, next: PlayLine): number =>
  !prev ? 0.6 : prev.speaker && prev.speaker === next.speaker ? 0.3 : 0.7;

/** Music volume: low, lower still while someone speaks. */
export const musicVolume = (speaking: boolean, featured = false): number => speaking ? 0.15 : featured ? 0.8 : 0.35;

/**
 * The made lines in plan order, each after the line it answers (`replies_to`), and the
 * made sounds and music beside them.
 */
export function playOrder(items: ProductionItem[]): { lines: PlayLine[]; beds: PlayBed[] } {
  const made = (i: ProductionItem) => i.enabled && i.status === 'done' && !!i.result?.clip_id;
  const all = items.filter((i) => i.kind === 'voice_line' && made(i));
  const ids = new Set(all.map((i) => i.id));
  const lines: PlayLine[] = [];
  const placed = new Set<string>();
  let rest = all;
  while (rest.length) {
    // The first line whose answer-to is already said (or not a made line of this scene).
    const next = rest.find((i) => !i.details?.replies_to || !ids.has(i.details.replies_to) || placed.has(i.details.replies_to)) ?? rest[0];
    lines.push({ id: next.id, clipId: next.result!.clip_id!, speaker: next.details?.speaker, text: next.details?.text });
    placed.add(next.id);
    rest = rest.filter((i) => i !== next);
  }
  const beds = items
    .filter((i) => (i.kind === 'sound' || i.kind === 'music') && made(i))
    .map((i) => {
      const music = i.kind === 'music';
      const featured = music && i.details?.featured === true;
      return { id: i.id, clipId: i.result!.clip_id!, music, featured, loop: !featured && (music || !!i.details?.loop) };
    });
  return { lines, beds };
}

/** Every clip the scene plays, fetched ahead (a phone plays sound only from inside the tap itself). */
export async function prepareClips(order: { lines: PlayLine[]; beds: PlayBed[] }): Promise<Map<string, string>> {
  const { fetchTtsClipAudioUrl } = await import('../../../lib/api/media');
  const urls = new Map<string, string>();
  for (const c of [...order.beds, ...order.lines]) {
    if (!urls.has(c.clipId)) urls.set(c.clipId, await fetchTtsClipAudioUrl(c.clipId).catch(() => ''));
  }
  return urls;
}

/** Plays a scene's order once: beds start together, lines one after another; stop() ends it all. */
export class ScenePlayer {
  private beds: HTMLAudioElement[] = [];
  private voice: HTMLAudioElement | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private stopped = false;
  /** The voice played once inside the tap (muted), so a phone lets it speak later. */
  private unlocked: Promise<void> = Promise.resolve();
  private readonly onLine: (line: PlayLine | null) => void;
  private readonly onEnd: () => void;

  constructor(onLine: (line: PlayLine | null) => void, onEnd: () => void) {
    this.onLine = onLine;
    this.onEnd = onEnd;
  }

  /** Call from the tap: the sounds start now, the voice is unlocked now, the lines follow. */
  start(order: { lines: PlayLine[]; beds: PlayBed[] }, urls: Map<string, string>): void {
    const performances: Promise<void>[] = [];
    for (const b of order.beds) {
      const url = urls.get(b.clipId);
      if (!url) continue;
      const a = new Audio(url);
      a.loop = b.loop;
      a.volume = b.music ? musicVolume(false, b.featured) : 0.6;
      if (b.music) a.dataset.music = b.featured ? 'featured' : 'background';
      this.beds.push(a);
      if (b.featured) {
        performances.push(new Promise<void>((done) => {
          a.onended = a.onerror = () => done();
          void a.play().catch(() => done());
        }));
      } else {
        void a.play().catch(() => undefined);
      }
    }
    const lines = order.lines.filter((l) => urls.get(l.clipId));
    const voice = (this.voice = new Audio());
    if (lines[0]) {
      voice.src = urls.get(lines[0].clipId)!;
      voice.muted = true;
      this.unlocked = voice.play().then(
        () => {
          voice.pause();
          voice.currentTime = 0;
        },
        () => undefined,
      ).finally(() => (voice.muted = false));
    }
    void this.run(lines, urls, performances);
  }

  private wait(seconds: number): Promise<void> {
    return new Promise((done) => (this.timer = setTimeout(done, seconds * 1000)));
  }

  private duck(speaking: boolean): void {
    for (const a of this.beds) if (a.dataset.music) a.volume = musicVolume(speaking, a.dataset.music === 'featured');
  }

  private async run(lines: PlayLine[], urls: Map<string, string>, performances: Promise<void>[]): Promise<void> {
    let prev: PlayLine | undefined;
    await this.unlocked;
    for (const line of lines) {
      await this.wait(gapBefore(prev, line));
      const voice = this.voice;
      if (this.stopped || !voice) return;
      if (voice.src !== urls.get(line.clipId)) voice.src = urls.get(line.clipId)!;
      this.onLine(line);
      this.duck(true);
      await new Promise<void>((done) => {
        voice.onended = voice.onerror = () => done();
        void voice.play().catch(() => done());
      });
      this.duck(false);
      this.onLine(null);
      prev = line;
    }
    // The place goes on a moment after the last line, then the scene ends.
    if (performances.length) await Promise.all(performances);
    else await this.wait(lines.length ? 1.5 : 6);
    if (!this.stopped) this.finish();
  }

  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.finish();
  }

  private finish(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    for (const a of [...this.beds, this.voice]) a?.pause();
    this.beds = [];
    this.voice = null;
    this.onLine(null);
    this.onEnd();
  }
}
