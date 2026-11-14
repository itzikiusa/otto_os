// Project tree right-click dependency manifest actions (`go.mod`, `pom.xml`,
// `build.gradle`, `build.gradle.kts`) calling `POST /ide/deps/action` (which
// snapshots Local History first) and reloading open IDE tabs + Git status.

import { api } from '../../../lib/api/client';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { ideGit } from '../git/ideGitController.svelte';
import { ide, ideBasename } from '../ideStore.svelte';

export interface DepsActionResp {
  ok: boolean;
  updated_content?: string | null;
  output?: string;
}

export async function runManifestDepsAction(
  filePath: string,
  action: string,
  label: string,
  onTreeOutput?: (title: string, output: string) => void,
): Promise<void> {
  try {
    const res = await api.post<DepsActionResp>('/ide/deps/action', {
      target: ide.target,
      path: filePath,
      action,
    });
    await ide.loadTree();
    await ideGit.reloadIdeTabIfOpen(filePath);
    if (ideBasename(filePath) === 'go.mod') {
      const siblingSum = `${filePath.slice(0, -'go.mod'.length)}go.sum`;
      await ideGit.reloadIdeTabIfOpen(siblingSum);
    }
    void ideGit.refreshActive();

    const out = (res.output ?? '').trim();
    if (action === 'maven_tree' && out && onTreeOutput) {
      onTreeOutput(`${label} — ${ideBasename(filePath)}`, out);
    } else {
      toasts.success(label, out ? out.split('\n')[0].slice(0, 140) : ideBasename(filePath));
    }
  } catch (e) {
    toastError(`Couldn’t run ${label}`, e);
  }
}

export function buildManifestMenuItems(
  filePath: string,
  fileName: string,
  onTreeOutput?: (title: string, output: string) => void,
): MenuItem[] {
  if (fileName === 'go.mod') {
    return [
      { separator: true },
      {
        label: 'Go Mod Tidy',
        icon: 'format',
        action: () => void runManifestDepsAction(filePath, 'tidy', 'Go Mod Tidy', onTreeOutput),
      },
      {
        label: 'Go Mod Vendor',
        icon: 'box',
        action: () => void runManifestDepsAction(filePath, 'vendor', 'Go Mod Vendor', onTreeOutput),
      },
      {
        label: 'Sync Go Module',
        icon: 'refresh',
        action: () => void runManifestDepsAction(filePath, 'sync', 'Sync Go Module', onTreeOutput),
      },
    ];
  }
  if (fileName === 'pom.xml' || /^build\.gradle(?:\.kts)?$/.test(fileName)) {
    return [
      { separator: true },
      {
        label: 'Reload Maven/Gradle Project',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(
            filePath,
            'maven_sync',
            'Reload Maven/Gradle Project',
            onTreeOutput,
          ),
      },
      {
        label: 'Show Dependency Tree',
        icon: 'layers',
        action: () =>
          void runManifestDepsAction(
            filePath,
            'maven_tree',
            'Dependency Tree',
            onTreeOutput,
          ),
      },
    ];
  }
  return [];
}
