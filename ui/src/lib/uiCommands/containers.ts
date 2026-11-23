// ⌘K command-palette entries for the Containers module.
// Kept pure (no imports from `src/modules/`) so `lib-imports-modules` stays
// green and unit tests can verify the command list directly.

import type { Command } from '../commands.svelte';
import type {
  ContainerSummary,
  DockerContextInfo,
  RuntimeStatusResp,
} from '../api/types/containers';

export interface ContainersCommandHandlers {
  containers: readonly ContainerSummary[];
  contexts: readonly DockerContextInfo[];
  runtime: RuntimeStatusResp | null;
  available: boolean;
  openTab: (tab: 'containers' | 'images' | 'volumes' | 'networks') => void;
  selectContainer: (id: string, detailTab?: 'overview' | 'logs' | 'terminal' | 'stats') => void;
  openRunModal: () => void;
  pullImage: () => void;
  refresh: () => void;
  systemPrune: () => void;
  selectContext: (ctx: DockerContextInfo) => void;
  startRuntime: () => void;
  stopRuntime: () => void;
  restartRuntime: () => void;
}

/** Build the ⌘K command set for the active Containers view. */
export function buildContainerCommands(h: ContainersCommandHandlers): Command[] {
  const cmds: Command[] = [
    {
      id: 'containers.tab.containers',
      title: 'Show Containers',
      group: 'Containers',
      keywords: 'docker ps running stacks compose',
      run: () => h.openTab('containers'),
    },
    {
      id: 'containers.tab.images',
      title: 'Show Container Images',
      group: 'Containers',
      keywords: 'docker images tags layers pull',
      run: () => h.openTab('images'),
    },
    {
      id: 'containers.tab.volumes',
      title: 'Show Container Volumes',
      group: 'Containers',
      keywords: 'docker volumes storage mounts data',
      run: () => h.openTab('volumes'),
    },
    {
      id: 'containers.tab.networks',
      title: 'Show Container Networks',
      group: 'Containers',
      keywords: 'docker networks bridge subnet dns',
      run: () => h.openTab('networks'),
    },
    {
      id: 'containers.refresh',
      title: 'Refresh containers',
      group: 'Containers',
      keywords: 'docker reload status daemon',
      run: () => h.refresh(),
    },
  ];

  if (h.available) {
    cmds.push(
      {
        id: 'containers.run',
        title: 'Run container…',
        group: 'Containers',
        keywords: 'docker run new start image',
        run: () => h.openRunModal(),
      },
      {
        id: 'containers.pull',
        title: 'Pull container image…',
        group: 'Containers',
        keywords: 'docker pull registry download image',
        run: () => h.pullImage(),
      },
      {
        id: 'containers.prune',
        title: 'Prune unused container resources…',
        group: 'Containers',
        keywords: 'docker system prune clean reclaim disk',
        run: () => h.systemPrune(),
      },
    );
  }

  if (h.runtime?.installed) {
    if (!h.runtime.running && h.runtime.can_start) {
      cmds.push({
        id: 'containers.runtime.start',
        title: 'Start Kiri runtime',
        group: 'Containers',
        detail: h.runtime.socket_path,
        keywords: 'kiri docker runtime vm start engine daemon',
        run: () => h.startRuntime(),
      });
    }
    if (h.runtime.running) {
      cmds.push(
        {
          id: 'containers.runtime.restart',
          title: 'Restart Kiri runtime',
          group: 'Containers',
          detail: h.runtime.vm_state,
          keywords: 'kiri docker runtime vm restart engine daemon',
          run: () => h.restartRuntime(),
        },
        {
          id: 'containers.runtime.stop',
          title: 'Stop Kiri runtime…',
          group: 'Containers',
          detail: h.runtime.vm_state,
          keywords: 'kiri docker runtime vm stop shutdown engine daemon',
          run: () => h.stopRuntime(),
        },
      );
    }
  }

  for (const ctx of h.contexts) {
    if (ctx.is_active) continue;
    cmds.push({
      id: `containers.context.${ctx.name}`,
      title: `Switch Docker context to ${ctx.name}`,
      group: 'Containers',
      detail: ctx.reachable ? ctx.socket_path : 'unreachable',
      keywords: `docker context socket engine ${ctx.engine_kind}`,
      run: () => h.selectContext(ctx),
    });
  }

  // Cap per-container palette entries so large hosts stay snappy in ⌘K.
  for (const c of h.containers.slice(0, 40)) {
    cmds.push({
      id: `containers.open.${c.id}`,
      title: `Open container ${c.name}`,
      group: 'Containers',
      detail: `${c.image} · ${c.state}`,
      keywords: `docker container ${c.short_id} ${c.compose_project ?? ''}`,
      run: () => h.selectContainer(c.id, 'overview'),
    });
    if (c.state.toLowerCase() === 'running') {
      cmds.push({
        id: `containers.logs.${c.id}`,
        title: `View logs for ${c.name}`,
        group: 'Containers',
        detail: c.image,
        keywords: `docker logs tail ${c.short_id}`,
        run: () => h.selectContainer(c.id, 'logs'),
      });
    }
  }

  return cmds;
}
