// Snip orchestration — the one-gesture screenshot → annotate → clipboard flow.
//
// `startSnip()` is the single entry point behind every trigger (global
// shortcut via the Tauri `otto://snip` event, in-app ⌘⇧S, ⌘K palette, menu
// item): it long-polls the daemon's interactive capture, then opens the
// annotation editor — a dedicated Tauri window when running in the desktop
// shell, an in-window `#/snip/{id}` route in a plain browser. The daemon has
// already put the capture on the clipboard by the time the response lands.

import { api, authedBlobUrl } from './api/client';
import type { CaptureSnipResp, Snip, SnipCopyResp } from './api/types';
import { router } from './router.svelte';
import { toasts } from './toast.svelte';
import { isTauri } from './stores/ui.svelte';

export const snipApi = {
  capture(): Promise<CaptureSnipResp> {
    return api.post<CaptureSnipResp>('/snips/capture', {});
  },
  upload(data_b64: string, filename?: string): Promise<Snip> {
    return api.post<Snip>('/snips', { data_b64, filename });
  },
  list(): Promise<Snip[]> {
    return api.get<Snip[]>('/snips');
  },
  saveAnnotated(id: string, data_b64: string): Promise<SnipCopyResp> {
    return api.post<SnipCopyResp>(`/snips/${id}/annotated`, { data_b64 });
  },
  copy(id: string): Promise<SnipCopyResp> {
    return api.post<SnipCopyResp>(`/snips/${id}/copy`, {});
  },
  remove(id: string): Promise<void> {
    return api.del<void>(`/snips/${id}`);
  },
  /** Object URL for the original PNG — caller revokes when done. */
  imageUrl(id: string): Promise<string> {
    return authedBlobUrl(`/snips/${id}/image`);
  },
};

/** Open the editor for an existing snip (Tauri: its own window; else route). */
export async function openSnipEditor(id: string): Promise<void> {
  if (isTauri) {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_snip_window', { snipId: id });
      return;
    } catch {
      // Older shell without the command — fall through to in-window routing.
    }
  }
  router.go(`snip/${id}`);
}

let capturing = false;

/** Trigger the capture flow. Re-entrancy-guarded client-side (the daemon also
 *  409s a concurrent capture — e.g. one started from another window). */
export async function startSnip(): Promise<void> {
  if (capturing) return;
  capturing = true;
  try {
    const resp = await snipApi.capture();
    if (resp.cancelled || !resp.snip) return; // Esc — silent, like the native tool
    await openSnipEditor(resp.snip.id);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (msg.includes('already in progress')) {
      toasts.warn('A screen capture is already in progress');
    } else {
      toasts.error('Screen capture failed', msg);
    }
  } finally {
    capturing = false;
  }
}
