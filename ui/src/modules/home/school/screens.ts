// Otto School — what each PC monitor shows. A kid's screen is its session's
// LIVE terminal (`GET /sessions/{id}/screen`, plain rows), painted onto a 2D
// canvas the scene uses as the monitor's texture. Pure painting + a small
// poller; no three.js. The palette is the screen's own (a terminal is dark in
// both themes), so it carries literal colours like the terminal theme does.
//
//  • Polling: only kids in the classroom you're in, only the ones the camera
//    can see, at most MAX_LIVE of them, every POLL_MS — and none at all while
//    the widget is off screen. A row that didn't change isn't repainted.
//  • Empty desks show a sleeping screen; a suspended / ended session a dimmed
//    "asleep" screen; a kid that needs you gets an amber banner.

import type { CharacterKey, Pose } from './model.ts';

export const SCREEN_W = 512;
export const SCREEN_H = 320;
export const MAX_LIVE = 12;
export const POLL_MS = 2000;
/** Visible text rows on a monitor. */
export const ROWS = 17;

export interface ScreenFeed {
  live: boolean;
  lines: string[];
}

export interface ScreenInfo {
  title: string;
  provider: string;
  character: CharacterKey;
  pose: Pose | 'empty';
  feed: ScreenFeed | null;
  selected?: boolean;
}

/* Terminal palette (a screen is a terminal: dark in both app themes). */
const BG = '#0d1117';
const FG = '#c9d1d9';
const DIM = '#6e7681';
const GREEN = '#3fb950';
const RED = '#f85149';
const AMBER = '#d29922';
const BLUE = '#58a6ff';
const WHITE = '#f0f6fc';
const BAR: Record<CharacterKey, string> = {
  claude: '#c96442',
  codex: '#4f8a3c',
  grok: '#26282c',
  agy: '#3b6fd8',
  shell: '#5b6470',
  custom: '#2f8f8a',
};

/** Colour for one terminal row (cheap heuristics that read well at a glance). */
export function lineColor(line: string): string {
  const t = line.trimStart();
  if (/^(\+|✓|✔)/.test(t) || /\b(passed|success|ok)\b/i.test(t)) return GREEN;
  if (/^(-|✗|✘)/.test(t) || /\b(error|failed|panic)\b/i.test(t)) return RED;
  if (/^(>|❯|\$|%|›)/.test(t)) return BLUE;
  if (/^(⏺|●|•)/.test(t)) return WHITE;
  if (/^(⎿|│|└|├|╰|╭)/.test(t)) return DIM;
  return FG;
}

/** The last `rows` non-blank-tail lines, each clipped to `cols` chars. */
export function visibleLines(lines: readonly string[], rows = ROWS, cols = 58): string[] {
  let end = lines.length;
  while (end > 0 && !lines[end - 1].trim()) end--;
  return lines.slice(Math.max(0, end - rows), end).map((l) => {
    const chars = [...l.replace(/\t/g, '  ')];
    return chars.length > cols ? `${chars.slice(0, cols - 1).join('')}…` : chars.join('');
  });
}

/** Stable key: repaint only when what the screen shows changed. */
export function screenKey(s: ScreenInfo): string {
  return [s.pose, s.title, s.selected ? 1 : 0, s.feed?.live ? 1 : 0, ...(s.feed ? visibleLines(s.feed.lines) : [])].join('\u0001');
}

function clip(ctx: CanvasRenderingContext2D, text: string, max: number): string {
  if (ctx.measureText(text).width <= max) return text;
  let t = text;
  while (t.length > 1 && ctx.measureText(`${t}…`).width > max) t = t.slice(0, -1);
  return `${t}…`;
}

/** Paint one monitor. `ctx` is a SCREEN_W × SCREEN_H canvas. */
export function paintScreen(ctx: CanvasRenderingContext2D, s: ScreenInfo): void {
  const W = SCREEN_W;
  const H = SCREEN_H;
  ctx.save();
  ctx.fillStyle = BG;
  ctx.fillRect(0, 0, W, H);

  if (s.pose === 'empty' || s.pose === 'away') {
    // Sleeping screen: soft gradient + a drifting logo + zzz.
    const g = ctx.createLinearGradient(0, 0, W, H);
    g.addColorStop(0, '#101826');
    g.addColorStop(1, '#1d2b45');
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, W, H);
    ctx.fillStyle = 'rgba(255,255,255,0.55)';
    ctx.font = '600 44px ui-rounded, -apple-system, system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText('otto', W / 2, H / 2 + 6);
    ctx.font = '22px -apple-system, system-ui, sans-serif';
    ctx.fillStyle = 'rgba(255,255,255,0.35)';
    ctx.fillText(s.pose === 'empty' ? 'free desk' : 'z z z', W / 2, H / 2 + 46);
    if (s.pose === 'away') {
      ctx.font = '16px -apple-system, system-ui, sans-serif';
      ctx.fillText(clip(ctx, s.title, W - 40), W / 2, H - 22);
    }
    ctx.restore();
    return;
  }

  // Title bar in the provider's colour.
  ctx.fillStyle = BAR[s.character];
  ctx.fillRect(0, 0, W, 34);
  ctx.fillStyle = '#ffffff';
  ctx.font = '600 17px -apple-system, system-ui, sans-serif';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(clip(ctx, s.title, W - 120), 12, 17);
  ctx.textAlign = 'right';
  ctx.font = '13px -apple-system, system-ui, sans-serif';
  ctx.fillStyle = 'rgba(255,255,255,0.85)';
  ctx.fillText(s.pose === 'working' ? '● working' : s.pose === 'needs-you' ? '✋ needs you' : s.pose === 'stale' ? 'reconnecting' : 'idle', W - 12, 17);

  let top = 46;
  if (s.pose === 'needs-you') {
    ctx.fillStyle = AMBER;
    ctx.fillRect(0, 34, W, 26);
    ctx.fillStyle = '#1a1300';
    ctx.textAlign = 'center';
    ctx.font = '600 15px -apple-system, system-ui, sans-serif';
    ctx.fillText('Waiting for you — click to answer', W / 2, 47);
    top = 70;
  }

  ctx.textAlign = 'left';
  ctx.textBaseline = 'top';
  ctx.font = '14px ui-monospace, SFMono-Regular, Menlo, monospace';
  const lines = s.feed ? visibleLines(s.feed.lines, Math.floor((H - top - 8) / 16.2)) : [];
  if (lines.length === 0) {
    ctx.fillStyle = DIM;
    ctx.fillText(s.feed && !s.feed.live ? 'session is not running' : 'connecting…', 12, top);
  }
  lines.forEach((l, i) => {
    ctx.fillStyle = lineColor(l);
    ctx.fillText(l, 12, top + i * 16.2);
  });
  if (s.pose === 'working') {
    // A blinking-ish cursor block after the last line.
    ctx.fillStyle = GREEN;
    ctx.fillRect(12, top + lines.length * 16.2 + 2, 8, 13);
  }
  if (s.selected) {
    ctx.strokeStyle = BLUE;
    ctx.lineWidth = 6;
    ctx.strokeRect(3, 3, W - 6, H - 6);
  }
  ctx.restore();
}

// ── Poller ───────────────────────────────────────────────────────────────

export interface ScreenPollDeps {
  fetch(id: string, signal: AbortSignal): Promise<ScreenFeed>;
  /** Ids worth polling right now (visible live kids in the open room). */
  wanted(): string[];
  onFeed(id: string, feed: ScreenFeed): void;
}

/** Polls the wanted screens every POLL_MS (one request per screen, in
 *  parallel, never overlapping itself). `stop()` aborts in-flight reads. */
export function pollScreens(deps: ScreenPollDeps, every = POLL_MS): { stop(): void; now(): void } {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let ctrl: AbortController | null = null;
  let busy = false;
  let resume = false;
  const doc = typeof document === 'undefined' ? null : document;
  const visible = () => !doc?.hidden;
  const cancelTimer = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  const schedule = (delay: number) => {
    cancelTimer();
    if (!stopped && visible()) timer = setTimeout(() => void tick(), delay);
  };
  const tick = async () => {
    if (stopped || busy || !visible()) return;
    cancelTimer();
    busy = true;
    resume = false;
    const batch = new AbortController();
    ctrl = batch;
    const ids = deps.wanted().slice(0, MAX_LIVE);
    await Promise.all(
      ids.map(async (id) => {
        try {
          const f = await deps.fetch(id, batch.signal);
          if (!stopped && !batch.signal.aborted && visible()) deps.onFeed(id, f);
        } catch {
          /* a failed read keeps the last frame; the next tick retries */
        }
      }),
    );
    busy = false;
    ctrl = null;
    schedule(resume ? 0 : every);
  };
  const visibilityChanged = () => {
    if (stopped) return;
    cancelTimer();
    if (!visible()) {
      resume = false;
      ctrl?.abort();
    } else if (busy) {
      // An abort-insensitive transport must settle before a fresh batch.
      resume = true;
    } else schedule(0);
  };
  doc?.addEventListener('visibilitychange', visibilityChanged);
  schedule(0);
  return {
    stop() {
      stopped = true;
      cancelTimer();
      ctrl?.abort();
      doc?.removeEventListener('visibilitychange', visibilityChanged);
    },
    now() {
      if (stopped || busy || !visible()) return;
      cancelTimer();
      void tick();
    },
  };
}
