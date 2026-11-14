import { untrack } from 'svelte';
import { api } from '../api/client';
import type { RuntimeDraftRecord } from '../api/types';
import { appLive } from '../live';
import { runtimeDrafts } from './runtimeDrafts.svelte';
import { DraftPersistence, composerContent } from './runtimeDraftPersistence';

const DRAFT_SYNC_EVENTS: ReadonlySet<string> = new Set([
  'provider_turn.updated',
  'turn.terminal',
  'provider_session.updated',
  'queue.updated',
]);

export function installRuntimeDraftRecovery(): void {
  runtimeDrafts.configure((id, draft) => {
    let active = true;
    const wire = composerContent(draft);
    const initialJson = JSON.stringify(wire);
    const path = `/runtime-sessions/${encodeURIComponent(id)}/draft`;
    const persistence = new DraftPersistence(wire, {
      load: () => api.get<RuntimeDraftRecord>(path),
      save: (value, revision) => api.put<RuntimeDraftRecord>(path, { revision, draft: value }),
    }, () => active && runtimeDrafts.owns(id, draft), (status, error) => {
      if (status === 'saved' || status === 'saving') draft.recovered = true;
      draft.saveStatus = status; draft.saveError = error ?? null;
    });
    draft.retrySave = () => { void persistence.retry(); };
    void persistence.load();
    // The root outlives a view remount and is disposed by identity change/thread deletion.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => { clearTimeout(timer); void persistence.save(); };
    const sync = () => { void persistence.syncIfClean(); };
    draft.flushSave = flush;
    if (typeof window !== 'undefined') {
      window.addEventListener('pagehide', flush);
      window.addEventListener('focus', sync);
    }
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') flush();
      else if (document.visibilityState === 'visible') sync();
    };
    if (typeof document !== 'undefined') {
      document.addEventListener('visibilitychange', onVisibility);
    }
    const offRuntime = appLive.on(['agent_runtime_event'], (ev) => {
      const rt = ev.event as { session_id?: string; type?: string } | undefined;
      if (rt?.session_id === id && (!rt.type || DRAFT_SYNC_EVENTS.has(rt.type))) sync();
    });
    const offResync = appLive.onResync(sync);
    const offConn = appLive.onConnection((up) => {
      if (up) sync();
    });
    let initialized = false;
    const stop = $effect.root(() => {
      $effect(() => {
        // Track fields without repeatedly serializing multi-megabyte images.
        void [wire.text, wire.model, wire.effort, wire.access_mode, wire.fresh_context];
        wire.images.map(image => [image.name, image.mime, image.data_base64]);
        wire.context.map(chip => Object.values(chip));
        untrack(() => {
          if (!initialized) {
            initialized = true;
            if (JSON.stringify(wire) === initialJson) return;
          }
          clearTimeout(timer);
          if (draft.recovered && draft.saveStatus !== 'error') draft.saveStatus = 'saving';
          timer = setTimeout(flush, draft.recovered ? 200 : 0);
        });
      });
    });
    return () => {
      active = false; clearTimeout(timer); stop();
      offRuntime(); offResync(); offConn();
      if (typeof window !== 'undefined') {
        window.removeEventListener('pagehide', flush);
        window.removeEventListener('focus', sync);
      }
      if (typeof document !== 'undefined') {
        document.removeEventListener('visibilitychange', onVisibility);
      }
    };
  });
}
