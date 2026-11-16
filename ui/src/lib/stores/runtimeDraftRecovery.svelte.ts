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

/** Each draft root saves edits (debounced) for as long as it is in memory;
 *  its live sync (window focus, reconnect, this thread's turn events) runs
 *  only while a view shows the thread (`activate`, from runtimeDrafts.hold),
 *  so leaving a thread stops its GETs. An idle draft leaving memory is saved
 *  first (`dispose(true)`). */
export function installRuntimeDraftRecovery(): void {
  runtimeDrafts.configure((id, draft, after) => {
    let active = true;
    /** Evicted while its last save drains: owns it though no longer listed. */
    let draining = false;
    const epoch = runtimeDrafts.epoch;
    const wire = composerContent(draft);
    const initialJson = JSON.stringify(wire);
    const path = `/runtime-sessions/${encodeURIComponent(id)}/draft`;
    const persistence = new DraftPersistence(wire, {
      load: () => api.get<RuntimeDraftRecord>(path),
      save: (value, revision) => api.put<RuntimeDraftRecord>(path, { revision, draft: value }),
    }, () => active && (runtimeDrafts.owns(id, draft) || (draining && runtimeDrafts.epoch === epoch)), (status, error) => {
      if (status === 'saved' || status === 'saving') draft.recovered = true;
      draft.saveStatus = status; draft.saveError = error ?? null;
    });
    draft.retrySave = () => { void persistence.retry(); };
    // A draft evicted a moment ago may still be saving: read after that lands.
    if (after) void after.then(() => { if (active) void persistence.load(); });
    else void persistence.load();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => { clearTimeout(timer); void persistence.save(); };
    const sync = () => { void persistence.syncIfClean(); };
    draft.flushSave = flush;
    if (typeof window !== 'undefined') window.addEventListener('pagehide', flush);
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') flush();
      else if (document.visibilityState === 'visible' && runtimeDrafts.held(id)) sync();
    };
    if (typeof document !== 'undefined') {
      document.addEventListener('visibilitychange', onVisibility);
    }
    let initialized = false;
    const stop = $effect.root(() => {
      $effect(() => {
        // Track fields by reference/scalar without per-keystroke array/object allocations.
        void [
          wire.text,
          wire.model,
          wire.model_default,
          wire.effort,
          wire.access_mode,
          wire.fresh_context,
          wire.images,
          wire.images.length,
          wire.context,
          wire.context.length,
        ];
        untrack(() => {
          if (!initialized) {
            initialized = true;
            if (JSON.stringify(wire) === initialJson) return;
          }
          if (draft.saveStatus !== 'saving') draft.saveStatus = 'saving';
          clearTimeout(timer);
          timer = setTimeout(flush, draft.recovered ? 450 : 0);
        });
      });
    });
    const detach = () => {
      clearTimeout(timer); stop();
      if (typeof window !== 'undefined') window.removeEventListener('pagehide', flush);
      if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibility);
    };
    return {
      activate: () => {
        if (typeof window !== 'undefined') window.addEventListener('focus', sync);
        const offRuntime = appLive.on(['agent_runtime_event'], (ev) => {
          const rt = ev.event as { session_id?: string; type?: string } | undefined;
          if (rt?.session_id === id && (!rt.type || DRAFT_SYNC_EVENTS.has(rt.type))) sync();
        });
        const offResync = appLive.onResync(sync);
        const offConn = appLive.onConnection((up) => {
          if (up) sync();
        });
        // Back on a thread: pick up what another window did meanwhile.
        sync();
        return () => {
          offRuntime(); offResync(); offConn();
          if (typeof window !== 'undefined') window.removeEventListener('focus', sync);
        };
      },
      dispose: (flushFirst = false) => {
        detach();
        if (!flushFirst) {
          active = false;
          return;
        }
        draining = true;
        return persistence.save().finally(() => { active = false; draining = false; });
      },
    };
  });
}
