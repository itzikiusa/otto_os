// One concurrency gate for every DB dashboard tile in this document (Home's
// DbDashboardBox and the Dashboards page alike): at most two widget queries
// run at once, the rest queue (r3-04-03). Without it a 20-tile dashboard
// fired 20 parallel DB queries at every refresh.
import { createLimiter } from '../../lib/poll';

export const widgetGate = createLimiter(2);
