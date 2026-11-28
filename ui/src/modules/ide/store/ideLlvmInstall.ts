// Managed LLVM download (design B §1): one confirm that names the source, the
// exact file and its sizes; the install job (download → SHA-256 → unpack →
// self-check runs on the daemon); then the project's C/C++ toolchain is set to
// it, with Undo. Lazy-loaded by the C/C++ settings and the Toolchains window.

import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { ide } from '../ideStore.svelte';
import { llvmConfirmText, llvmSizes } from './ideLlvmText';
import type { IdeDownloadableSdk, IdeInstallJob } from './ideTypes';

async function waitForJob(id: string): Promise<IdeInstallJob> {
  for (;;) {
    await new Promise((r) => setTimeout(r, 1500));
    const job = (await api.get<{ job: IdeInstallJob }>(`/ide/toolchains/jobs/${encodeURIComponent(id)}`)).job;
    if (job.status !== 'running') return job;
  }
}

/** Confirm, install and select LLVM. `onSelected` keeps an open Settings draft in step. */
export async function downloadLlvm(rel: IdeDownloadableSdk, onSelected?: (path: string) => void): Promise<void> {
  const { download } = llvmSizes(rel);
  const ok = await confirmer.ask(llvmConfirmText(rel), {
    title: `Download LLVM ${rel.version}?`,
    confirmLabel: download ? `Download (${download})` : 'Download',
  });
  if (!ok) return;
  const started = await ide.installToolchain('cpp', 'sdk', rel.version);
  if (!started) return;
  const job = await waitForJob(started.id);
  if (job.status !== 'done' || !job.installed_path) {
    toasts.error(`LLVM ${rel.version} was not installed`, job.error ?? undefined);
    return;
  }
  const path = job.installed_path;
  const prev = ide.projectEnv?.cpp_toolchain ?? null;
  if (await ide.saveEnvironment({ cpp_toolchain: path })) onSelected?.(path);
  await ide.loadToolchains({ includeCatalog: true });
  toasts.push('success', `LLVM ${rel.version} installed and selected for this project`, undefined, 10000, {
    action: {
      label: 'Undo',
      run: async () => {
        if (await ide.saveEnvironment({ cpp_toolchain: prev })) onSelected?.(prev ?? '');
      },
    },
  });
}
