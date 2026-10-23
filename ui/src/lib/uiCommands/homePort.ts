// Home's layout store lives in the Home module (modules/home/home.svelte.ts),
// and lib/ never imports modules/. So the store binds itself to this port
// when it loads (`registerHomePort(home)` at the bottom of that file), and
// the `otto.ui_home_*` handlers (./home.ts) navigate to Home and then await
// the port — which also covers the Home chunk still loading after the
// navigation. Structural types only: the store is the one implementation.

import type { HomeBoxKind } from '../home/kinds';
import type { Feature } from '../api/types';
import { createPagePort } from './pagePort';

export interface HomePortBox {
  id: string;
  kind: HomeBoxKind;
  w: number;
  h: number;
  config: Record<string, unknown>;
}

export interface HomePortView {
  id: string;
  name: string;
  boxes: HomePortBox[];
}

/** What the handlers drive — the home store's public surface. */
export interface HomePort {
  readonly views: HomePortView[];
  readonly active: HomePortView | undefined | null;
  readonly activeIndex: number;
  readonly zoomedId: string | null;
  readonly autoRotate: boolean;
  /** Boxes one view can hold. */
  readonly maxBoxes: number;
  ensureDefault(can: (f: Feature) => boolean): void;
  goTo(i: number): void;
  toggleZoom(boxId: string): void;
  addBox(viewId: string, kind: HomeBoxKind, config?: Record<string, unknown>): HomePortBox | null;
  removeBox(viewId: string, boxId: string): void;
}

export const homePort = createPagePort<HomePort>('the Home layout');

/** Called once by the home store when its module loads. */
export function registerHomePort(store: HomePort): () => void {
  return homePort.bind(store);
}
