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

export function classifyDepsActionOutcome(
  res: DepsActionResp,
  label: string,
  fileName: string,
): { level: 'success' | 'error'; title: string; detail: string } {
  const out = (res.output ?? '').trim();
  const detail = out ? out.split('\n')[0].slice(0, 140) : fileName;
  if (!res.ok) {
    return { level: 'error', title: `Couldn’t run ${label}`, detail };
  }
  return { level: 'success', title: label, detail };
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
    const outcome = classifyDepsActionOutcome(res, label, ideBasename(filePath));
    if (outcome.level === 'error') {
      toasts.error(outcome.title, outcome.detail);
    } else if (showModalOutput && out && onTreeOutput) {
      onTreeOutput(`${label} — ${ideBasename(filePath)}`, out);
    } else {
      toasts.success(outcome.title, outcome.detail);
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
  if (fileName === 'build.zig.zon') {
    return [
      { separator: true },
      {
        label: 'Zig Fetch Dependencies',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'sync', 'Zig Fetch Dependencies', onTreeOutput, false),
      },
    ];
  }
  const lower = fileName.toLowerCase();
  if (lower === 'cmakelists.txt') {
    return [
      { separator: true },
      {
        label: 'CMake Configure (Generate compile_commands.json)',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'sync', 'CMake Configure', onTreeOutput, false),
      },
    ];
  }
  if (lower === 'vcpkg.json') {
    return [
      { separator: true },
      {
        label: 'vcpkg Install Dependencies',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'sync', 'vcpkg Install', onTreeOutput, false),
      },
    ];
  }
  if (lower === 'conanfile.txt') {
    return [
      { separator: true },
      {
        label: 'Conan Install Dependencies',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'sync', 'Conan Install', onTreeOutput, false),
      },
    ];
  }
  if (lower.endsWith('.csproj') || lower.endsWith('.fsproj') || lower.endsWith('.vbproj') || lower === 'directory.packages.props') {
    return [
      { separator: true },
      {
        label: 'Restore NuGet Packages',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'sync', 'Restore NuGet Packages', onTreeOutput, false),
      },
      {
        label: 'List Outdated NuGet Packages',
        icon: 'layers',
        action: () =>
          void runManifestDepsAction(filePath, 'outdated', 'Outdated NuGet Packages', onTreeOutput, true),
      },
    ];
  }
  if (lower === 'requirements.txt' || lower === 'pyproject.toml') {
    return [
      { separator: true },
      {
        label: 'Sync Python Dependencies (uv / pip)',
        icon: 'refresh',
        action: () =>
          void runManifestDepsAction(filePath, 'sync', 'Sync Python Dependencies', onTreeOutput, false),
      },
    ];
  }
  return [];
}
