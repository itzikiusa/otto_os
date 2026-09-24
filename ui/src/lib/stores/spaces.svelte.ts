// Spaces 01–04: the one shared notion of "which space am I in" for the Home
// desktop (its up-to-four views) and the floating "type or speak" bar. The
// floating bar owns the four spaces (lib/stores/bar.svelte.ts: name,
// workspace, agent, thread; `otto_bar_spaces`, synced across windows); this is
// the thin view Home uses of them, so Home's 01–04 and the bar's 01–04 are the
// same spaces:
//
//   • the ACTIVE space is the bar's — Home shows view N while the bar is on N
//     (clamped to the views Home has), and switching on Home switches the bar;
//   • Home's view names ARE the space names — Home publishes them here, and a
//     rename in the bar comes back to Home (HomePage.svelte).
//
// Home still owns what a space contains (modules/home/home.svelte.ts).

import { barStore } from './bar.svelte';
import { MAX_SPACE_NAME, SPACE_COUNT, clampSpace, spaceLabel } from '../floatingBar';

export { SPACE_COUNT };

/** `0` → `"01"`: how a space is numbered everywhere (Home, the bar). */
export const spaceNumber = spaceLabel;

class SpacesView {
  /** Active space, 0-based. */
  get active(): number {
    return barStore.state.active;
  }

  /** The four space names, in order. */
  get names(): string[] {
    return barStore.state.spaces.map((s) => s.name);
  }

  /** Label for space `i`: its name, else "Space 01". */
  label(i: number): string {
    return this.names[i] ?? `Space ${spaceNumber(i)}`;
  }

  select(i: number): void {
    if (clampSpace(i) !== barStore.state.active) barStore.setActive(i);
  }

  /** Home publishes its view names: space i takes view i's name. Spaces Home
   *  has no view for keep theirs. */
  setNames(names: string[]): void {
    names.slice(0, SPACE_COUNT).forEach((raw, i) => {
      const name = raw.trim().slice(0, MAX_SPACE_NAME);
      if (name && barStore.state.spaces[i] && barStore.state.spaces[i].name !== name) {
        barStore.patchSpace(i, { name });
      }
    });
  }
}

export const spaces = new SpacesView();
