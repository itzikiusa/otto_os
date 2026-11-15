// Per-provider capabilities on the structured agent runtime, delegating the
// canonical runtime-provider list to `lib/providers`.

import type { RuntimeProvider } from '../../../lib/api/types.ts';
import { isRuntimeProvider, RUNTIME_PROVIDERS } from '../../../lib/providerRegistry.ts';

export { isRuntimeProvider, RUNTIME_PROVIDERS };
export type RuntimeProviderId = RuntimeProvider;

/** Whether `provider`'s structured runtime protocol supports mid-turn `Steer`
 *  (injecting a user message into the running turn without cancelling it):
 *  - `claude`: Agent SDK stream-json `{"type": "user", "message": ...}`
 *  - `codex`: `codex app-server` v2 `turn/steer {threadId, input, expectedTurnId}`
 *  - `pi`: `pi --mode rpc` `{"type": "steer", "message": ..., "images"?: ...}`
 *  - ACP v1 (`agy`, `gemini`, `grok`, `opencode`): no `session/steer` RPC in
 *    ACP v1 (`session/prompt` + `session/cancel` only), so queued messages
 *    offer `Send now` (cancel & send) rather than `Steer`. */
export function providerSupportsSteer(provider: string): boolean {
  return provider === 'claude' || provider === 'codex' || provider === 'pi';
}
