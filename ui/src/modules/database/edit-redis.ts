// Redis edit adapter — a stub for now. `EditFlow.resolveTarget` still returns
// before consulting any adapter when the engine has no SQL capability, so this
// is wired (`adapterFor('redis')`) but unreachable until the Redis key editing
// work lifts that gate and fills the builders in.
import type { EditAdapter } from './edit-types';

export const redisAdapter: EditAdapter = {
  target() {
    return { target: null, reason: 'Redis results are read-only here' };
  },
  buildUpdate() {
    return null;
  },
  buildDelete() {
    return null;
  },
  buildInsert() {
    return null;
  },
  buildDuplicate() {
    return null;
  },
};
