// Machine resources: the status-bar chip's state, the Daemon page's live line
// and its settings form (read as the daemon parses, refused where the daemon
// would silently fall back), plus the Usage page's outage banner lines.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  backgroundLine,
  chipState,
  detailRows,
  formatGiB,
  formatWait,
  queuedLine,
  sourceLabel,
  statusLine,
} from '../src/modules/settings/machineStatusText.ts';
import {
  defaultMaxLive,
  machineBody,
  machineFormError,
  machineFormErrors,
  machineFormFrom,
  sameMachineForm,
  type MachineForm,
} from '../src/modules/settings/machineSettings.ts';
import { usageHealthLines, outageTime } from '../src/modules/usage/usageHealth.ts';
import { availableSections, filterSections, matchedSetting, findSection } from '../src/modules/settings/sections.ts';
import type { MachineStatus } from '../src/lib/api/types/machine.ts';

const GIB = 1024 ** 3;
const plain = <T>(x: T): T => JSON.parse(JSON.stringify(x));

function status(over: Partial<MachineStatus> = {}): MachineStatus {
  return {
    load1: 3.1,
    cores: 8,
    load_per_core: 0.3875,
    memory_pressure: 'normal',
    swap_used_bytes: 512 * 1024 * 1024,
    admission_enabled: true,
    max_load_per_core: 1.5,
    max_wait_secs: 300,
    background_live: 2,
    background_max_live: 6,
    agents_live: 3,
    busy: false,
    busy_reason: null,
    queued: [],
    disk_free_bytes: 142 * GIB,
    low_disk_gb: 30,
    low_disk: false,
    ...over,
  };
}

test('status line reads like the spec sentence', () => {
  assert.equal(
    statusLine(status()),
    'Now: load 3.1 on 8 cores · memory normal · 2 of 6 background agents · nothing queued · 142 GB free',
  );
  const busy = status({ memory_pressure: 'warn', queued: [{ source: 'review', waiting_secs: 95, reason: null }], disk_free_bytes: undefined });
  assert.equal(statusLine(busy), 'Now: load 3.1 on 8 cores · memory high · 2 of 6 background agents · 1 queued');
  assert.equal(statusLine(status({ load1: null, cores: 1, disk_free_bytes: null })), 'Now: load unknown on 1 core · memory normal · 2 of 6 background agents · nothing queued');
  assert.equal(backgroundLine(status({ background_max_live: null, background_live: 1 })), '1 background agent (no limit)');
});

test('chip: icon alone when calm, warning text when busy or low on disk', () => {
  assert.equal(chipState(null), null, 'nothing before the first read');
  const calm = chipState(status())!;
  assert.equal(calm.tone, 'calm');
  assert.equal(calm.text, '');
  assert.match(calm.label, /^Machine: load 3\.1 on 8 cores, memory normal, 2 of 6 background agents\. Show details$/);

  const busy = chipState(status({ busy: true, busy_reason: 'memory pressure is high', queued: [
    { source: 'review', waiting_secs: 95, reason: 'memory pressure is high' },
    { source: 'workflow', waiting_secs: 40, reason: null },
  ] }))!;
  assert.deepEqual(plain(busy).tone, 'busy');
  assert.equal(busy.text, 'Machine busy · 2 queued');
  assert.match(busy.label, /Machine busy \(memory pressure is high\).*2 agents queued/);
  assert.equal(chipState(status({ busy: true }))!.text, 'Machine busy');
  // Agents still draining the queue on a calmer machine: still busy.
  assert.equal(chipState(status({ queued: [{ source: 'review', waiting_secs: 3, reason: null }] }))!.text, 'Machine busy · 1 queued');

  const low = chipState(status({ low_disk: true, disk_free_bytes: 12 * GIB }))!;
  assert.equal(low.tone, 'low-disk');
  assert.equal(low.text, 'Low disk · 12.0 GB free');
  assert.match(low.label, /warning below 30 GB/);
  // Busy wins the text; the label still says the disk is low.
  const both = chipState(status({ busy: true, low_disk: true, disk_free_bytes: 12 * GIB }))!;
  assert.equal(both.tone, 'busy');
  assert.match(both.label, /Only 12\.0 GB free on disk/);
});

test('details: every number the chip summarises, and each queued agent with why', () => {
  const rows = detailRows(status({ swap_used_bytes: null }));
  assert.deepEqual(rows.map((r) => r.label), ['Load', 'Memory pressure', 'Swap in use', 'Background agents', 'All agents running', 'Free disk']);
  assert.match(rows[0].value, /3\.1 on 8 cores \(0\.39 per core; queueing above 1\.5\)/);
  assert.equal(rows[2].value, 'unknown');
  assert.equal(rows[5].value, '142 GB (warning below 30 GB)');
  assert.equal(detailRows(status({ admission_enabled: false }))[3].value, '2 running (queueing is off)');
  // An older daemon without the disk fields: no disk row.
  assert.equal(detailRows(status({ disk_free_bytes: undefined })).length, 5);
  assert.equal(queuedLine({ source: 'review', waiting_secs: 95, reason: 'load average 13.0 is above 12.0' }), 'Code review · waiting 1 min · load average 13.0 is above 12.0');
  assert.equal(queuedLine({ source: 'some_new_engine', waiting_secs: 4000, reason: null }), 'some new engine · waiting 1 h 6 min');
  assert.equal(sourceLabel('scheduled_task'), 'Scheduled task');
  assert.equal(formatWait(40), '40 s');
  assert.equal(formatWait(3600), '1 h');
  assert.equal(formatGiB(null), 'unknown');
  assert.equal(formatGiB(29.55 * GIB), '29.6 GB');
});

test('settings form reads values exactly as the daemon parses them', () => {
  const empty = plain(machineFormFrom({}));
  assert.deepEqual(empty, {
    admission: true, maxLive: null, maxLoadPerCore: null, maxWaitSecs: null, lowPriority: true,
    buildMode: 'auto', buildWorkers: null, bgArchiveDays: 7, lowDiskGb: null,
  });
  const set = plain(machineFormFrom({
    agent_admission_enabled: false, agent_background_max_live: 0, agent_admission_max_load_per_core: 2.5,
    agent_admission_max_wait_secs: 60, agent_background_low_priority: false, agent_build_parallelism: '4',
    session_auto_archive_background_days: 0, machine_low_disk_gb: 50,
  }));
  assert.deepEqual(set, {
    admission: false, maxLive: 0, maxLoadPerCore: 2.5, maxWaitSecs: 60, lowPriority: false,
    buildMode: 'fixed', buildWorkers: 4, bgArchiveDays: 0, lowDiskGb: 50,
  });
  // The daemon's fallbacks: junk → automatic / default; "off" / false → no budget;
  // a stored non-number archive day count reads as OFF (the daemon does).
  const junk = plain(machineFormFrom({
    agent_admission_enabled: 'yes', agent_background_max_live: 'many', agent_admission_max_load_per_core: -1,
    agent_build_parallelism: 'OFF', session_auto_archive_background_days: 'weekly', machine_low_disk_gb: -5,
  }));
  assert.equal(junk.admission, true);
  assert.equal(junk.maxLive, null);
  assert.equal(junk.maxLoadPerCore, null);
  assert.equal(junk.buildMode, 'off');
  assert.equal(junk.bgArchiveDays, 0);
  assert.equal(junk.lowDiskGb, null);
  assert.equal(machineFormFrom({ agent_build_parallelism: 0 }).buildMode, 'auto');
  assert.equal(machineFormFrom({ agent_build_parallelism: false }).buildMode, 'off');
  assert.equal(defaultMaxLive(8), 6);
  assert.equal(defaultMaxLive(2), 2);
  assert.equal(defaultMaxLive(16), 12);
});

test('settings validators refuse what the daemon would quietly replace', () => {
  const base = machineFormFrom({});
  const err = (over: Partial<MachineForm>) => machineFormErrors({ ...base, ...over });
  assert.deepEqual(plain(err({})), {});
  assert.equal(machineFormError(base), '');
  assert.ok(err({ maxLive: -1 }).maxLive);
  assert.ok(err({ maxLive: 2.5 }).maxLive);
  assert.ok(!err({ maxLive: 0 }).maxLive, '0 = no ceiling');
  assert.ok(!err({ maxLive: undefined }).maxLive, 'blank = automatic');
  assert.ok(err({ maxLoadPerCore: 0 }).maxLoadPerCore);
  assert.ok(!err({ maxLoadPerCore: 0.8 }).maxLoadPerCore);
  assert.ok(err({ maxWaitSecs: 1.5 }).maxWaitSecs);
  assert.ok(err({ buildMode: 'fixed', buildWorkers: null }).buildWorkers);
  assert.ok(err({ buildMode: 'fixed', buildWorkers: 0 }).buildWorkers);
  assert.ok(!err({ buildMode: 'fixed', buildWorkers: 3 }).buildWorkers);
  assert.ok(err({ bgArchiveDays: null }).bgArchiveDays, 'never blank: a stored blank would switch archiving off');
  assert.ok(!err({ bgArchiveDays: 0 }).bgArchiveDays);
  assert.ok(err({ lowDiskGb: -1 }).lowDiskGb);
  assert.match(machineFormError({ ...base, maxLive: -1 }), /whole number from 0/);
});

test('save body carries only the changed keys; blank means the daemon default', () => {
  const saved = machineFormFrom({ agent_background_max_live: 4 });
  assert.ok(sameMachineForm(saved, machineFormFrom({ agent_background_max_live: 4 })));
  assert.deepEqual(plain(machineBody({ ...saved, maxLive: undefined }, saved)), { agent_background_max_live: null });
  assert.deepEqual(plain(machineBody({ ...saved, buildMode: 'fixed', buildWorkers: 3, lowPriority: false }, saved)), {
    agent_background_low_priority: false,
    agent_build_parallelism: 3,
  });
  assert.deepEqual(plain(machineBody({ ...saved, bgArchiveDays: 0, lowDiskGb: 50 }, saved)), {
    session_auto_archive_background_days: 0,
    machine_low_disk_gb: 50,
  });
  // A blank and an `undefined` field are the same value: not dirty.
  assert.ok(sameMachineForm({ ...saved, lowDiskGb: undefined }, { ...saved, lowDiskGb: null }));
});

test('usage page banners: database down, tailer, journal over half full', () => {
  const now = new Date('2026-11-12T15:00:00');
  assert.deepEqual(usageHealthLines(null), []);
  assert.deepEqual(usageHealthLines({ journal_fill: 0.5 }), [], 'exactly half is not yet a warning');
  const since = new Date('2026-11-12T14:05:00').toISOString();
  const lines = usageHealthLines({ outage_since: since, journal_fill: 0.734 }, now);
  assert.deepEqual(lines.map((l) => l.key), ['outage', 'journal']);
  assert.equal(lines[0].text, `Usage has not been recorded since ${outageTime(since, now)}: the usage database is down. Otto is retrying in the background.`);
  assert.doesNotMatch(outageTime(since, now), /Nov/, 'today: the time alone');
  assert.match(outageTime(new Date('2026-11-10T09:00:00').toISOString(), now), /Nov/, 'another day: with the date');
  assert.match(lines[1].text, /journal is 73% full/);
  const tailer = usageHealthLines({ tailer: { state: 'stopped', cause: 'corrupt checkpoint' } });
  assert.equal(tailer[0].text, 'Usage isn’t being recorded: corrupt checkpoint. Restart Otto after fixing it.');
  assert.equal(outageTime('not a date'), 'not a date');
});

test('settings search finds the machine resources rows on the Daemon page', () => {
  const all = availableSections({ can: () => true, isRoot: true });
  for (const q of ['cpu', 'memory', 'load', 'queue', 'parallel', 'priority', 'disk']) {
    assert.ok(filterSections(all, q).some((s) => s.id === 'daemon'), q);
  }
  assert.equal(matchedSetting(findSection('daemon')!, 'build workers'), 'Build workers per agent session');
  // The existing archive search still names the foreground rule first.
  assert.equal(matchedSetting(findSection('daemon')!, 'archive'), 'Archive idle sessions');
});
