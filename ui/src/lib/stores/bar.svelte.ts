// Floating bar state shared by its two hosts (the in-app pill and the ⌥Space
// panel): the four spaces + their threads, the in-app visibility preference,
// and the ⌘K "focus the bar" request channel. Spaces persist per device and
// sync across windows through the `storage` event (same origin).

import {
  SPACES_KEY,
  clampSpace,
  parseBarPref,
  parseSpaces,
  patchTurn,
  pushTurn,
  serializeSpaces,
  type BarPref,
  type BarSpace,
  type BarSpaces,
  type BarTurn,
} from '../floatingBar';
import { lsGet, lsSet } from '../storage';

const PREF_KEY = 'otto_bar_pref';

class BarStore {
  state: BarSpaces = $state(parseSpaces(lsGet(SPACES_KEY)));
  /** In-app visibility: auto · pinned (always full) · docked · hidden. */
  pref: BarPref = $state(parseBarPref(lsGet(PREF_KEY)));
  /** An in-app bar is mounted and takes ⌘K (desktop width, not hidden). */
  mounted = $state(false);
  /** Bumped by ⌘K; the mounted bar toggles focus on change. */
  focusTick = $state(0);

  constructor() {
    if (typeof window === 'undefined') return;
    // Another window (the ⌥Space panel ↔ the main window) saved spaces:
    // adopt them, keeping this window's transient plan/confirm fields.
    window.addEventListener('storage', (e) => {
      if (e.key === PREF_KEY) this.pref = parseBarPref(e.newValue);
      if (e.key !== SPACES_KEY) return;
      const next = parseSpaces(e.newValue);
      next.spaces.forEach((sp, i) => {
        const local = this.state.spaces[i]?.thread ?? [];
        sp.thread = sp.thread.map((t) => {
          const mine = local.find((l) => l.id === t.id);
          return mine && (mine.plan || mine.closeIds || mine.tone === 'pending') ? mine : t;
        });
      });
      this.state = next;
    });
  }

  get active(): BarSpace {
    return this.state.spaces[this.state.active];
  }

  private save(): void {
    lsSet(SPACES_KEY, serializeSpaces(this.state));
  }

  setActive(i: number): void {
    this.state.active = clampSpace(i);
    this.save();
  }

  patchSpace(i: number, patch: Partial<Omit<BarSpace, 'thread'>>): void {
    const k = clampSpace(i);
    this.state.spaces[k] = { ...this.state.spaces[k], ...patch };
    this.save();
  }

  addTurn(i: number, turn: BarTurn): void {
    const k = clampSpace(i);
    this.state.spaces[k].thread = pushTurn(this.state.spaces[k].thread, turn);
    this.save();
  }

  updateTurn(i: number, id: string, patch: Partial<BarTurn>): void {
    const k = clampSpace(i);
    this.state.spaces[k].thread = patchTurn(this.state.spaces[k].thread, id, patch);
    this.save();
  }

  clearThread(i: number): void {
    this.state.spaces[clampSpace(i)].thread = [];
    this.save();
  }

  setPref(p: BarPref): void {
    this.pref = p;
    lsSet(PREF_KEY, p);
  }

  /** ⌘K: focus (or, when focused, close) the mounted in-app bar. */
  requestFocus(): void {
    this.focusTick += 1;
  }
}

export const barStore = new BarStore();
