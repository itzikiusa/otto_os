// "Attach to Unity Editor" (rc.6 A1): the launch target's running / not
// running suffix, from `POST /ide/debug/unity/instances` (one small file read
// on the daemon, never a connection). Asked when the target menu opens.
//
// Import-free (type-only imports), so Node runs it directly in unit tests.

import type { IdeUnityEditor } from '../../../lib/api/types/ide-unity';

export const UNITY_ATTACH_RUNNER = 'unity_attach';

/** The Run button's title for a debug-only attach target. */
export const ATTACH_ONLY_TITLE = 'Attach targets can only be debugged';

/** "Attach to Unity Editor · 6000.0.23f1 · running" / "… · not running";
 *  the bare name until the menu has asked once. */
export function unityTargetLabel(name: string, editors: readonly IdeUnityEditor[] | null): string {
  if (!editors) return name;
  const ed = editors[0];
  return ed ? `${name} · ${ed.version} · running` : `${name} · not running`;
}
