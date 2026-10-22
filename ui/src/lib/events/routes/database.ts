import { databaseStore } from './stores';
import type { EventRoutes } from './types';

export const databaseRoutes: EventRoutes = {
  // The DB Assistant agent session is live (turn start) → attach its shell in
  // the embedded DB Assistant panel (beside the query editor).
  db_assist_session_started: (parsed) =>
    databaseStore.use((database) =>
      database.setAssistSession(parsed.assist_id, parsed.connection_id, parsed.session_id),
    ),
  // Live proposed SQL/note from the DB Assistant agent → the panel's
  // read-only SQL block (Insert into editor / Run).
  db_assist_updated: (parsed) =>
    databaseStore.use((database) =>
      database.applyAssistUpdate(parsed.assist_id, parsed.connection_id, parsed.sql, parsed.note),
    ),
};
