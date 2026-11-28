// The Unity Editor the open project has (rc.6 A1), asked when the launch
// target menu opens; feeds the "· running / · not running" suffix.
import { api } from '../../../lib/api/client';
import type { IdeTargetSpec } from '../../../lib/api/types';
import type { IdeUnityEditor, IdeUnityInstancesResp } from '../../../lib/api/types/ide-unity';

export const unityAttach = $state<{ editors: IdeUnityEditor[] | null }>({ editors: null });

export async function refreshUnityEditors(target: IdeTargetSpec, cwd?: string | null): Promise<void> {
  try {
    const resp = await api.post<IdeUnityInstancesResp>('/ide/debug/unity/instances', { target, cwd: cwd ?? null });
    unityAttach.editors = resp.editors;
  } catch {
    unityAttach.editors = null;
  }
}
