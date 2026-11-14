// Per-provider capabilities on the structured agent runtime (no imports: unit
// tests load it directly).

export const RUNTIME_PROVIDERS = [
  'claude',
  'codex',
  'pi',
  'agy',
  'gemini',
  'grok',
  'opencode',
] as const;

export type RuntimeProviderId = (typeof RUNTIME_PROVIDERS)[number];

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
