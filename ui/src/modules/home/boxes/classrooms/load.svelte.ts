// Otto School — the Classrooms box's data (ClassroomsBox): the cross-workspace
// `GET /sessions?archived=false` (incl. engine rows) plus the last archived
// ones for the detention bench, filtered to what this device shows. The box
// overlays the workspace store's live rows on top; this only owns the fetch.
import { api } from '../../../../lib/api/client';
import { visibleOnThisDevice } from '../../../../lib/stores/workspace.svelte';
import { loadErrorText } from '../../../../lib/loadError';
import type { Session } from '../../../../lib/api/types';

export class SchoolData {
  fetched = $state.raw<Session[]>([]);
  archivedRows = $state.raw<Session[]>([]);
  /** A load has finished (success or a real failure — never an abort). */
  loaded = $state(false);
  error = $state('');

  /** One refresh. `true` = fresh data; `false` = nothing loaded (a failure, or
   *  an abort — the box went off screen mid-request — which `freshness` must
   *  not stamp fresh, so coming back fetches at once, as K8sBox does). */
  load = async (signal?: AbortSignal): Promise<boolean> => {
    let aborted = false;
    try {
      const [rows, arch] = await Promise.all([
        api.bg.get<Session[]>('/sessions?archived=false&limit=1000', signal),
        api.bg.get<Session[]>('/sessions?archived=true&limit=60', signal).catch(() => [] as Session[]),
      ]);
      this.fetched = rows.filter(visibleOnThisDevice);
      this.archivedRows = arch.filter(visibleOnThisDevice);
      this.error = '';
      return true;
    } catch (e) {
      if (signal?.aborted) {
        aborted = true;
        return false;
      }
      this.error = loadErrorText(e);
      return false;
    } finally {
      // An aborted first load keeps the skeleton up: it loaded nothing.
      if (!aborted) this.loaded = true;
    }
  };
}
