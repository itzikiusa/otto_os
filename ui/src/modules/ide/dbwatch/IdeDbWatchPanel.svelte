<script lang="ts">
  // DB Watch: what did my code just write? ONE toolbar row — Target ▾ (with its
  // environment), Tables ▾ (ranked, checkable, "Add table…" from the target's
  // real tables), Since last stop | Since start, one primary (Snapshot / Start
  // watching) and ⋯ — then a section per watched table. The daemon takes the
  // checkpoints (every debugger stop, a run's end); the store arms the watch
  // and refetches, so this panel only reads and steers (ideDbWatchState).
  import { tick, untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import EnvBadge from '../../../lib/components/EnvBadge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { plural } from '../../../lib/plural';
  import { pollWhileVisible } from '../../../lib/poll';
  import { router } from '../../../lib/router.svelte';
  import type { IdeDbWatchItemDiff, IdeDbWatchSnapshotItem, IdeDbWatchTarget } from '../../../lib/api/types/ide-db-watch';
  import { ide, ideBasename } from '../ideStore.svelte';
  import { ideLayout } from '../shell/ideLayout.svelte';
  import IdeDbWatchTable from './IdeDbWatchTable.svelte';
  import {
    canCompareStart,
    changesAsText,
    engineLabel,
    filterText,
    isDbWatchPanelVisible,
    isProdTarget,
    MAX_WATCH_ITEMS,
    parseFilter,
    pickAutoItems,
    pickAutoTarget,
    targetKeyOf,
    targetPlace,
    unverifiedNames,
  } from './ideDbWatchModel';
  import { ideDbWatch } from './ideDbWatchState.svelte';

  const w = $derived(ideDbWatch.watch);
  const read = $derived(ideDbWatch.read);
  const pending = $derived(ideDbWatch.pendingTarget);
  const engine = $derived(w?.target.engine ?? pending?.engine ?? null);
  const items = $derived<IdeDbWatchItemDiff[]>(read?.items ?? []);
  const changedItems = $derived(items.filter((i) => i.error || i.added_count + i.modified_count + i.removed_count > 0));
  const quietItems = $derived(items.filter((i) => !i.error && i.added_count + i.modified_count + i.removed_count === 0));
  const waiting = $derived(Boolean(read && read.from_seq === null));
  const prod = $derived(isProdTarget(w?.target));
  /** "Show current rows" per table. */
  let currentRows = $state<Record<string, IdeDbWatchSnapshotItem | null>>({});
  let bodyEl: HTMLElement | undefined = $state();
  let targetBtn: HTMLButtonElement | undefined = $state();
  let tablesBtn: HTMLButtonElement | undefined = $state();

  const targetLabel = $derived(w?.target.label ?? pending?.name ?? 'Target');
  const targetEnv = $derived(w?.target.environment ?? (pending?.target_kind === 'saved' ? pending.environment : null));
  const tablesLabel = $derived.by(() => {
    const names = (w?.items ?? []).map((i) => i.object);
    if (names.length === 0) return 'Tables';
    return names.length <= 2 ? names.join(', ') : `${names.slice(0, 2).join(', ')} +${names.length - 2}`;
  });
  const caption = $derived.by(() => {
    const l = w?.latest;
    if (!l) return '';
    const where = l.file && l.line ? ` · ${ideBasename(l.file)}:${l.line}` : '';
    const at = new Date(l.captured_at_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    const what = l.seq === 0 ? 'Baseline' : l.reason === 'manual' ? `Snapshot ${l.seq}` : l.reason === 'stop' ? `Stop ${l.seq}` : l.reason === 'run_end' ? 'Run ended' : l.reason === 'session_end' ? 'Session ended' : `Checkpoint ${l.seq}`;
    return `${what}${where} · ${at}`;
  });

  // The strip in the Debug panel asks for one table: bring it into view.
  $effect(() => {
    const focus = ideDbWatch.focusObject;
    if (!focus || !bodyEl || items.length === 0) return;
    untrack(() => {
      ideDbWatch.focusObject = '';
      void tick().then(() => bodyEl?.querySelector(`[data-object="${CSS.escape(focus)}"]`)?.scrollIntoView({ block: 'start' }));
    });
  });

  const panelVisible = $derived(
    isDbWatchPanelVisible({ stack: ideLayout.stack, stackPane: ideLayout.pane, bottomDockOpen: ide.bottomDockOpen, activeBottomTab: ide.activeBottomTab }),
  );
  // Optional 3 s poll (⋯), only while this panel shows and never on prod.
  $effect(() => {
    if (!ideDbWatch.liveWatch || !panelVisible || prod || !w) return;
    const poller = pollWhileVisible(async () => { if (!untrack(() => ideDbWatch.refreshing)) await ideDbWatch.snapshotNow(); }, { ms: 3000, floorMs: 2000, immediate: false });
    return () => poller.stop();
  });

  function codeRefOf(object: string): { file: string; line: number } | null {
    const d = ideDbWatch.detected.find((x) => x.object_name === object);
    return d ? { file: d.source_file, line: d.line } : null;
  }

  function targetHint(t: IdeDbWatchTarget): string {
    const where = `${engineLabel(t.engine)} · ${t.host}:${t.port}`;
    if (t.target_kind === 'saved') return [t.environment || 'dev', t.local ? 'local' : 'remote', where].join(' · ');
    return t.container_image ? `${t.container_image} · :${t.port}` : where;
  }

  async function openTargetMenu(): Promise<void> {
    await ideDbWatch.discover();
    const cur = w ? (w.target.kind === 'saved' ? `saved:${w.target.connection_id}` : `ephemeral:${w.target.ephemeral_id}`) : pending ? targetKeyOf(pending) : '';
    const row = (t: IdeDbWatchTarget): MenuItem => ({
      label: t.name,
      hint: targetHint(t),
      checked: targetKeyOf(t) === cur,
      danger: t.target_kind === 'saved' && isProdTarget(t),
      action: () => void ideDbWatch.setTarget(t),
    });
    const run = ideDbWatch.ephemeralTargets.filter((t) => t.source !== 'container');
    const other = ideDbWatch.ephemeralTargets.filter((t) => t.source === 'container');
    const groups: MenuItem[][] = [run.map(row), ideDbWatch.savedConnections.map(row), other.map(row)].filter((g) => g.length > 0);
    const list: MenuItem[] = groups.flatMap((g, i) => (i === 0 ? g : [{ separator: true }, ...g]));
    if (list.length === 0) {
      list.push({ label: ideDbWatch.detectError ?? 'No databases found: start a Testcontainer or add a saved connection', disabled: true });
    }
    ctxMenu.showAt(targetBtn ?? null, list, { filter: list.length > 12, filterPlaceholder: 'Find a database…' });
  }

  async function openTablesMenu(): Promise<void> {
    if (!engine) return;
    await ideDbWatch.discover({ target: pending ?? undefined });
    showTablesMenu();
  }

  function showTablesMenu(): void {
    if (!engine) return;
    const watched = new Set((ideDbWatch.watch?.items ?? []).map((i) => i.object));
    const full = watched.size >= MAX_WATCH_ITEMS;
    const ranked = pickAutoItems(ideDbWatch.detected, engine, true);
    const extra = [...watched].filter((o) => !ranked.some((r) => r.object === o)).map((object) => ({ object }));
    const rows: MenuItem[] = [...ranked, ...extra].map(({ object }) => {
      const d = ideDbWatch.detected.find((x) => x.object_name === object && x.engine === engine);
      const on = watched.has(object);
      return {
        label: object,
        checked: on,
        disabled: !on && full,
        hint: d ? `${d.operations.join('/').toUpperCase() || 'READ'} · ${ideBasename(d.source_file)}:${d.line}` : undefined,
        action: () => void (on ? ideDbWatch.removeItem(object) : ideDbWatch.addItem({ object })).then(showTablesMenu),
      };
    });
    const missing = unverifiedNames(ideDbWatch.detected, engine);
    const db = ideDbWatch.watch?.database ?? pending?.database ?? 'the target';
    const list: MenuItem[] = [
      { label: engine === 'redis' ? 'Add key prefix…' : engine === 'mongodb' ? 'Add collection…' : 'Add table…', icon: 'plus', pinned: true, disabled: full, action: () => void openObjectsMenu() },
      ...(engine === 'redis' ? [{ label: 'Key pattern…', icon: 'edit', pinned: true, disabled: full, action: () => void addPattern() } as MenuItem] : []),
      { separator: true, pinned: true },
      ...rows,
    ];
    if (rows.length === 0) list.push({ label: 'No tables found in the code yet', disabled: true });
    if (missing.length > 0) {
      list.push({ separator: true }, { label: `${plural(missing.length, 'name')} in code not found in ${db}`, disabled: true, title: missing.join(', ') });
    }
    ctxMenu.showAt(tablesBtn ?? null, list, { filter: rows.length > 10, filterPlaceholder: 'Find a table…' });
  }

  async function openObjectsMenu(): Promise<void> {
    await ideDbWatch.loadObjects();
    const watched = new Set((ideDbWatch.watch?.items ?? []).map((i) => i.object));
    const list: MenuItem[] = ideDbWatch.objects
      .filter((o) => !watched.has(o.name))
      .map((o) => ({ label: o.name, action: () => void ideDbWatch.addItem({ object: o.name }) }));
    if (list.length === 0) list.push({ label: ideDbWatch.detectError ?? 'Nothing else to add', disabled: true });
    ctxMenu.showAt(tablesBtn ?? null, list, { filter: true, filterPlaceholder: 'Find a table…', maxVisible: 60 });
  }

  async function addPattern(initial = '', replace?: string): Promise<void> {
    const v = await confirmer.promptText('Keys to watch: a prefix (session:) or a glob pattern (session:*:route).', {
      title: replace ? 'Change key pattern' : 'Watch Redis keys',
      confirmLabel: 'Watch',
      initial,
      placeholder: 'session:*',
    });
    if (!v) return;
    if (replace) await ideDbWatch.replaceItem(replace, { object: v });
    else await ideDbWatch.addItem({ object: v });
  }

  async function editFilter(object: string): Promise<void> {
    const info = ideDbWatch.watch?.items.find((i) => i.object === object);
    const v = await confirmer.promptText(`Only rows of ${object} where a column equals a value. Leave empty to watch every row.`, {
      title: 'Filter rows',
      confirmLabel: 'Apply',
      initial: filterText(info?.filter),
      placeholder: "status = 'open'",
      allowEmpty: true,
      validate: (s) => (s === '' || parseFilter(s) ? null : 'Write it as column = value, e.g. account_id = 77017 (text: letters, digits and _ . : @ -)'),
    });
    if (v === null) return;
    await ideDbWatch.replaceItem(object, v === '' ? { object } : { object, filter: parseFilter(v) });
  }

  async function toggleCurrent(object: string): Promise<void> {
    if (currentRows[object]) {
      const { [object]: _, ...rest } = currentRows;
      currentRows = rest;
      return;
    }
    const resp = await ideDbWatch.diffSince(ideDbWatch.compare, { rows: true });
    const it = resp?.items.find((i) => i.object === object);
    currentRows = { ...currentRows, [object]: it?.current ?? { columns: it?.columns ?? [], rows: [] } };
  }

  function openTableMenu(e: MouseEvent, it: IdeDbWatchItemDiff): void {
    const redis = it.engine === 'redis';
    const menu: MenuItem[] = [
      redis
        ? { label: 'Change key pattern…', icon: 'edit', action: () => void addPattern(it.object, it.object) }
        : { label: 'Filter rows…', icon: 'filter', action: () => void editFilter(it.object) },
      { label: currentRows[it.object] ? 'Show changes' : 'Show current rows', icon: 'listBullet', action: () => void toggleCurrent(it.object) },
      { label: 'Copy query', icon: 'copy', action: () => void copyWithToast(it.executed_query, 'query') },
      { separator: true },
      { label: 'Stop watching this table', icon: 'x', action: () => void ideDbWatch.removeItem(it.object) },
    ];
    ctxMenu.show(e, menu);
  }

  /** The primary when nothing is armed: the run's own database + the tables the
   *  code touches, else the menus that are missing a choice. */
  async function startWatching(): Promise<void> {
    if (pending) return void openTablesMenu();
    await ideDbWatch.discover();
    const t = pickAutoTarget(ideDbWatch.detected, ideDbWatch.ephemeralTargets);
    if (!t) return void openTargetMenu();
    await ideDbWatch.discover({ target: t });
    const picked = pickAutoItems(ideDbWatch.detected, t.engine, true);
    if (picked.length === 0) {
      await ideDbWatch.setTarget(t);
      return void openTablesMenu();
    }
    await ideDbWatch.start({ target: { kind: 'ephemeral', ephemeral_id: t.ephemeral_id }, database: t.database, items: picked });
  }

  function openMore(e: MouseEvent): void {
    const menu: MenuItem[] = [
      {
        label: 'Live poll every 3 s',
        checked: ideDbWatch.liveWatch,
        disabled: !w || prod,
        title: prod ? 'Not on a production database' : !w ? 'Start watching first' : 'Take a snapshot every 3 seconds while this panel shows',
        action: () => (ideDbWatch.liveWatch = !ideDbWatch.liveWatch),
      },
      { label: 'Keep Testcontainers alive after a run', checked: ide.keepTestcontainersAlive, action: () => (ide.keepTestcontainersAlive = !ide.keepTestcontainersAlive) },
      { separator: true },
      { label: 'Copy changes as text', icon: 'copy', disabled: changedItems.length === 0, action: () => void copyWithToast(changesAsText(items), 'changes') },
    ];
    if (w?.target.kind === 'saved') menu.push({ label: 'Open in DB Explorer', icon: 'external', action: () => router.go('database') });
    menu.push({ separator: true }, { label: 'Stop watching', icon: 'stop', disabled: !w, action: () => void ideDbWatch.stop() });
    ctxMenu.show(e, menu);
  }
</script>

<div class="dbw" data-testid="ide-dbwatch-panel">
  <header class="dbw-bar">
    <button type="button" class="btn small ghost dbw-pick" bind:this={targetBtn} aria-haspopup="menu" title={w ? `${w.target.label} · ${targetPlace(w.target)}` : 'Choose the database to watch'} onclick={() => void openTargetMenu()} data-testid="ide-dbw-target">
      <Icon name="db" size={12} />
      <span class="dbw-pick-label">{targetLabel}</span>
      {#if targetEnv}<EnvBadge env={targetEnv} />{/if}
      <Icon name="chevronDown" size={12} />
    </button>
    <button type="button" class="btn small ghost dbw-pick" bind:this={tablesBtn} aria-haspopup="menu" disabled={!engine} title={engine ? 'Choose the tables to watch' : 'Choose a target first'} onclick={() => void openTablesMenu()} data-testid="ide-dbw-tables">
      <span class="dbw-pick-label mono" dir="ltr">{tablesLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>
    <span class="dbw-caption" title={caption}>
      {#if ideDbWatch.refreshing || w?.busy}<span class="spinner dbw-spin" aria-hidden="true"></span>{/if}
      {caption}
    </span>
    {#if canCompareStart(w)}
      <div class="segmented dbw-seg" role="group" aria-label="Compare with">
        <button type="button" class:active={ideDbWatch.compare === 'previous_stop'} aria-pressed={ideDbWatch.compare === 'previous_stop'} onclick={() => void ideDbWatch.setCompare('previous_stop')} data-testid="ide-dbw-since-last">Since last stop</button>
        <button type="button" class:active={ideDbWatch.compare === 'start'} aria-pressed={ideDbWatch.compare === 'start'} onclick={() => void ideDbWatch.setCompare('start')} data-testid="ide-dbw-since-start">Since start</button>
      </div>
    {/if}
    {#if w}
      <button type="button" class="btn small primary" disabled={w.busy} title="Take a checkpoint now and compare" onclick={() => void ideDbWatch.snapshotNow()} data-testid="ide-dbw-snapshot">Snapshot</button>
    {:else}
      <button type="button" class="btn small primary" title="Watch the run’s database and the tables your code touches" onclick={() => void startWatching()} data-testid="ide-dbw-start">Start watching</button>
    {/if}
    <button type="button" class="icon-btn" aria-label="DB Watch options" title="Live poll, keep containers, copy changes, stop watching" aria-haspopup="menu" onclick={openMore} data-testid="ide-dbw-options-btn">
      <Icon name="more" size={13} />
    </button>
  </header>

  <div class="dbw-body" bind:this={bodyEl}>
    <LoadState
      what="the database watch"
      errorTitle={w ? `Couldn’t read ${w.target.label}` : 'Couldn’t start watching'}
      loading={ideDbWatch.loading && !read}
      error={ideDbWatch.error}
      empty={!w}
      onretry={() => void (w ? ideDbWatch.snapshotNow() : startWatching())}
      rows={4}
    >
      {#snippet errorActions()}
        {#if (w?.target.kind ?? pending?.target_kind) === 'ephemeral' && !ide.keepTestcontainersAlive}
          <button type="button" class="btn small" onclick={() => (ide.keepTestcontainersAlive = true)}>Keep Testcontainers alive</button>
        {/if}
      {/snippet}
      {#snippet emptyView()}
        {#if pending}
          <EmptyState icon="db" title="Choose tables to watch" body="Pick the tables {pending.name} should compare at every stop." actionLabel="Tables" actionIcon="listBullet" actionKind="secondary" onaction={() => void openTablesMenu()} />
        {:else}
          <EmptyState icon="db" title="No database to watch" body="Debug or run a test that starts a Testcontainer, or pick a saved connection." actionLabel="Choose target" actionIcon="db" actionKind="secondary" onaction={() => void openTargetMenu()} />
        {/if}
      {/snippet}

      {#if read && w}
        {#if waiting}
          <p class="dbw-note" data-testid="ide-dbw-armed">
            Watching {plural(w.items.length, 'table')} on {w.target.label}. Changes appear at the next stop.
          </p>
        {:else if changedItems.length === 0}
          <p class="dbw-note" data-testid="ide-dbw-no-changes">
            No changes since {ideDbWatch.compare === 'start' ? 'the start' : read.from_seq === 0 ? 'the baseline' : `stop ${read.from_seq}`} in {plural(items.length, 'table')}{w.target.engine === 'mysql' || w.target.engine === 'postgres' ? ' (writes the paused code has not committed are not visible yet)' : ''}.
          </p>
        {/if}
        {#each waiting ? items : changedItems as it (it.object)}
          <IdeDbWatchTable
            item={it}
            info={w.items.find((i) => i.object === it.object)}
            current={currentRows[it.object] ?? null}
            codeRef={codeRefOf(it.object)}
            {waiting}
            onmenu={(e) => openTableMenu(e, it)}
            onretry={() => void ideDbWatch.snapshotNow()}
            onjump={() => { const r = codeRefOf(it.object); if (r) void ide.openFile(r.file, { line: r.line, preview: false }); }}
          />
        {/each}
        {#if !waiting && changedItems.length > 0 && quietItems.length > 0}
          <p class="dbw-note dbw-quiet" data-testid="ide-dbw-quiet">No changes: {quietItems.map((i) => i.object).join(', ')}</p>
        {/if}
      {/if}
    </LoadState>
  </div>
</div>

<style>
  .dbw { container-type: inline-size; display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
  .dbw-bar {
    display: flex; align-items: center; gap: var(--sp-2);
    block-size: 32px; min-block-size: 32px; padding-inline: var(--sp-3);
    border-block-end: 1px solid var(--border); flex-shrink: 0; overflow: hidden;
  }
  .dbw-pick { display: inline-flex; align-items: center; gap: var(--sp-2); block-size: 24px; min-inline-size: 0; max-inline-size: 34%; flex: 0 1 auto; }
  .dbw-pick-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .dbw-caption { display: inline-flex; align-items: center; gap: var(--sp-2); flex: 1 1 0; min-inline-size: 0; color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dbw-spin { --spinner-size: 12px; flex-shrink: 0; }
  .dbw-seg { block-size: 24px; flex-shrink: 0; }
  .dbw-seg button { padding-block: 0; padding-inline: var(--sp-4); font-size: var(--fs-xs); white-space: nowrap; }
  .dbw-bar > :global(.btn.primary), .dbw-bar > :global(.icon-btn) { flex-shrink: 0; }
  .dbw-body { flex: 1; min-block-size: 0; min-inline-size: 0; overflow: auto; }
  .dbw-note { margin: 0; padding-block: var(--sp-3); padding-inline: var(--sp-4); color: var(--text-dim); font-size: var(--fs-xs); border-block-end: 1px solid var(--separator); }
  .dbw-quiet { border-block-end: 0; }
  @container (max-width: 620px) { .dbw-caption { display: none; } }
  @container (max-width: 480px) { .dbw-seg { display: none; } }
</style>
