// Moved to lib/k8s/resourcePoll.ts (shared code the k8s store imports must not live
// under modules/). Re-exported here so module importers keep working.
export * from '../../lib/k8s/resourcePoll.ts';
