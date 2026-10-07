import { execFileSync } from 'node:child_process';

type ProcessRow = { pid: number; parent: number; start: string; cpu: number; rss: number; name: string };
type ResourceRow = { process: string; cpu_percent: number; rss_mb: number; at: number; pids: number[] };
const coalitionCache = new Map<string, { coalition: number; at: number }>();
const COALITION_TTL_MS = 30_000;
const MAX_COALITION_CANDIDATES = 64;

// macOS launches Playwright WebKit XPC helpers under launchd (PPID 1). Match
// both the owned engine distribution and its resource coalition, never just a
// WebKit executable name: other tests and the installed app may run alongside.
function webkitDistribution(name: string): string | null {
  return /^(.*\/ms-playwright\/webkit-[^/]+\/)(?:Playwright\.app\/Contents\/MacOS\/Playwright|com\.apple\.WebKit\.[^/]+\.xpc\/Contents\/MacOS\/com\.apple\.WebKit\.[^/]+)$/.exec(name)?.[1] ?? null;
}
function identity(row: ProcessRow): string { return `${row.pid}\0${row.start}\0${row.name}`; }

const READ_COALITIONS = `
import ctypes, json, os, subprocess, sys
candidates = json.loads(sys.argv[1])
lib = ctypes.CDLL('/usr/lib/libproc.dylib')
lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
lib.proc_pidinfo.restype = ctypes.c_int
# Revalidate the process incarnation immediately before reading its coalition.
result = subprocess.run(['ps', '-p', ','.join(str(p['pid']) for p in candidates), '-o', 'pid=,lstart='], capture_output=True, text=True, env={**os.environ, 'LC_ALL': 'C'}, timeout=2)
starts = {}
for line in result.stdout.splitlines():
    fields = line.split()
    if len(fields) == 6:
        starts[int(fields[0])] = ' '.join(fields[1:])
rows = []
for p in candidates:
    coalition = None
    if starts.get(p['pid']) == p['start']:
        info = (ctypes.c_uint64 * 5)()
        if lib.proc_pidinfo(p['pid'], 20, 0, info, ctypes.sizeof(info)) == ctypes.sizeof(info) and info[0]:
            coalition = int(info[0])
    rows.append({**p, 'coalition': coalition})
print(json.dumps(rows))
`;

function resourceCoalitions(candidates: ProcessRow[], at: number): Map<number, number> {
  if (candidates.length > MAX_COALITION_CANDIDATES) throw new Error('Too many WebKit coalition candidates; refusing partial browser totals');
  const current = new Set(candidates.map(identity));
  for (const key of coalitionCache.keys()) if (!current.has(key)) coalitionCache.delete(key);
  const stale = candidates.filter((row) => {
    const cached = coalitionCache.get(identity(row));
    return !cached || at - cached.at >= COALITION_TTL_MS;
  });
  if (stale.length) {
    const output = execFileSync('python3', ['-c', READ_COALITIONS, JSON.stringify(stale.map(({ pid, start }) => ({ pid, start })))], {
      encoding: 'utf8', timeout: 3_000, maxBuffer: 64 * 1024,
    });
    const results = JSON.parse(output) as { pid: number; start: string; coalition: number | null }[];
    for (const row of stale) {
      const result = results.find((value) => value.pid === row.pid && value.start === row.start);
      if (result?.coalition && Number.isSafeInteger(result.coalition)) coalitionCache.set(identity(row), { coalition: result.coalition, at });
      else coalitionCache.delete(identity(row));
    }
  }
  return new Map(candidates.flatMap((row) => {
    const value = coalitionCache.get(identity(row));
    return value ? [[row.pid, value.coalition] as const] : [];
  }));
}

/** Read-only process totals for one isolated daemon and the test-owned browser.
 * The driver is reported separately and excluded from total. CPU is ps's
 * platform estimate; 100% is one core, not exclusive operation CPU. */
export function processResources(daemonPid: number): ResourceRow[] {
  const at = Date.now();
  // Read process IDs, parents, start identities and totals, never arguments.
  const output = execFileSync('ps', ['-axo', 'pid=,ppid=,lstart=,%cpu=,rss=,comm='], {
    encoding: 'utf8', env: { ...process.env, LC_ALL: 'C' }, timeout: 3_000,
  });
  const processes = output.trim().split('\n').map((line) => {
    const match = /^\s*(\d+)\s+(\d+)\s+(\w+\s+\w+\s+\d+\s+\d+:\d+:\d+\s+\d+)\s+([\d.]+)\s+(\d+)\s+(.+)$/.exec(line);
    return match ? { pid: Number(match[1]), parent: Number(match[2]), start: match[3].replace(/\s+/g, ' '), cpu: Number(match[4]), rss: Number(match[5]) / 1024, name: match[6] } : null;
  }).filter((row): row is ProcessRow => row !== null);
  const parents = new Map(processes.map((row) => [row.pid, row.parent]));
  const descendant = (pid: number, root: number): boolean => {
    for (let depth = 0; depth < 64 && pid > 1; depth++) { if (pid === root) return true; pid = parents.get(pid) ?? 0; }
    return false;
  };
  const webkitRoots = process.platform === 'darwin' ? processes.filter((row) =>
    row.name.endsWith('/Playwright.app/Contents/MacOS/Playwright') && webkitDistribution(row.name) && descendant(row.pid, process.pid)) : [];
  const distributions = new Set(webkitRoots.map((row) => webkitDistribution(row.name)));
  const coalitions = resourceCoalitions(processes.filter((row) => distributions.has(webkitDistribution(row.name))), at);
  const ownedCoalitions = new Set(webkitRoots.map((row) => {
    const coalition = coalitions.get(row.pid);
    if (!coalition) throw new Error('Cannot establish owned WebKit resource coalition; refusing partial browser totals');
    return coalition;
  }));
  const groups = new Map<string, { cpu: number; rss: number; pids: number[] }>();
  for (const row of processes) {
    let label: string;
    if (row.pid === process.pid) label = 'load-driver';
    else if (row.pid === daemonPid) label = 'daemon';
    else if (descendant(row.pid, daemonPid)) label = row.name.includes('otelcol') ? 'collector' : row.name.includes('clickhouse') ? 'clickhouse' : 'agent-children';
    else if ((descendant(row.pid, process.pid) && /chromium|chrome|webkit|MiniBrowser/i.test(row.name))
      || (distributions.has(webkitDistribution(row.name)) && ownedCoalitions.has(coalitions.get(row.pid) ?? 0))) label = 'browser';
    else continue;
    const group = groups.get(label) ?? { cpu: 0, rss: 0, pids: [] };
    group.cpu += row.cpu; group.rss += row.rss; group.pids.push(row.pid); groups.set(label, group);
  }
  const rows: ResourceRow[] = [];
  const total = { cpu: 0, rss: 0, pids: [] as number[] };
  for (const [label, group] of groups) {
    rows.push({ process: label, cpu_percent: group.cpu, rss_mb: group.rss, at, pids: group.pids });
    if (label !== 'load-driver') { total.cpu += group.cpu; total.rss += group.rss; total.pids.push(...group.pids); }
  }
  rows.push({ process: 'total', cpu_percent: total.cpu, rss_mb: total.rss, at, pids: total.pids });
  return rows;
}
