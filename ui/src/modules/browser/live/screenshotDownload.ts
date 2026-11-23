// Download a live Chromium tab screenshot (viewport or full-page PNG).
import * as liveApi from '../../../lib/api/browserLive';
import { downloadBlob } from '../../../lib/downloadBlob';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';

export async function downloadLiveScreenshot(
  tabId: string,
  pageUrl: string,
  mode: 'viewport' | 'full_page' = 'viewport',
  presetSuffix = '',
): Promise<void> {
  try {
    const blob = await liveApi.screenshotLive(tabId, { mode, format: 'png' });
    let host = 'page';
    try {
      host = new URL(pageUrl).hostname.replace(/[^a-z0-9.-]/gi, '_') || 'page';
    } catch {
      /* fallback */
    }
    const suffix = presetSuffix ? `-${presetSuffix}` : '';
    downloadBlob(blob, `otto-${host}${suffix}-${mode}-${Date.now()}.png`);
    toasts.success('Screenshot saved', `${mode === 'full_page' ? 'Full page' : 'Viewport'} PNG downloaded`);
  } catch (e) {
    toastError('Couldn’t capture screenshot', e);
  }
}
