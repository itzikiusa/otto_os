import { untrack } from 'svelte';
import { api } from '../api/client';
import type { RuntimeDraftRecord } from '../api/types';
import { runtimeDrafts } from './runtimeDrafts.svelte';
import { DraftPersistence, composerContent } from './runtimeDraftPersistence';

export function installRuntimeDraftRecovery(): void {
  runtimeDrafts.configure((id, draft) => {
    let active = true;
    const wire = composerContent(draft);
    const path = `/runtime-sessions/${encodeURIComponent(id)}/draft`;
    const persistence = new DraftPersistence(wire, {
      load: () => api.get<RuntimeDraftRecord>(path),
      save: (value, revision) => api.put<RuntimeDraftRecord>(path, { revision, draft: value }),
    }, () => active && runtimeDrafts.owns(id, draft), (status, error) => {
      if (status === 'saved' || status === 'saving') draft.recovered = true;
      draft.saveStatus = status; draft.saveError = error ?? null;
    });
    draft.retrySave = () => { void persistence.retry(); };
    // The root outlives a view remount and is disposed by identity change/thread deletion.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => { clearTimeout(timer); void persistence.save(); };
    draft.flushSave = flush;
    window.addEventListener('pagehide', flush);
    const onVisibility = () => { if (document.visibilityState === 'hidden') flush(); };
    document.addEventListener('visibilitychange', onVisibility);
    const stop = $effect.root(() => {
      $effect(() => {
        // Track fields without repeatedly serializing multi-megabyte images.
        void [wire.text, wire.model, wire.effort, wire.access_mode, wire.fresh_context];
        wire.images.map(image => [image.name, image.mime, image.data_base64]);
        wire.context.map(chip => Object.values(chip));
        untrack(() => {
          clearTimeout(timer);
          if (draft.recovered && draft.saveStatus !== 'error') draft.saveStatus = 'saving';
          timer = setTimeout(flush, draft.recovered ? 200 : 0);
        });
      });
    });
    return () => {
      active = false; clearTimeout(timer); stop();
      window.removeEventListener('pagehide', flush);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  });
}
