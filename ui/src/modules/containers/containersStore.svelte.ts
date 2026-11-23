// Reactive state & actions controller for the Containers module.
// Keeps `ContainersPage.svelte` focused on layout/chrome (<400 LOC).

import { containersApi } from '../../lib/api/containers';
import { ApiError } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { formatBytes } from '../../lib/metric-format';
import { plural } from '../../lib/plural';
import { recallSelection, rememberSelection } from '../../lib/lastSelection';
import type {
  ContainerSummary,
  ContainersStatusResp,
  DockerContextInfo,
  ImageSummary,
  NetworkSummary,
  RuntimeStatusResp,
  VolumeSummary,
} from '../../lib/api/types/containers';
import {
  pickInitialContainer,
  type ContainerDetailTab,
  type ContainersMainTab,
} from './containers-util';

const SELECTION_KEY = 'containers.selected';

export class ContainersController {
  status = $state<ContainersStatusResp | null>(null);
  statusError = $state<string | null>(null);
  unavailable = $state(false);
  contexts = $state.raw<DockerContextInfo[]>([]);
  runtime = $state<RuntimeStatusResp | null>(null);
  runtimeBusy = $state<'install' | 'start' | 'stop' | 'restart' | null>(null);
  runtimeError = $state<string | null>(null);

  activeTab = $state<ContainersMainTab>('containers');
  detailTab = $state<ContainerDetailTab>('overview');
  selectedId = $state<string | null>(null);
  phoneDetailOpen = $state(false);

  containers = $state.raw<ContainerSummary[]>([]);
  containersLoading = $state(false);
  containersError = $state<string | null>(null);

  images = $state.raw<ImageSummary[]>([]);
  imagesLoaded = $state(false);
  imagesLoading = $state(false);
  imagesError = $state<string | null>(null);

  volumes = $state.raw<VolumeSummary[]>([]);
  volumesLoaded = $state(false);
  volumesLoading = $state(false);
  volumesError = $state<string | null>(null);

  networks = $state.raw<NetworkSummary[]>([]);
  networksLoaded = $state(false);
  networksLoading = $state(false);
  networksError = $state<string | null>(null);

  busyIds = $state<Set<string>>(new Set());
  busyStacks = $state<Set<string>>(new Set());
  pulling = $state(false);
  switchingContext = $state(false);
  detailRefreshTick = $state(0);

  private bootSeq = 0;

  get selectedContainer(): ContainerSummary | null {
    return this.selectedId ? (this.containers.find((c) => c.id === this.selectedId) ?? null) : null;
  }

  get reachableFallback(): DockerContextInfo | null {
    return this.contexts.find((c) => c.reachable && !c.is_active) ?? null;
  }

  async loadStatusAndContexts(): Promise<boolean> {
    const seq = ++this.bootSeq;
    this.statusError = null;
    this.unavailable = false;
    try {
      const [st, ctxResp, rtResp] = await Promise.all([
        containersApi.status(),
        containersApi.contexts().catch(() => null),
        containersApi.runtime().catch(() => null),
      ]);
      if (seq !== this.bootSeq) return false;
      this.status = st;
      if (ctxResp) this.contexts = ctxResp.contexts;
      if (rtResp) this.runtime = rtResp;
      if (st.available) this.runtimeError = null;
      return st.available;
    } catch (e) {
      if (seq !== this.bootSeq) return false;
      if (e instanceof ApiError && e.status === 404) {
        this.unavailable = true;
      } else {
        this.statusError = loadErrorText(e);
      }
      return false;
    }
  }

  async loadContainers(): Promise<void> {
    this.containersLoading = true;
    this.containersError = null;
    try {
      const resp = await containersApi.list({ all: true });
      this.containers = resp.containers;
      const picked = pickInitialContainer(resp.containers, this.selectedId ?? recallSelection(SELECTION_KEY));
      this.selectedId = picked;
      if (picked) rememberSelection(SELECTION_KEY, picked);
      this.detailRefreshTick += 1;
    } catch (e) {
      this.containersError = loadErrorText(e);
    } finally {
      this.containersLoading = false;
    }
  }

  async loadImages(): Promise<void> {
    this.imagesLoading = true;
    this.imagesError = null;
    try {
      const resp = await containersApi.listImages();
      this.images = resp.images;
      this.imagesLoaded = true;
    } catch (e) {
      this.imagesError = loadErrorText(e);
    } finally {
      this.imagesLoading = false;
    }
  }

  async loadVolumes(): Promise<void> {
    this.volumesLoading = true;
    this.volumesError = null;
    try {
      const resp = await containersApi.listVolumes();
      this.volumes = resp.volumes;
      this.volumesLoaded = true;
    } catch (e) {
      this.volumesError = loadErrorText(e);
    } finally {
      this.volumesLoading = false;
    }
  }

  async loadNetworks(): Promise<void> {
    this.networksLoading = true;
    this.networksError = null;
    try {
      const resp = await containersApi.listNetworks();
      this.networks = resp.networks;
      this.networksLoaded = true;
    } catch (e) {
      this.networksError = loadErrorText(e);
    } finally {
      this.networksLoading = false;
    }
  }

  async refreshActiveTab(): Promise<void> {
    const ok = await this.loadStatusAndContexts();
    if (!ok) return;
    if (this.activeTab === 'containers') await this.loadContainers();
    else if (this.activeTab === 'images') await this.loadImages();
    else if (this.activeTab === 'volumes') await this.loadVolumes();
    else if (this.activeTab === 'networks') await this.loadNetworks();
  }

  switchTab(tab: ContainersMainTab): void {
    this.activeTab = tab;
    this.phoneDetailOpen = false;
    if (!this.status?.available) return;
    if (tab === 'containers' && this.containers.length === 0 && !this.containersLoading) {
      void this.loadContainers();
    } else if (tab === 'images' && !this.imagesLoaded && !this.imagesLoading) {
      void this.loadImages();
    } else if (tab === 'volumes' && !this.volumesLoaded && !this.volumesLoading) {
      void this.loadVolumes();
    } else if (tab === 'networks' && !this.networksLoaded && !this.networksLoading) {
      void this.loadNetworks();
    }
  }

  selectContainer(id: string, isPhone: boolean, tab?: ContainerDetailTab): void {
    this.selectedId = id;
    rememberSelection(SELECTION_KEY, id);
    if (tab) this.detailTab = tab;
    if (isPhone) this.phoneDetailOpen = true;
  }

  async handleContainerAction(
    c: ContainerSummary,
    action: 'start' | 'stop' | 'restart' | 'pause' | 'unpause' | 'remove',
  ): Promise<void> {
    if (action === 'remove') {
      const isRunning = c.state.toLowerCase() === 'running';
      const ok = await confirmer.ask(
        isRunning
          ? `Force remove running container “${c.name}”? The container will be stopped immediately and its writable layer deleted.`
          : `Remove container “${c.name}”? Its writable filesystem layer will be deleted.`,
        {
          title: 'Remove container',
          confirmLabel: isRunning ? 'Force remove container' : 'Remove container',
          danger: true,
        },
      );
      if (!ok) return;
    }

    const nextBusy = new Set(this.busyIds);
    nextBusy.add(c.id);
    this.busyIds = nextBusy;
    try {
      if (action === 'start') {
        await containersApi.start(c.id);
        toasts.success('Container started', c.name);
      } else if (action === 'stop') {
        await containersApi.stop(c.id);
        toasts.success('Container stopped', c.name);
      } else if (action === 'restart') {
        await containersApi.restart(c.id);
        toasts.success('Container restarted', c.name);
      } else if (action === 'pause') {
        await containersApi.pause(c.id);
        toasts.success('Container paused', c.name);
      } else if (action === 'unpause') {
        await containersApi.unpause(c.id);
        toasts.success('Container resumed', c.name);
      } else if (action === 'remove') {
        const isRunning = c.state.toLowerCase() === 'running';
        await containersApi.remove(c.id, { force: isRunning });
        toasts.success('Container removed', c.name);
        if (this.selectedId === c.id) this.phoneDetailOpen = false;
      }
      await Promise.all([this.loadStatusAndContexts(), this.loadContainers()]);
    } catch (e) {
      toastError(`Couldn’t ${action} container`, e);
    } finally {
      const done = new Set(this.busyIds);
      done.delete(c.id);
      this.busyIds = done;
    }
  }

  async handleStackAction(project: string, action: 'start' | 'stop' | 'restart'): Promise<void> {
    if (action === 'stop') {
      const ok = await confirmer.ask(
        `Stop all running containers in Compose stack “${project}”?`,
        {
          title: 'Stop Compose stack',
          confirmLabel: 'Stop stack',
          danger: true,
        },
      );
      if (!ok) return;
    }
    const nextBusy = new Set(this.busyStacks);
    nextBusy.add(project);
    this.busyStacks = nextBusy;
    try {
      const res =
        action === 'start'
          ? await containersApi.stackStart(project)
          : action === 'stop'
            ? await containersApi.stackStop(project)
            : await containersApi.stackRestart(project);
      if (res.failed.length > 0) {
        toasts.warn(
          `Stack “${project}” ${action} finished with ${plural(res.failed.length, 'error')}`,
          res.failed.map((f) => `${f.name}: ${f.error}`).join(' · '),
        );
      } else {
        toasts.success(
          `Stack “${project}” ${action === 'start' ? 'started' : action === 'stop' ? 'stopped' : 'restarted'}`,
          plural(res.succeeded.length, 'container'),
        );
      }
      await Promise.all([this.loadStatusAndContexts(), this.loadContainers()]);
    } catch (e) {
      toastError(`Couldn’t ${action} stack`, e);
    } finally {
      const done = new Set(this.busyStacks);
      done.delete(project);
      this.busyStacks = done;
    }
  }

  async pullImagePrompt(): Promise<void> {
    if (this.pulling) return;
    const ref = await confirmer.promptText('Enter an image reference to pull from its registry.', {
      title: 'Pull container image',
      confirmLabel: 'Pull image',
      placeholder: 'postgres:16-alpine',
      validate: (v) => (v.trim() ? null : 'Enter an image name, such as nginx:alpine.'),
    });
    if (!ref) return;
    this.pulling = true;
    try {
      const res = await containersApi.pullImage({ reference: ref });
      toasts.success(
        'Image pulled',
        res.digest ? `${res.reference} (${res.digest.slice(0, 19)})` : res.reference,
      );
      await Promise.all([this.loadStatusAndContexts(), this.loadImages()]);
    } catch (e) {
      toastError('Couldn’t pull image', e);
    } finally {
      this.pulling = false;
    }
  }

  async handleSystemPrune(): Promise<void> {
    const ok = await confirmer.ask(
      'Remove all stopped containers, unused networks, and dangling images? Running containers and named volumes are kept.',
      {
        title: 'Prune unused container resources',
        confirmLabel: 'Prune resources',
        danger: true,
      },
    );
    if (!ok) return;
    try {
      const res = await containersApi.systemPrune({ include_volumes: false, all_unused_images: false });
      const total =
        res.containers_deleted.length +
        res.images_deleted.length +
        res.networks_deleted.length +
        res.volumes_deleted.length;
      toasts.success(
        total > 0 ? `Pruned ${plural(total, 'item')}` : 'Nothing to prune',
        res.space_reclaimed_bytes > 0 ? `Reclaimed ${formatBytes(res.space_reclaimed_bytes)}` : undefined,
      );
      this.imagesLoaded = false;
      this.volumesLoaded = false;
      this.networksLoaded = false;
      await this.refreshActiveTab();
    } catch (e) {
      toastError('Couldn’t prune container resources', e);
    }
  }

  async installRuntime(startAfterInstall = true): Promise<void> {
    if (this.runtimeBusy || this.runtime?.installing) return;
    this.runtimeBusy = 'install';
    this.runtimeError = null;
    try {
      const rt = await containersApi.installRuntime({ start_after_install: startAfterInstall });
      this.runtime = rt;
      this.imagesLoaded = false;
      this.volumesLoaded = false;
      this.networksLoaded = false;
      if (rt.install_error) {
        this.runtimeError = rt.install_error;
        toasts.error('Couldn’t install Kiri runtime', rt.install_error);
      } else {
        toasts.success(
          rt.running ? 'Kiri runtime installed and started' : 'Kiri runtime installed',
          rt.binary_path ?? rt.socket_path,
        );
      }
      await this.refreshActiveTab();
    } catch (e) {
      this.runtimeError = loadErrorText(e);
      toastError('Couldn’t install Kiri runtime', e);
    } finally {
      this.runtimeBusy = null;
    }
  }

  async startRuntime(): Promise<void> {
    if (this.runtimeBusy) return;
    this.runtimeBusy = 'start';
    this.runtimeError = null;
    try {
      const rt = await containersApi.startRuntime();
      this.runtime = rt;
      this.imagesLoaded = false;
      this.volumesLoaded = false;
      this.networksLoaded = false;
      toasts.success('Kiri runtime started', rt.socket_path);
      await this.refreshActiveTab();
    } catch (e) {
      this.runtimeError = loadErrorText(e);
      toastError('Couldn’t start Kiri runtime', e);
    } finally {
      this.runtimeBusy = null;
    }
  }

  async stopRuntime(): Promise<void> {
    if (this.runtimeBusy) return;
    const runningCount = this.status?.containers_running ?? 0;
    const ok = await confirmer.ask(
      runningCount > 0
        ? `Stop the embedded Kiri runtime? ${plural(runningCount, 'running container')} will be stopped.`
        : 'Stop the embedded Kiri runtime? You can start it again at any time from Containers.',
      {
        title: 'Stop Kiri runtime',
        confirmLabel: 'Stop Kiri runtime',
        danger: true,
      },
    );
    if (!ok) return;
    this.runtimeBusy = 'stop';
    this.runtimeError = null;
    try {
      const rt = await containersApi.stopRuntime();
      this.runtime = rt;
      toasts.info('Kiri runtime stopped');
      await this.refreshActiveTab();
    } catch (e) {
      this.runtimeError = loadErrorText(e);
      toastError('Couldn’t stop Kiri runtime', e);
    } finally {
      this.runtimeBusy = null;
    }
  }

  async restartRuntime(): Promise<void> {
    if (this.runtimeBusy) return;
    const runningCount = this.status?.containers_running ?? 0;
    if (runningCount > 0) {
      const ok = await confirmer.ask(
        `Restart the embedded Kiri runtime? ${plural(runningCount, 'running container')} will be interrupted during restart.`,
        {
          title: 'Restart Kiri runtime',
          confirmLabel: 'Restart Kiri runtime',
          danger: true,
        },
      );
      if (!ok) return;
    }
    this.runtimeBusy = 'restart';
    this.runtimeError = null;
    try {
      const rt = await containersApi.restartRuntime();
      this.runtime = rt;
      toasts.success('Kiri runtime restarted', rt.socket_path);
      await this.refreshActiveTab();
    } catch (e) {
      this.runtimeError = loadErrorText(e);
      toastError('Couldn’t restart Kiri runtime', e);
    } finally {
      this.runtimeBusy = null;
    }
  }

  async handleSelectContext(ctx: DockerContextInfo): Promise<void> {
    if (this.switchingContext) return;
    this.switchingContext = true;
    try {
      const updated = await containersApi.selectContext({ name: ctx.name });
      this.contexts = updated.contexts;
      this.imagesLoaded = false;
      this.volumesLoaded = false;
      this.networksLoaded = false;
      this.selectedId = null;
      this.phoneDetailOpen = false;
      toasts.info('Switched Docker context', `${ctx.name} (${ctx.socket_path})`);
      await this.refreshActiveTab();
    } catch (e) {
      toastError('Couldn’t switch Docker context', e);
    } finally {
      this.switchingContext = false;
    }
  }
}
