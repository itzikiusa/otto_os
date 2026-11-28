// "Open beside the content by default, as a sheet only when asked" (design.md
// §3 Detail pane): the open state a page hands to DetailPane as `control`.
// Inline the pane follows the page's preference `inline` (starts open, or a
// remembered value under `key`); presented as a sheet (no room, a phone, a
// phone on its side) it is closed until asked for (`asked`), and asking never
// changes the inline preference. A change of presentation forgets the ask.
//
//   const details = new DetailOpen();
//   <button aria-pressed={details.open} onclick={() => details.toggle()}>…</button>
//   <DetailPane control={details} module="design" title="Details">…</DetailPane>
import { lsGet, lsSet } from '../../storage';

export class DetailOpen {
  inline = $state(true);
  asked = $state(false);
  #sheet = $state(false);
  readonly #key: string | undefined;

  constructor(key?: string) {
    this.#key = key;
    if (key) this.inline = lsGet(key) !== '0';
  }

  /** Presented as a sheet right now (DetailPane reports it). */
  get sheet(): boolean {
    return this.#sheet;
  }
  present(sheet: boolean): void {
    if (sheet === this.#sheet) return;
    this.#sheet = sheet;
    this.asked = false;
  }

  get open(): boolean {
    return this.#sheet ? this.asked : this.inline;
  }
  set(open: boolean): void {
    if (this.#sheet) {
      this.asked = open;
      return;
    }
    this.inline = open;
    if (this.#key) lsSet(this.#key, open ? '1' : '0');
  }
  toggle(): void {
    this.set(!this.open);
  }
}
