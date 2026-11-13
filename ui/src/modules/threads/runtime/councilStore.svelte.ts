// Council state (docs/features/threads-structured-runtime.md "Council"): the
// seats picked in the new-thread stage, the two council routes, and which
// council threads a person chose to continue as a plain thread.

import { api } from '../../../lib/api/client';
import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
import { harnessLabel } from '../../../lib/providers';
import type { CouncilFollowUpReq, CouncilResp, CouncilRoundsResp, CreateCouncilReq, RuntimeProvider, RuntimeSession, SynthesizeCouncilReq } from '../../../lib/api/types';
import { threads } from '../threadsStore.svelte';
import { COUNCIL_MAX_SEATS, COUNCIL_MIN_SEATS, councilChipLabel, councilMembers, councilOf, toggleSeat, type CouncilSeat } from './council';
import { isRuntimeProvider, runtimeThreads } from './runtimeThreads.svelte';

class CouncilStore {
  /** "Ask several" is on in the new-thread stage. */
  on = $state(false);
  /** The seats picked there (2–4 to send). */
  seats = $state<CouncilSeat[]>([]);
  /** Sidebar: councils shown expanded into their seats, by council id. */
  expanded = $state<Record<string, true>>({});
  /** Council threads continued as a plain thread ("Continue here"), by id. */
  continued = $state<Record<string, true>>({});
  /** Each council's rounds (`GET …/council/rounds`), by council id. */
  rounds = $state<Record<string, CouncilRoundsResp>>({});
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
    this.seats = this.on && isRuntimeProvider(current.provider) ? [current] : [];
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
    if (!isRuntimeProvider(seat.provider)) return;
    if (!this.on) {
      if (!this.multi) return;
      this.on = true;
      this.seats = this.base && isRuntimeProvider(this.base.provider) ? [this.base] : [];
    }
    this.seats = toggleSeat(this.seats, seat);
  }

  remove(i: number): void {
    this.seats = this.seats.filter((_, j) => j !== i);
  }

  get full(): boolean {
    return this.seats.length >= COUNCIL_MAX_SEATS;
  }

  /** Start the council (all seats or none); returns its seats, seat 1 first. */
  async start(workspaceId: string, req: { prompt: string; title: string; cwd: string | null; images?: CreateCouncilReq['images'] }): Promise<RuntimeSession[]> {
    const body: CreateCouncilReq = {
      prompt: req.prompt,
      title: req.title,
      ...(req.cwd ? { cwd: req.cwd } : {}),
      seats: this.seats.map((s) => ({
        provider: s.provider as RuntimeProvider,
        ...(s.model ? { model: s.model } : {}),
        ...(s.effort ? { effort: s.effort } : {}),
      })),
      ...(req.images?.length ? { images: req.images } : {}),
    };
    const resp = await api.post<CouncilResp>(`/workspaces/${workspaceId}/runtime-sessions/council`, body);
    for (const s of resp.seats) runtimeThreads.byId[s.id] = s;
    this.on = false;
    this.seats = [];
    return resp.seats;
  }

  /** Open the council's judge (409 while a seat still works). */
  async synthesize(id: string, req: SynthesizeCouncilReq): Promise<RuntimeSession> {
    const judge = await api.post<RuntimeSession>(`/runtime-sessions/${id}/council/synthesize`, req);
    runtimeThreads.byId[judge.id] = judge;
    return judge;
  }

  /** Load the rounds of the council `id` sits on. */
  async loadRounds(councilId: string, id: string): Promise<CouncilRoundsResp> {
    const r = await api.get<CouncilRoundsResp>(`/runtime-sessions/${id}/council/rounds`);
    this.rounds = { ...this.rounds, [councilId]: r };
    return r;
  }

  /** A follow-up round (every seat when `seats` is empty); a seat still
   *  working queues it. Sent only from a click: nothing re-asks by itself. */
  async followUp(councilId: string, id: string, req: CouncilFollowUpReq): Promise<CouncilRoundsResp> {
    const r = await api.post<CouncilRoundsResp>(`/runtime-sessions/${id}/council/rounds`, req);
    this.rounds = { ...this.rounds, [councilId]: r };
    return r;
  }

  /** Threads shows `id` as a council (not a plain thread). */
  showsCouncil(id: string | null | undefined): boolean {
    return !!id && !!councilOf(runtimeThreads.get(id)) && !this.continued[id];
  }

  /** "Continue here": focus one seat (or the synthesis) as a plain thread. */
  continueHere(id: string): void {
    this.continued = { ...this.continued, [id]: true };
    threads.selectThread(id);
  }

  /** Back to the council view of `id`'s council. */
  showCouncil(id: string): void {
    const c = councilMembers(Object.values(runtimeThreads.byId), id);
    const ids = c ? [...c.seats, ...(c.synthesis ? [c.synthesis] : [])].map((s) => s.id) : [id];
    const next = { ...this.continued };
    for (const m of ids) delete next[m];
    this.continued = next;
    threads.selectThread(c?.seats[0]?.id ?? id);
  }

  toggleExpanded(councilId: string): void {
    const { [councilId]: was, ...rest } = this.expanded;
    this.expanded = was ? rest : { ...rest, [councilId]: true };
  }
}

export const council = new CouncilStore();

/** How the stage names a seat: "Opus · High" / "Codex". */
export function seatLabel(seat: CouncilSeat, meta: Parameters<typeof formatModelLabel>[2]): string {
  const model = seat.model ? formatModelLabel(seat.provider, seat.model, meta) : harnessLabel(seat.provider);
  return seat.effort ? `${model} · ${formatEffortShort(seat.effort)}` : model;
}

/** The agent chip while "Ask several" is on: "Opus, Sol, +1". */
export function councilAgentLabel(meta: Parameters<typeof formatModelLabel>[2]): string {
  return council.seats.length ? councilChipLabel(council.seats.map((s) => seatLabel(s, meta))) : 'Pick 2–4 agents';
}
