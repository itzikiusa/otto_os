// Council state (docs/features/threads-structured-runtime.md "Council"): the
// seats picked in the new-thread stage, the two council routes, and the
// council's ⋯ items. A council is a linked group of normal threads: seats
// open in the Split / Grid panes, and every follow-up is typed in a seat.

import { api } from '../../../lib/api/client';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { withQuote } from '../../../lib/quote/quote';
import { pluralNoun } from '../../../lib/plural';
import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
import { agentProviders, harnessLabel, providerReadiness, sessionProviderSlug } from '../../../lib/providers';
import { router } from '../../../lib/router.svelte';
import { auth } from '../../../lib/stores/auth.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import type { CouncilResp, CreateCouncilReq, RuntimeProviderOrSlug, RuntimeSession, SynthesizeCouncilReq } from '../../../lib/api/types';
import { messageThread } from '../relatedMessage';
import { fanOut, relayTargets } from '../threadRelay';
import { threads } from '../threadsStore.svelte';
import { answerOf } from './childThreads';
import { COUNCIL_MAX_SEATS, COUNCIL_MIN_SEATS, councilChipLabel, councilMembers, councilOf, toggleSeat, type CouncilSeat } from './council';
import { paneMessage } from './paneMessage.svelte';
import { askSeatsText, councilErrorText, seatLabel as runningSeatLabel, sideBySideIds, type SeatNamer } from './paneStrip';
import { asThreadSession, isRuntimeProvider, runtimeThreads } from './runtimeThreads.svelte';
import { runtimeViews } from './runtimeView.svelte';
import { stopThreadWork } from './taskStops.svelte';

class CouncilStore {
  /** "Ask several" is on in the new-thread stage. */
  on = $state(false);
  /** The seats picked there (2–4 to send). */
  seats = $state<CouncilSeat[]>([]);
  /** ⌘ / Shift was held on the last click in the model picker. */
  private multi = false;
  /** The stage's pick when the picker opened (a ⌘-click adds to it). */
  private base: CouncilSeat | null = null;

  /** Enough seats to send as a council. */
  get ready(): boolean {
    return this.on && this.seats.length >= COUNCIL_MIN_SEATS;
  }

  /** Turn "Ask several" on (seeded with the stage's pick) or off. */
  toggle(current: CouncilSeat): void {
    this.on = !this.on;
    this.seats = this.on && isRuntimeProvider(current.provider, auth.meta) ? [current] : [];
  }

  /** While the model picker is open, remember whether ⌘ / Shift is held on
   *  each click (the picker reports only the model). Returns the cleanup. */
  watchModifiers(current: CouncilSeat): () => void {
    this.base = current;
    const note = (e: PointerEvent | KeyboardEvent): void => {
      this.multi = e.metaKey || e.shiftKey;
    };
    window.addEventListener('pointerdown', note, true);
    window.addEventListener('keydown', note, true);
    return () => {
      window.removeEventListener('pointerdown', note, true);
      window.removeEventListener('keydown', note, true);
      this.multi = false;
    };
  }

  /** A model picked in the stage: with "Ask several" on (or ⌘ / Shift held)
   *  it is added to the seats, or taken out when already there. */
  pick(seat: CouncilSeat): void {
    if (!isRuntimeProvider(seat.provider, auth.meta)) return;
    if (!this.on) {
      if (!this.multi) return;
      this.on = true;
      this.seats = this.base && isRuntimeProvider(this.base.provider, auth.meta) ? [this.base] : [];
    }
    this.seats = toggleSeat(this.seats, seat);
  }

  remove(i: number): void {
    this.seats = this.seats.filter((_, j) => j !== i);
  }

  get full(): boolean {
    return this.seats.length >= COUNCIL_MAX_SEATS;
  }

  /** Start the council (all seats or none); returns its seats, seat 1 first.
   *  `panes` opens them side by side in Threads; `background` (⌘↵, the IDE)
   *  leaves the view as it is and toasts "Started N threads" with Show. */
  async start(
    workspaceId: string,
    req: { prompt: string; title: string; cwd: string | null; images?: CreateCouncilReq['images'] },
    open: 'panes' | 'background',
  ): Promise<RuntimeSession[]> {
    const body: CreateCouncilReq = {
      prompt: req.prompt,
      title: req.title,
      ...(req.cwd ? { cwd: req.cwd } : {}),
      seats: this.seats.map((s) => ({
        provider: s.provider as RuntimeProviderOrSlug,
        ...(s.model ? { model: s.model } : {}),
        ...(s.effort ? { effort: s.effort } : {}),
      })),
      ...(req.images?.length ? { images: req.images } : {}),
    };
    const resp = await api.post<CouncilResp>(`/workspaces/${workspaceId}/runtime-sessions/council`, body);
    for (const s of resp.seats) runtimeThreads.byId[s.id] = s;
    this.on = false;
    this.seats = [];
    const ids = resp.seats.map((x) => x.id);
    if (open === 'panes') threads.openGroup(ids);
    else toasts.push('success', `Started ${ids.length} threads`, undefined, 8000, { action: { label: 'Show', run: () => this.showInThreads(ids) } });
    return resp.seats;
  }

  /** Show seats `ids` side by side in Threads, from anywhere (the IDE). */
  showInThreads(ids: readonly string[]): void {
    threads.openGroup(ids);
    if (ids[0]) router.go(`threads/${ids[0]}`);
  }

  /** Open the council's judge (409 while a seat still works). */
  async synthesize(id: string, req: SynthesizeCouncilReq): Promise<RuntimeSession> {
    const judge = await api.post<RuntimeSession>(`/runtime-sessions/${id}/council/synthesize`, req);
    runtimeThreads.byId[judge.id] = judge;
    return judge;
  }
}

export const council = new CouncilStore();

/** How the stage names a seat: "Opus · High" / "Codex". */
export function seatLabel(seat: CouncilSeat, meta: Parameters<typeof formatModelLabel>[2]): string {
  const model = seat.model ? formatModelLabel(seat.provider, seat.model, meta) : harnessLabel(seat.provider);
  return seat.effort ? `${model} · ${formatEffortShort(seat.effort)}` : model;
}

/** How a running seat's model and effort read (paneStrip.ts `seatLabel`). */
export const seatNamer: SeatNamer = (provider, model, effort) => ({
  model: model ? formatModelLabel(provider, model, auth.meta) : harnessLabel(provider),
  effort: effort ? formatEffortShort(effort) : null,
});

/** "Seat 2 · Opus · High" for a council thread, null for any other. */
export function paneSeatLabel(id: string): string | null {
  const c = membersOf(id);
  const s = runtimeThreads.get(id);
  return c && s ? runningSeatLabel(s, c.seats, seatNamer) : null;
}

/** The agent chip while "Ask several" is on: "Opus, Sol, +1". */
export function councilAgentLabel(meta: Parameters<typeof formatModelLabel>[2]): string {
  return council.seats.length ? councilChipLabel(council.seats.map((s) => seatLabel(s, meta))) : 'Pick 2–4 agents';
}

/** The council `id` sits on, from the loaded threads. */
const membersOf = (id: string) => councilMembers(Object.values(runtimeThreads.byId), id);

/** "Ask every seat…": the same text to each open seat, queued behind a turn
 *  that still runs (one prompt per seat, like typing it in each); a stopped
 *  seat restarts to take it, and the sheet says so. */
export async function askEverySeat(id: string): Promise<void> {
  const seats = (membersOf(id)?.seats ?? []).filter((s) => s.status !== 'closed');
  if (!seats.length) return;
  const text = await paneMessage.ask({
    title: 'Ask every model',
    intro: `Send the same message to ${askSeatsText(seats)}, each as its next turn.`,
    confirmLabel: `Ask ${seats.length}`,
    placeholder: 'Follow-up question',
  });
  if (text) await fanOut(seats.map(asThreadSession), text, null, 'seat');
}

/** The agents that may judge `id`'s council: seat 1's first. */
export function judgeChoices(id: string): string[] {
  const c = membersOf(id);
  const first = c?.seats[0] ? sessionProviderSlug(c.seats[0]) : 'claude';
  return [first, ...agentProviders().filter((p) => p !== first && isRuntimeProvider(p, auth.meta))];
}

/** Synthesize with `provider`: the judge opens beside the seats (with four
 *  seats, beside the three most recent — sideBySideIds). */
export async function synthesizeWith(id: string, provider: RuntimeProviderOrSlug): Promise<void> {
  try {
    const judge = await council.synthesize(id, { provider });
    const c = membersOf(id);
    threads.openGroup(sideBySideIds(c?.seats ?? [], judge));
    threads.focusPane(judge.id);
  } catch (e) {
    toasts.error('Couldn’t compare the answers', councilErrorText(e));
  }
}

/** Stop every seat (and judge) of `ids` that still works or waits. */
export async function stopSeats(ids: readonly string[]): Promise<void> {
  const busy = (st?: string): boolean => st === 'working' || st === 'starting' || st === 'awaiting_permission';
  const r = await Promise.allSettled(
    ids.map((x) => stopThreadWork(x, { turnRunning: busy(runtimeThreads.get(x)?.status), taskIds: (runtimeThreads.get(x)?.background_tasks ?? []).map((b) => b.id) })),
  );
  const failed = r.find((x): x is PromiseRejectedResult => x.status === 'rejected');
  if (failed) toastError('Couldn’t stop every compared thread', failed.reason);
}

/** A thread's last answer: from its open conversation, else loaded briefly. */
async function answerFor(id: string): Promise<{ text: string; error?: string }> {
  if (runtimeViews.projections[id]) return { text: answerOf(runtimeViews.projections[id]) };
  const release = runtimeViews.open(id);
  try {
    for (let i = 0; i < 60 && !runtimeViews.projections[id] && !runtimeViews.loadError[id]; i++) {
      await new Promise((r) => setTimeout(r, 100));
    }
    if (!runtimeViews.projections[id] && runtimeViews.loadError[id]) {
      return { text: '', error: runtimeViews.loadError[id] };
    }
    return { text: answerOf(runtimeViews.projections[id]) };
  } finally {
    release();
  }
}

/** "Continue with this answer": the new-thread stage in the thread's folder,
 *  the answer quoted in its draft and a line left for the instruction.
 *  Nothing is sent until the person presses Send. */
export async function continueWithAnswer(id: string): Promise<void> {
  const s = runtimeThreads.get(id);
  const { text, error } = await answerFor(id);
  if (error) {
    toasts.error('Couldn’t load the answer', error);
    return;
  }
  if (!s || !text.trim()) {
    toasts.info('No answer to continue with yet', 'Wait for it to answer, then try again.');
    return;
  }
  threads.startNewThread({ workspaceId: s.workspace_id, cwd: s.cwd, draft: withQuote('', text) });
}

/** "Send to a thread ▸ <thread>": the shared Message sheet opens with the
 *  answer quoted (the person edits it and presses Send). */
async function sendAnswer(id: string, target: string): Promise<void> {
  const { text, error } = await answerFor(id);
  if (error) {
    toasts.error('Couldn’t load the answer', error);
    return;
  }
  if (!text.trim()) {
    toasts.info('No answer to send yet', 'Wait for it to answer, then try again.');
    return;
  }
  await messageThread(id, target, withQuote('', text));
}

/** A seat's or the synthesis's result actions (its chip menu, its ⋯). */
export function answerItems(id: string): MenuItem[] {
  const own = new Set([id, ...(membersOf(id)?.seats ?? []).map((s) => s.id)]);
  const targets = relayTargets(id).filter((t) => !own.has(t.id)).slice(0, 12);
  return [
    { label: 'Continue with this answer', icon: 'edit', action: () => void continueWithAnswer(id) },
    {
      label: 'Send to a thread',
      icon: 'send',
      disabled: targets.length === 0,
      title: targets.length ? undefined : 'No other open threads',
      children: targets.map((t) => ({ label: t.title, action: () => void sendAnswer(id, t.id) })),
    },
  ];
}

/** The council items of a council thread's ⋯ (none for any other thread). */
export function councilMenuItems(id: string): MenuItem[] {
  const c = membersOf(id);
  if (!c) return [];
  const ids = sideBySideIds(c.seats, c.synthesis);
  const busy = c.seats.filter((s) => s.status === 'working').map((s) => councilOf(s)?.seat);
  const why = busy.length ? `${busy.length === 1 ? 'One model is' : `${busy.length} models are`} still working` : undefined;
  return [
    { label: 'Open side by side', icon: 'grid', action: () => threads.openGroup(ids) },
    { label: 'Ask every model…', icon: 'send', action: () => void askEverySeat(id) },
    {
      label: 'Compare answers with',
      icon: 'sparkle',
      disabled: !!why,
      title: why,
      children: judgeChoices(id).map((p) => ({
        label: harnessLabel(p),
        disabled: !providerReadiness(p).available,
        action: () => void synthesizeWith(id, p as RuntimeProviderOrSlug),
      })),
    },
    ...answerItems(id),
  ];
}
