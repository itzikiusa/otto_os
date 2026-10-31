// Page-owned stores load on their FIRST event (perf F2): statically importing
// all of them made every document — main window, pop-out, side pane — parse
// and evaluate the database / apiClient / product / k8s / … stores before the
// shell could paint. The shell-owned stores (routes/shell.ts) stay static.
//
// Most handlers `peek()` (perf G2/H1): a page imports its store statically
// and loads its own data on mount, so a document that never opened the page
// has nothing to patch — `use()` there imported the store into every window
// and, for API history / Run with Otto / AWS, fetched per event. `use()` is
// kept only where the event's payload must survive until the page mounts
// (an agent session or proposal to attach); `unit/lazyModule.test.ts` holds
// that allow-list. Each feature's route file owns its loader; the ones the
// dispatcher also needs (resync, restart) are exported here.
import { lazyModule } from '../../lazyModule';

export const usageStore = lazyModule(() => import('../../api/usage.svelte').then((m) => m.usage), 'usage');
export const databaseStore = lazyModule(() => import('../../stores/database.svelte').then((m) => m.database), 'database');
export const personalAgentsStore = lazyModule(() => import('../../stores/personalAgents.svelte').then((m) => m.personalAgents), 'personalAgents');
