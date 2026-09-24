// Minimal fuzzy matcher for the ⌘K palette: subsequence match with a score
// favouring word-boundary and consecutive hits. Implemented in commandSearch.ts
// (import-free, unit-tested); re-exported here for existing callers.
export { fuzzyMatch, type FuzzyResult } from './commandSearch';
