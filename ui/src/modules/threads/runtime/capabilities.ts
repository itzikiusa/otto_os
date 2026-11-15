// Per-provider capabilities on the structured agent runtime, delegating the
// canonical runtime-provider list to `lib/providers`.

import type { RuntimeProvider } from '../../../lib/api/types.ts';
import {
  isRuntimeProvider,
  providerSupportsSteer,
  RUNTIME_PROVIDERS,
} from '../../../lib/providerRegistry.ts';

export { isRuntimeProvider, providerSupportsSteer, RUNTIME_PROVIDERS };
export type RuntimeProviderId = RuntimeProvider;
