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
  showModalOutput = action === 'maven_tree',
): Promise<void> {
  const root = ideGit.rootContainingPath(filePath)?.path || ide.target.root;
  try {
    const res = await api.post<DepsActionResp>('/ide/deps/action', {
      target: { ...ide.targetPayload, root },
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
    if (showModalOutput && out && onTreeOutput) {
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
        action: () => void runManifestDepsAction(filePath, 'tidy', 'Go Mod Tidy', onTreeOutput, false),
      },
      {
        label: 'Go Mod Vendor',
        icon: 'box',
        action: () => void runManifestDepsAction(filePath, 'vendor', 'Go Mod Vendor', onTreeOutput, false),
      },
      {
        label: 'Sync Go Module',
        icon: 'refresh',
        action: () => void runManifestDepsAction(filePath, 'sync', 'Sync Go Module', onTreeOutput, false),
      },
    ];
  }
  if (fileName === 'pom.xml') {
    return [
      { separator: true },
      {
        label: 'Reload Maven Project',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'maven_sync', 'Reload Maven Project', onTreeOutput, false),
      },
      {
        label: 'Show Dependency Tree',
        icon: 'layers',
        action: () =>
          void runManifestDepsAction(filePath, 'maven_tree', 'Dependency Tree', onTreeOutput, true),
      },
    ];
  }
  if (/^build\.gradle(?:\.kts)?$/.test(fileName)) {
    return [
      { separator: true },
      {
        label: 'Reload Gradle Project',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'gradle_sync', 'Reload Gradle Project', onTreeOutput, false),
      },
      {
        label: 'Show Dependency Tree',
        icon: 'layers',
        action: () =>
          void runManifestDepsAction(filePath, 'gradle_sync', 'Dependency Tree', onTreeOutput, true),
      },
    ];
  }
  return [];
}
