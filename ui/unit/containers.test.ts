// Unit tests for Otto's Containers module pure helpers and ⌘K commands:
// compose stack grouping, initial selection, search/state filtering, env secret
// masking, port formatting/URL generation, run-modal parsers, and ⌘K commands.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource, deferred } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

import type {
  ContainerDetail,
  ContainerSummary,
  ContainersStatusResp,
  DockerContextInfo,
  EnvEntry,
  ImageSummary,
  NetworkSummary,
  PortMapping,
  RuntimeStatusResp,
  VolumeSummary,
} from '../src/lib/api/types/containers.ts';
import {
  SECRET_MASK,
  buildOfflineDetail,
  composeProjectOf,
  containerHealthInfo,
  containerStateInfo,
  dedupePorts,
  displayEnvEntry,
  engineKindLabel,
  filterContainers,
  filterImages,
  filterLogLines,
  filterNetworks,
  filterVolumes,
  formatLogsForCopy,
  formatPortMapping,
  formatPortsSummary,
  groupContainersByProject,
  imageDisplayName,
  maskEnvValue,
  parseCommandArgs,
  parseEnvLines,
  parsePortSpecs,
  pickInitialContainer,
  portUrl,
  runtimeStateInfo,
} from '../src/modules/containers/containers-util.ts';
import { buildContainerCommands } from '../src/lib/uiCommands/containers.ts';

function makeContainer(overrides: Partial<ContainerSummary> = {}): ContainerSummary {
  return {
    id: 'c-default-1234567890ab',
    short_id: 'c-default-12',
    name: 'web',
    image: 'nginx:alpine',
    image_id: 'sha256:111',
    command: 'nginx -g "daemon off;"',
    created: 1_700_000_000,
    state: 'running',
    status: 'Up 2 hours',
    health: null,
    compose_project: null,
    compose_service: null,
    ports: [],
    labels: {},
    networks: ['bridge'],
    mounts_count: 0,
    ...overrides,
  };
}

describe('containers compose grouping and ordering', () => {
  it('groups by com.docker.compose.project label or compose_project field and places standalone last', () => {
    const items: ContainerSummary[] = [
      makeContainer({ id: '1', name: 'solo-old', state: 'exited', created: 100 }),
      makeContainer({
        id: '2',
        name: 'payments-db',
        state: 'running',
        created: 200,
        labels: { 'com.docker.compose.project': 'payments', 'com.docker.compose.service': 'db' },
      }),
      makeContainer({
        id: '3',
        name: 'payments-api',
        state: 'exited',
        created: 300,
        compose_project: 'payments',
        compose_service: 'api',
      }),
      makeContainer({
        id: '4',
        name: 'alpha-cache',
        state: 'running',
        created: 150,
        compose_project: 'alpha',
      }),
      makeContainer({ id: '5', name: 'solo-live', state: 'running', created: 50 }),
    ];

    const groups = groupContainersByProject(items);
    assert.deepEqual(
      groups.map((g) => g.label),
      ['alpha', 'payments', 'Standalone'],
    );
    assert.equal(groups[0].runningCount, 1);
    assert.equal(groups[1].runningCount, 1);
    assert.equal(groups[1].totalCount, 2);
    // Inside a stack, running containers sort before exited ones.
    assert.deepEqual(
      groups[1].containers.map((c) => c.name),
      ['payments-db', 'payments-api'],
    );
    // Standalone keeps running before stopped.
    assert.deepEqual(
      groups[2].containers.map((c) => c.name),
      ['solo-live', 'solo-old'],
    );
  });

  it('extracts compose project from field first and falls back to label', () => {
    assert.equal(
      composeProjectOf({ compose_project: 'stack-a', labels: { 'com.docker.compose.project': 'stack-b' } }),
      'stack-a',
    );
    assert.equal(
      composeProjectOf({ compose_project: null, labels: { 'com.docker.compose.project': 'stack-b' } }),
      'stack-b',
    );
    assert.equal(composeProjectOf({ compose_project: '', labels: {} }), null);
  });

  it('picks remembered container, else first running (newest), else newest stopped', () => {
    const stoppedOld = makeContainer({ id: 's1', name: 'old', state: 'exited', created: 100 });
    const stoppedNew = makeContainer({ id: 's2', name: 'new', state: 'exited', created: 500 });
    const runningOld = makeContainer({ id: 'r1', name: 'run-old', state: 'running', created: 200 });
    const runningNew = makeContainer({ id: 'r2', name: 'run-new', state: 'running', created: 400 });

    assert.equal(pickInitialContainer([stoppedOld, stoppedNew, runningOld, runningNew]), 'r2');
    assert.equal(pickInitialContainer([stoppedOld, stoppedNew, runningOld, runningNew], 's1'), 's1');
    assert.equal(pickInitialContainer([stoppedOld, stoppedNew, runningOld, runningNew], 'gone'), 'r2');
    assert.equal(pickInitialContainer([stoppedOld, stoppedNew]), 's2');
    assert.equal(pickInitialContainer([]), null);
  });
});

describe('containers search and state filtering', () => {
  const sample: ContainerSummary[] = [
    makeContainer({
      id: 'c1',
      short_id: 'aaa111',
      name: 'billing-api',
      image: 'node:22-alpine',
      state: 'running',
      compose_project: 'billing',
      ports: [{ private_port: 3000, public_port: 8080, protocol: 'tcp' }],
    }),
    makeContainer({
      id: 'c2',
      short_id: 'bbb222',
      name: 'redis-cache',
      image: 'redis:7-alpine',
      state: 'paused',
      ports: [{ private_port: 6379, public_port: 6379, protocol: 'tcp' }],
    }),
    makeContainer({
      id: 'c3',
      short_id: 'ccc333',
      name: 'migrations',
      image: 'postgres:16',
      state: 'exited',
    }),
  ];

  it('filters by state bucket', () => {
    assert.deepEqual(filterContainers(sample, '', 'all').map((c) => c.id), ['c1', 'c2', 'c3']);
    assert.deepEqual(filterContainers(sample, '', 'running').map((c) => c.id), ['c1']);
    assert.deepEqual(filterContainers(sample, '', 'paused').map((c) => c.id), ['c2']);
    assert.deepEqual(filterContainers(sample, '', 'stopped').map((c) => c.id), ['c3']);
  });

  it('matches name, image, project, short_id, and published port', () => {
    assert.deepEqual(filterContainers(sample, 'billing').map((c) => c.id), ['c1']);
    assert.deepEqual(filterContainers(sample, 'redis:7').map((c) => c.id), ['c2']);
    assert.deepEqual(filterContainers(sample, '8080').map((c) => c.id), ['c1']);
    assert.deepEqual(filterContainers(sample, 'ccc333').map((c) => c.id), ['c3']);
  });
});

describe('env secret masking helpers', () => {
  it('relies on backend-provided secret and masked flags without duplicating regex heuristics', () => {
    assert.equal(maskEnvValue('hunter2', true, false), SECRET_MASK);
    assert.equal(maskEnvValue('hunter2', true, true), 'hunter2');
    assert.equal(maskEnvValue('production', false, false), 'production');

    const nonSecret: EnvEntry = { key: 'NODE_ENV', value: 'production', secret: false, masked: false };
    assert.deepEqual(displayEnvEntry(nonSecret, new Set()), {
      isSecret: false,
      isMasked: false,
      text: 'production',
    });

    const serverMasked: EnvEntry = { key: 'API_TOKEN', value: SECRET_MASK, secret: true, masked: true };
    const revealedSet = new Set(['API_TOKEN']);
    // Still masked if server has not returned the revealed value yet.
    assert.equal(displayEnvEntry(serverMasked, revealedSet).isMasked, true);

    const serverRevealed: EnvEntry = { key: 'API_TOKEN', value: 'tok_live_123', secret: true, masked: false };
    assert.deepEqual(displayEnvEntry(serverRevealed, new Set()), {
      isSecret: true,
      isMasked: true,
      text: SECRET_MASK,
    });
    assert.deepEqual(displayEnvEntry(serverRevealed, revealedSet), {
      isSecret: true,
      isMasked: false,
      text: 'tok_live_123',
    });
  });
});

describe('port formatting and URLs', () => {
  it('formats published, identical, UDP, and exposed-only ports', () => {
    assert.equal(formatPortMapping({ private_port: 80, public_port: 8080, protocol: 'tcp' }), '8080 → 80');
    assert.equal(formatPortMapping({ private_port: 5432, public_port: 5432, protocol: 'tcp' }), '5432');
    assert.equal(formatPortMapping({ private_port: 53, public_port: 5353, protocol: 'udp' }), '5353 → 53/udp');
    assert.equal(formatPortMapping({ private_port: 9000, public_port: null, protocol: 'tcp' }), '9000');
  });

  it('generates localhost clickable URLs for published TCP ports only', () => {
    assert.equal(portUrl({ ip: '0.0.0.0', private_port: 80, public_port: 8080, protocol: 'tcp' }), 'http://localhost:8080');
    assert.equal(portUrl({ ip: '127.0.0.1', private_port: 3000, public_port: 3000, protocol: 'tcp' }), 'http://127.0.0.1:3000');
    assert.equal(portUrl({ private_port: 53, public_port: 5353, protocol: 'udp' }), null);
    assert.equal(portUrl({ private_port: 80, public_port: null, protocol: 'tcp' }), null);
    assert.equal(
      portUrl({ private_port: 80, public_port: 8080, protocol: 'tcp', url: 'http://web.otto.localhost' }),
      'http://web.otto.localhost',
    );
  });

  it('deduplicates IPv4/IPv6 dual bindings and summarizes compactly', () => {
    const ports: PortMapping[] = [
      { ip: '0.0.0.0', private_port: 80, public_port: 8080, protocol: 'tcp' },
      { ip: '::', private_port: 80, public_port: 8080, protocol: 'tcp' },
      { ip: '0.0.0.0', private_port: 443, public_port: 8443, protocol: 'tcp' },
      { ip: '0.0.0.0', private_port: 9090, public_port: 9090, protocol: 'tcp' },
    ];
    assert.equal(dedupePorts(ports).length, 3);
    assert.equal(formatPortsSummary(ports, 2), '8080 → 80, 8443 → 443 +1');
  });
});

describe('run-modal parsers and resource filters', () => {
  it('parses port specs and rejects malformed entries', () => {
    const ok = parsePortSpecs('8080:80, 127.0.0.1:5432:5432/tcp\n5353:53/udp');
    assert.equal(ok.error, null);
    assert.deepEqual(ok.ports, [
      { host_ip: null, host_port: 8080, container_port: 80, protocol: 'tcp' },
      { host_ip: '127.0.0.1', host_port: 5432, container_port: 5432, protocol: 'tcp' },
      { host_ip: null, host_port: 5353, container_port: 53, protocol: 'udp' },
    ]);

    assert.ok(parsePortSpecs('99999:80').error);
    assert.ok(parsePortSpecs('8080:80/icmp').error);
  });

  it('parses KEY=VALUE env lines and quoted commands', () => {
    const envOk = parseEnvLines('# comment\nNODE_ENV=production\nURL=https://a=b&c=d\n\n');
    assert.equal(envOk.error, null);
    assert.deepEqual(envOk.env, ['NODE_ENV=production', 'URL=https://a=b&c=d']);
    assert.ok(parseEnvLines('MISSING_EQUALS').error);

    assert.deepEqual(parseCommandArgs('sh -c "echo hello world" \'arg 2\''), [
      'sh',
      '-c',
      'echo hello world',
      'arg 2',
    ]);
  });

  it('filters logs, images, volumes, networks and maps status/engine labels', () => {
    const logs = [
      { stream: 'stdout', timestamp: '2026-11-19T10:00:00Z', text: 'server ready' },
      { stream: 'stderr', timestamp: '2026-11-19T10:00:01Z', text: 'connection refused' },
    ];
    assert.equal(filterLogLines(logs, 'refused', 'stderr').length, 1);
    assert.equal(filterLogLines(logs, 'refused', 'stdout').length, 0);
    assert.equal(formatLogsForCopy(logs, false), 'server ready\nconnection refused');

    const img: ImageSummary = {
      id: 'sha256:abcdef123456',
      short_id: 'abcdef123456',
      repo_tags: ['ghcr.io/org/app:v2.1'],
      repo_digests: [],
      created: 1_700_000_000,
      size_bytes: 12_000_000,
      containers: 1,
      dangling: false,
      in_use: true,
    };
    assert.deepEqual(imageDisplayName(img), { repo: 'ghcr.io/org/app', tag: 'v2.1' });
    assert.equal(filterImages([img], 'v2.1', false).length, 1);
    assert.equal(filterImages([img], '', true).length, 0);

    const vol: VolumeSummary = {
      name: 'pgdata',
      driver: 'local',
      mountpoint: '/var/lib/docker/volumes/pgdata/_data',
      scope: 'local',
      labels: {},
      in_use: false,
    };
    assert.equal(filterVolumes([vol], 'pg', true).length, 1);

    const net: NetworkSummary = {
      id: 'n1',
      short_id: 'n1',
      name: 'app_default',
      driver: 'bridge',
      scope: 'local',
      internal: false,
      attachable: true,
      built_in: false,
      subnet: '172.20.0.0/16',
      containers: [{ id: 'c1', name: 'api', ipv4_address: '172.20.0.2/16' }],
      labels: {},
    };
    assert.equal(filterNetworks([net], '172.20').length, 1);
    assert.equal(containerStateInfo('running').tone, 'ok');
    assert.equal(containerHealthInfo('unhealthy')?.tone, 'bad');
    assert.equal(engineKindLabel('kiri'), 'Kiri');
    assert.equal(engineKindLabel('orbstack'), 'OrbStack');

    const baseStatus: ContainersStatusResp = {
      available: false,
      error: 'connection refused',
      active_context: 'kiri',
      socket_path: '/Users/u/.kiri/run/docker.sock',
      engine_kind: 'kiri',
      containers_running: 0,
      containers_paused: 0,
      containers_stopped: 0,
      images_count: 0,
      docker_cli_available: true,
    };
    const installedStopped: RuntimeStatusResp = {
      runtime: 'kiri',
      installed: true,
      running: false,
      daemon_reachable: false,
      vm_state: 'stopped',
      socket_path: '/Users/u/.kiri/run/docker.sock',
      managed_by_otto: false,
      can_start: true,
      can_stop: false,
      guest_assets_present: true,
      log_path: '/Users/u/Library/Application Support/Otto/logs/kiri.log',
      entitlement_note: 'signed',
    };
    assert.equal(runtimeStateInfo(installedStopped).tone, 'warn');
    assert.equal(runtimeStateInfo({ ...installedStopped, vm_state: 'asleep' }).tone, 'info');
    assert.equal(runtimeStateInfo({ ...installedStopped, vm_state: 'running' }).live, true);
    assert.match(buildOfflineDetail(baseStatus, installedStopped), /Start the embedded Kiri runtime/);
    assert.match(
      buildOfflineDetail(baseStatus, {
        ...installedStopped,
        installed: false,
        can_start: false,
        can_install: true,
      }),
      /Install and start the embedded Kiri runtime in one click/,
    );
    assert.match(
      buildOfflineDetail(baseStatus, {
        ...installedStopped,
        installed: false,
        can_start: false,
        can_install: true,
        installing: true,
        install_progress: 'Downloading Kiri binary…',
      }),
      /Downloading Kiri binary…/,
    );
    assert.match(
      buildOfflineDetail(baseStatus, {
        ...installedStopped,
        installed: false,
        can_start: false,
        can_install: true,
        install_error: 'Checksum mismatch on archive',
      }),
      /Checksum mismatch on archive/,
    );
    assert.match(
      buildOfflineDetail(baseStatus, {
        ...installedStopped,
        installed: false,
        can_start: false,
        can_install: false,
        install_hint: 'Install Kiri via brew install kiri',
      }),
      /Install Kiri via brew install kiri/,
    );
  });
});

describe('containers ⌘K command builder', () => {
  it('registers tab, lifecycle, runtime, context switch, and running-container log commands', () => {
    const contexts: DockerContextInfo[] = [
      {
        name: 'desktop-linux',
        socket_path: '/Users/u/.docker/run/docker.sock',
        host: 'unix:///Users/u/.docker/run/docker.sock',
        engine_kind: 'docker_desktop',
        is_active: true,
        is_docker_default: true,
        reachable: true,
      },
      {
        name: 'colima',
        socket_path: '/Users/u/.colima/default/docker.sock',
        host: 'unix:///Users/u/.colima/default/docker.sock',
        engine_kind: 'colima',
        is_active: false,
        is_docker_default: false,
        reachable: true,
      },
    ];
    const runtime: RuntimeStatusResp = {
      runtime: 'kiri',
      installed: true,
      running: true,
      daemon_reachable: true,
      vm_state: 'running',
      socket_path: '/Users/u/.kiri/run/docker.sock',
      managed_by_otto: true,
      can_start: false,
      can_stop: true,
      guest_assets_present: true,
      log_path: '/tmp/kiri.log',
      entitlement_note: 'signed',
    };
    const cmds = buildContainerCommands({
      containers: [
        makeContainer({ id: 'c1', name: 'api', state: 'running' }),
        makeContainer({ id: 'c2', name: 'worker', state: 'exited' }),
      ],
      contexts,
      runtime,
      available: true,
      openTab: () => {},
      selectContainer: () => {},
      openRunModal: () => {},
      pullImage: () => {},
      refresh: () => {},
      systemPrune: () => {},
      selectContext: () => {},
      startRuntime: () => {},
      stopRuntime: () => {},
      restartRuntime: () => {},
    });
    const ids = cmds.map((c) => c.id);
    assert.ok(ids.includes('containers.run'));
    assert.ok(ids.includes('containers.pull'));
    assert.ok(ids.includes('containers.prune'));
    assert.ok(ids.includes('containers.runtime.restart'));
    assert.ok(ids.includes('containers.runtime.stop'));
    assert.ok(!ids.includes('containers.runtime.start'));
    assert.ok(ids.includes('containers.context.colima'));
    assert.ok(!ids.includes('containers.context.desktop-linux'));
    assert.ok(ids.includes('containers.open.c1'));
    assert.ok(ids.includes('containers.logs.c1'));
    assert.ok(ids.includes('containers.open.c2'));
    assert.ok(!ids.includes('containers.logs.c2'));
  });
});

// ── Stateful store & component behavioral tests ───────────────────────────────

describe('ContainersController behavioral flows', () => {
  class FakeApiError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  }

  function loadController(apiOverrides: Record<string, unknown> = {}) {
    const remembered = new Map<string, string>();
    const toastCalls: string[] = [];
    const mod = loadSource(
      new URL('../src/modules/containers/containersStore.svelte.ts', import.meta.url),
      {
        '../../lib/api/containers': {
          containersApi: {
            status: async () => ({
              available: true,
              active_context: 'colima',
              socket_path: '/tmp/colima.sock',
              engine_kind: 'colima',
              containers_running: 1,
              containers_paused: 0,
              containers_stopped: 1,
              images_count: 2,
              docker_cli_available: true,
            }),
            contexts: async () => ({
              active_context: 'colima',
              active_socket: '/tmp/colima.sock',
              contexts: [],
            }),
            runtime: async () => null,
            list: async () => ({
              containers: [
                makeContainer({ id: 'c-stopped', name: 'db', state: 'exited', created: 200 }),
                makeContainer({ id: 'c-running', name: 'api', state: 'running', created: 100 }),
              ],
              stacks: [],
            }),
            ...apiOverrides,
          },
        },
        '../../lib/api/client': { ApiError: FakeApiError },
        '../../lib/confirm.svelte': { confirmer: { ask: async () => true, promptText: async () => null } },
        '../../lib/toast.svelte': {
          toasts: {
            success: (t: string) => toastCalls.push(`ok:${t}`),
            info: (t: string) => toastCalls.push(`info:${t}`),
            warn: (t: string) => toastCalls.push(`warn:${t}`),
          },
        },
        '../../lib/toastError': { toastError: (t: string) => toastCalls.push(`err:${t}`) },
        '../../lib/plural': { plural: (n: number, s: string) => `${n} ${s}${n === 1 ? '' : 's'}` },
        '../../lib/lastSelection': {
          recallSelection: (k: string) => remembered.get(k) ?? null,
          rememberSelection: (k: string, v: string) => remembered.set(k, v),
        },
      },
      {},
      { relative: true },
    );
    return { ctrl: new mod.ContainersController(), remembered, toastCalls };
  }

  it('loads status/containers, picks running container initially, and ignores stale boot responses', async () => {
    const firstStatus = deferred<ContainersStatusResp>();
    const secondStatus = deferred<ContainersStatusResp>();
    let calls = 0;
    const { ctrl, remembered } = loadController({
      status: () => (++calls === 1 ? firstStatus.promise : secondStatus.promise),
    });

    const p1 = ctrl.loadStatusAndContexts();
    const p2 = ctrl.loadStatusAndContexts();

    secondStatus.resolve({
      available: true,
      active_context: 'kiri',
      socket_path: '/tmp/kiri.sock',
      engine_kind: 'kiri',
      containers_running: 1,
      containers_paused: 0,
      containers_stopped: 0,
      images_count: 1,
      docker_cli_available: true,
    });
    assert.equal(await p2, true);
    assert.equal(ctrl.status?.active_context, 'kiri');

    // Stale first response must not overwrite the newer status.
    firstStatus.resolve({
      available: false,
      active_context: 'stale',
      socket_path: '/tmp/stale.sock',
      engine_kind: 'unknown',
      containers_running: 0,
      containers_paused: 0,
      containers_stopped: 0,
      images_count: 0,
      docker_cli_available: false,
    });
    assert.equal(await p1, false);
    assert.equal(ctrl.status?.active_context, 'kiri');

    await ctrl.loadContainers();
    assert.equal(ctrl.selectedId, 'c-running');
    assert.equal(remembered.get('containers.selected'), 'c-running');
    assert.equal(ctrl.containersError, null);
  });

  it('surfaces 404 as unavailable and list errors inline', async () => {
    const { ctrl } = loadController({
      status: async () => {
        throw new FakeApiError(404, 'Not Found');
      },
      list: async () => {
        throw new Error('socket closed');
      },
    });

    assert.equal(await ctrl.loadStatusAndContexts(), false);
    assert.equal(ctrl.unavailable, true);
    assert.equal(ctrl.statusError, null);

    await ctrl.loadContainers();
    assert.ok(ctrl.containersError?.includes('socket closed'));
    assert.equal(ctrl.containersLoading, false);
  });
});

describe('ContainerTerminalTab openShell race across container switches', () => {
  it('discards stale exec session when containerId switches while openShell is in flight', async () => {
    const execA = deferred<{ id: string }>();
    const execB = deferred<{ id: string }>();
    const deletedSessions: string[] = [];

    const state = componentFunctions(
      new URL('../src/modules/containers/ContainerTerminalTab.svelte', import.meta.url),
      ['openShell', 'closeShell'],
      {
        containerId: 'container-A',
        running: true,
        sessionId: null as string | null,
        status: null as string | null,
        opening: false,
        error: '',
        customCommand: '',
        openSeq: 0,
        ws: { currentId: 'ws-1' },
        NO_WORKSPACE: 'No workspace selected',
        parseCommandArgs,
        loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
        containersApi: {
          exec: (id: string) => (id === 'container-A' ? execA.promise : execB.promise),
        },
        api: {
          del: async (path: string) => {
            deletedSessions.push(path);
          },
        },
      },
    );

    const openA = state.openShell();
    assert.equal(state.opening, true);

    // Simulate switching to Container B ($effect cleanup increments openSeq and resets state).
    state.openSeq += 1;
    state.containerId = 'container-B';
    state.opening = false;
    state.error = '';
    state.customCommand = '';

    // Container A's exec resolves late: must NOT mount in Container B's tab and MUST delete sess-A.
    execA.resolve({ id: 'sess-A' });
    await openA;
    assert.equal(state.sessionId, null);
    assert.deepEqual(deletedSessions, ['/sessions/sess-A']);

    // Opening a shell on Container B succeeds cleanly.
    const openB = state.openShell();
    execB.resolve({ id: 'sess-B' });
    await openB;
    assert.equal(state.sessionId, 'sess-B');
    assert.equal(state.opening, false);
  });
});

describe('ContainerDetail toggleSecret across container switches', () => {
  function makeDetail(id: string, secretValue: string, masked: boolean): ContainerDetail {
    return {
      id,
      short_id: id.slice(0, 12),
      name: id,
      image: 'nginx:alpine',
      image_id: 'sha256:111',
      created: '2026-11-19T00:00:00Z',
      state: 'running',
      status: 'running',
      running: true,
      paused: false,
      restarting: false,
      oom_killed: false,
      pid: 100,
      exit_code: 0,
      restart_policy: 'no',
      entrypoint: [],
      cmd: [],
      working_dir: '/',
      hostname: id,
      ports: [],
      env: [{ key: 'DB_PASSWORD', value: secretValue, secret: true, masked }],
      mounts: [],
      networks: [],
      labels: {},
    };
  }

  it('calls revealSecrets and ignores stale response after switching containers', async () => {
    const revealA = deferred<ContainerDetail>();
    const revealB = deferred<ContainerDetail>();
    const revealCalls: string[] = [];

    const state = componentFunctions(
      new URL('../src/modules/containers/ContainerDetail.svelte', import.meta.url),
      ['toggleSecret'],
      {
        container: makeContainer({ id: 'container-A', name: 'a' }),
        detail: makeDetail('container-A', SECRET_MASK, true),
        secretsLoaded: false,
        revealingSecret: false,
        revealedKeys: new Set<string>(),
        loadSeq: 1,
        toastError: () => {},
        containersApi: {
          revealSecrets: (id: string) => {
            revealCalls.push(id);
            return id === 'container-A' ? revealA.promise : revealB.promise;
          },
        },
      },
    );

    const pendingA = state.toggleSecret('DB_PASSWORD');
    assert.equal(state.revealingSecret, true);
    assert.deepEqual(revealCalls, ['container-A']);

    // Switch to Container B before revealA completes.
    state.container = makeContainer({ id: 'container-B', name: 'b' });
    state.detail = makeDetail('container-B', SECRET_MASK, true);
    state.secretsLoaded = false;
    state.revealedKeys = new Set<string>();
    state.loadSeq += 1;

    revealA.resolve(makeDetail('container-A', 'secret-A', false));
    await pendingA;
    assert.equal(state.detail.id, 'container-B');
    assert.equal(state.secretsLoaded, false);
    assert.equal(state.revealedKeys.has('DB_PASSWORD'), false);

    // Now reveal on Container B cleanly.
    const pendingB = state.toggleSecret('DB_PASSWORD');
    revealB.resolve(makeDetail('container-B', 'secret-B', false));
    await pendingB;
    assert.equal(state.detail.env[0].value, 'secret-B');
    assert.equal(state.secretsLoaded, true);
    assert.equal(state.revealedKeys.has('DB_PASSWORD'), true);
    assert.equal(state.revealingSecret, false);
  });
});

describe('RunContainerModal initialImage seeding and clearing', () => {
  it('seeds image from initialImage once without re-filling when the user clears the input', async () => {
    const runCalls: unknown[] = [];
    const state = componentFunctions(
      new URL('../src/modules/containers/RunContainerModal.svelte', import.meta.url),
      ['syncInitialImage', 'clearError', 'submit'],
      {
        image: 'nginx:alpine',
        seededInitialImage: 'nginx:alpine',
        name: '',
        portsText: '',
        envText: '',
        commandText: '',
        volumesText: '',
        workingDir: '',
        restartPolicy: 'no',
        autoRemove: false,
        pullIfMissing: true,
        submitting: false,
        fieldErrors: {} as Record<string, string>,
        parsePortSpecs,
        parseEnvLines,
        parseCommandArgs,
        containersApi: {
          run: async (body: unknown) => {
            runCalls.push(body);
            return { id: 'c1234567890ab', short_id: 'c1234567890a', warnings: [] };
          },
        },
        toasts: { success: () => {}, warn: () => {} },
        toastError: () => {},
        oncreated: () => {},
      },
    );

    // User clears the Image input: syncing with the unchanged initialImage must NOT re-fill it.
    state.image = '';
    state.syncInitialImage('nginx:alpine');
    assert.equal(state.image, '');

    // Submitting with cleared image produces an inline error instead of running the old initialImage.
    await state.submit();
    assert.ok(state.fieldErrors.image);
    assert.equal(runCalls.length, 0);

    // If parent explicitly updates initialImage to a new image, it seeds once and submits cleanly.
    state.syncInitialImage('postgres:16');
    assert.equal(state.image, 'postgres:16');
    state.clearError('image');
    await state.submit();
    assert.equal(runCalls.length, 1);
    assert.equal((runCalls[0] as { image: string }).image, 'postgres:16');
  });
});
