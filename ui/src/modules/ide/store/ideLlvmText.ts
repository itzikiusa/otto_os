// Pure text for the managed LLVM download confirm (unit-tested in
// ui/unit/ideLlvmText.test.ts). The daemon's catalog entry carries the pinned
// file's URL and "871 MB download · 444 MB on disk" as its size_hint.

import type { IdeDownloadableSdk } from './ideTypes';

/** "871 MB download · 444 MB on disk" → { download: "871 MB", disk: "444 MB" }. */
export function llvmSizes(rel: IdeDownloadableSdk): { download: string; disk: string } {
  const [dl = '', disk = ''] = (rel.size_hint ?? '').split(' · ');
  return { download: dl.replace(/ download$/, ''), disk: disk.replace(/ on disk$/, '') };
}

/** The confirm body: where the file comes from, its exact name and size, what lands on disk. */
export function llvmConfirmText(rel: IdeDownloadableSdk): string {
  const file = rel.url.split('/').pop() ?? rel.url;
  const tag = /\/releases\/download\/([^/]+)\//.exec(rel.url)?.[1] ?? rel.version;
  const { download, disk } = llvmSizes(rel);
  const lines = [
    `Source: github.com/llvm/llvm-project, release ${tag}`,
    `File: ${file} (${download})`,
    `Installs ${disk} under Otto’s toolchains folder. Otto checks the prerequisites and free disk first, and the file’s SHA-256 before unpacking.`,
  ];
  if (rel.os === 'macos') lines.push('Needs Apple’s Command Line Tools. Debugging keeps using Apple’s lldb-dap.');
  return lines.join('\n');
}
