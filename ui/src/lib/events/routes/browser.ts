import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const browserStore = lazyModule(() => import('../../stores/browser.svelte').then((m) => m.browser), 'browser');
const browserLiveStore = lazyModule(() => import('../../stores/browserLive.svelte').then((m) => m.browserLive), 'browserLive');

export const browserRoutes: EventRoutes = {
  // Browser page: tab strip / annotation list refresh in place.
  browser_tab_updated: (parsed) => browserStore.peek()?.applyEvent(parsed),
  browser_annotation_added: (parsed) => browserStore.peek()?.applyEvent(parsed),
  // Browser page / Settings → Browser: Chromium download progress.
  browser_engine_install_updated: (parsed) => browserLiveStore.peek()?.applyEvent(parsed),
};
