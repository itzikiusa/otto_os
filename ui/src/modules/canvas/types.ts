// Moved to lib/canvas/types.ts (the canvas store in lib/ imports these, and
// lib must not import from modules/ — architecture-1-13). Re-exported so the
// module importers keep working.
export type * from '../../lib/canvas/types';
