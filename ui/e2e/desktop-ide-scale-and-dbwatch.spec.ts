import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { createServer, type AddressInfo, type Server as NetServer, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import {
  expectContentHasHeight,
  expectFullyInViewport,
  expectNoHorizontalOverflow,
} from './helpers';
import { processResources } from './process-resources';

// Otto IDE — Live DB Watch, Auto-Save, Modal Lifecycle & 7-Project × 50-Repo Scale Gate
// Verifies against a real isolated `ottod` daemon and real installed language servers:
//   1. Opening and closing all IDE modals without console/page errors (`onclose` regression guard),
//      default IntelliJ-style Auto-Save (`autoSave: true` — debounce, tab switch, close without
//      discard prompt), Delve `__debug_bin*` noise filtering in the project tree, 20-tab LRU
//      eviction (preserving dirty + active tabs), and Live DB Watch (`detect` -> `Capture baseline`
//      -> live mutation -> `Refresh & diff` -> `Before -> After` cell diffs).
//   2. 7 Named Multi-Repo Projects (`2 Go`, `2 Java` Maven/Gradle, `1 Rust`, `1 TypeScript`,
//      `1 Python`) × 50 real `.git` repositories per project (`350` repos total) × 20 open tabs
//      per project (`140` tabs total), cycling across projects, repos, files, edits, LSP
//      (status/symbols/hover/format), Search Everywhere, Git, Run, DB Watch, and Terminal while
//      enforcing `active_server_count <= 3`, combined RSS (`ottod` + child LSPs + UI) <= 1.8 GB
//      (`1,887,436` KB), zero crashes, and zero console or `/api/v1/ide/*` 4xx/5xx errors.

/** The IDE header is one toolbar row: Settings, Share, Git, Search… live in its ⋯ menu. */
async function openIdeMenuItem(page: Page, name: string | RegExp): Promise<void> {
  await page.getByTestId('ide-more-btn').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name }).first().click();
}

/** Open a named multi-repo project from the workspace switcher (the pills row is gone). */
async function openNamedProject(page: Page, name: string): Promise<void> {
  await page.getByTestId('ide-target-switcher').click();
  await page.getByTestId('ide-named-project-option').filter({ hasText: name }).first().click();
}

test.use({ serviceWorkers: 'block' });
test.describe.configure({ mode: 'serial' });

const MAX_COMBINED_RSS_KB = 1_887_436; // 1.8 GiB in KiB (1,843.2 MiB)
const REPOS_PER_PROJECT = 50;
const TABS_PER_PROJECT = 20;

type ScaleLang = 'go' | 'java-maven' | 'java-gradle' | 'rust' | 'typescript' | 'python';

interface ScaleProjectSpec {
  key: string;
  name: string;
  lang: ScaleLang;
  lspLang: 'go' | 'java' | 'rust' | 'typescript' | 'python';
  entryRelPath: string;
}

interface ScaleProjectSeed extends ScaleProjectSpec {
  projectId: string;
  parentDir: string;
  repos: string[];
}

interface LspStatusDto {
  active_server_count: number;
  max_servers_per_owner: number;
  max_global_servers: number;
  total_rss_bytes: number;
  rss_budget_bytes: number;
  servers: Array<{
    language: string;
    root: string;
    pid?: number | null;
    rss_bytes: number;
    open_documents: number;
  }>;
  available_servers?: Array<{
    language: string;
    available: boolean;
    command: string;
  }>;
}

interface ScaleCheckpoint {
  phase: string;
  atIso: string;
  switchMs?: number;
  openTabsCount?: number;
  daemonRssKb: number;
  lspChildrenRssKb: number;
  browserRssKb: number;
  combinedIdeRssKb: number;
  activeLspCount: number;
  lspLanguages: string[];
  lspReportedRssKb: number;
}

const SCALE_PROJECTS: readonly ScaleProjectSpec[] = [
  {
    key: 'go-platform',
    name: 'Go Platform Core',
    lang: 'go',
    lspLang: 'go',
    entryRelPath: 'service_01.go',
  },
  {
    key: 'go-wallet',
    name: 'Go Wallet Engine',
    lang: 'go',
    lspLang: 'go',
    entryRelPath: 'service_01.go',
  },
  {
    key: 'java-maven',
    name: 'Java Billing Maven',
    lang: 'java-maven',
    lspLang: 'java',
    entryRelPath: 'src/main/java/com/otto/Service01.java',
  },
  {
    key: 'java-gradle',
    name: 'Java Bonus Gradle',
    lang: 'java-gradle',
    lspLang: 'java',
    entryRelPath: 'src/main/java/com/otto/Service01.java',
  },
  {
    key: 'rust-proto',
    name: 'Rust Protocol Crate',
    lang: 'rust',
    lspLang: 'rust',
    entryRelPath: 'src/lib.rs',
  },
  {
    key: 'ts-studio',
    name: 'Web Studio TypeScript',
    lang: 'typescript',
    lspLang: 'typescript',
    entryRelPath: 'src/module_01.ts',
  },
  {
    key: 'py-analytics',
    name: 'Python Analytics Lab',
    lang: 'python',
    lspLang: 'python',
    entryRelPath: 'analytics/worker_01.py',
  },
];

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

function readDaemonPid(): number {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const metaPath = join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json');
  const meta = JSON.parse(readFileSync(metaPath, 'utf8')) as { pid: number };
  if (!Number.isInteger(meta.pid) || meta.pid <= 1) {
    throw new Error(`Invalid daemon pid in ${metaPath}: ${String(meta.pid)}`);
  }
  return meta.pid;
}

function pidIsAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function respBulk(value: string): string {
  return `$${Buffer.byteLength(value, 'utf8')}\r\n${value}\r\n`;
}

function parseRespCommands(buffer: string): { commands: string[][]; rest: string } {
  const commands: string[][] = [];
  let cursor = 0;
  while (cursor < buffer.length) {
    if (buffer[cursor] !== '*') {
      const nl = buffer.indexOf('\r\n', cursor);
      if (nl < 0) break;
      const parts = buffer.slice(cursor, nl).trim().split(/\s+/).filter(Boolean);
      if (parts.length > 0) commands.push(parts);
      cursor = nl + 2;
      continue;
    }
    const headerEnd = buffer.indexOf('\r\n', cursor);
    if (headerEnd < 0) break;
    const argc = Number.parseInt(buffer.slice(cursor + 1, headerEnd), 10);
    if (!Number.isFinite(argc) || argc < 0) break;
    let pos = headerEnd + 2;
    const args: string[] = [];
    let incomplete = false;
    for (let i = 0; i < argc; i++) {
      if (pos >= buffer.length || buffer[pos] !== '$') {
        incomplete = true;
        break;
      }
      const lenEnd = buffer.indexOf('\r\n', pos);
      if (lenEnd < 0) {
        incomplete = true;
        break;
      }
      const byteLen = Number.parseInt(buffer.slice(pos + 1, lenEnd), 10);
      const dataStart = lenEnd + 2;
      // Convert UTF-8 byte length on ASCII/UTF-8 test payloads cleanly via Buffer slicing
      const tailBuf = Buffer.from(buffer.slice(dataStart), 'utf8');
      if (tailBuf.length < byteLen + 2) {
        incomplete = true;
        break;
      }
      const val = tailBuf.subarray(0, byteLen).toString('utf8');
      const remainder = tailBuf.subarray(byteLen + 2).toString('utf8');
      args.push(val);
      pos = buffer.length - remainder.length;
    }
    if (incomplete) break;
    commands.push(args);
    cursor = pos;
  }
  return { commands, rest: buffer.slice(cursor) };
}

async function startMiniRespServer(
  store: Map<string, string>,
): Promise<{ server: NetServer; sockets: Set<Socket>; port: number }> {
  const sockets = new Set<Socket>();
  const server = createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.setEncoding('utf8');
    let pending = '';
    socket.on('data', (chunk: string) => {
      pending += chunk;
      const parsed = parseRespCommands(pending);
      pending = parsed.rest;
      for (const args of parsed.commands) {
        if (args.length === 0) continue;
        const cmd = args[0].toUpperCase();
        if (cmd === 'PING') {
          socket.write('+PONG\r\n');
        } else if (cmd === 'SELECT' || cmd === 'CLIENT') {
          socket.write('+OK\r\n');
        } else if (cmd === 'AUTH' || cmd === 'HELLO') {
          socket.write('-ERR unsupported\r\n');
        } else if (cmd === 'INFO') {
          socket.write(respBulk(`# Keyspace\r\ndb0:keys=${store.size},expires=0,avg_ttl=0\r\n`));
        } else if (cmd === 'SCAN') {
          let pattern = '*';
          for (let i = 2; i + 1 < args.length; i += 2) {
            if (args[i].toUpperCase() === 'MATCH') pattern = args[i + 1];
          }
          const prefix = pattern.replace(/\*+$/, '').replace(/\\/g, '');
          const keys = [...store.keys()]
            .filter((k) => !prefix || k.startsWith(prefix))
            .sort();
          let out = `*2\r\n$1\r\n0\r\n*${keys.length}\r\n`;
          for (const k of keys) out += respBulk(k);
          socket.write(out);
        } else if (cmd === 'TYPE') {
          socket.write(store.has(args[1] ?? '') ? '+string\r\n' : '+none\r\n');
        } else if (cmd === 'TTL') {
          socket.write(':-1\r\n');
        } else if (cmd === 'OBJECT') {
          socket.write(respBulk('embstr'));
        } else if (cmd === 'STRLEN') {
          const val = store.get(args[1] ?? '');
          socket.write(`:${val ? Buffer.byteLength(val, 'utf8') : 0}\r\n`);
        } else if (cmd === 'GET' || cmd === 'GETRANGE') {
          const val = store.get(args[1] ?? '');
          socket.write(val !== undefined ? respBulk(val) : '$-1\r\n');
        } else {
          socket.write('-ERR unknown command\r\n');
        }
      }
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => resolve());
  });
  const port = (server.address() as AddressInfo).port;
  return { server, sockets, port };
}

function populateTemplateRepo(templateDir: string, lang: ScaleLang): void {
  mkdirSync(templateDir, { recursive: true });
  if (lang === 'go') {
    writeFileSync(join(templateDir, 'go.mod'), 'module example.com/ottoscale\n\ngo 1.22\n');
    for (let i = 1; i <= 22; i++) {
      const pad = String(i).padStart(2, '0');
      writeFileSync(
        join(templateDir, `service_${pad}.go`),
        `package ottoscale\n\n// ComputeBucket${pad} calculates deterministic score ${i}.\nfunc ComputeBucket${pad}(v int) int {\n\treturn v*${i} + ${i}\n}\n`,
      );
    }
  } else if (lang === 'java-maven' || lang === 'java-gradle') {
    const pkgDir = join(templateDir, 'src', 'main', 'java', 'com', 'otto');
    mkdirSync(pkgDir, { recursive: true });
    if (lang === 'java-maven') {
      writeFileSync(
        join(templateDir, 'pom.xml'),
        [
          '<project xmlns="http://maven.apache.org/POM/4.0.0">',
          '  <modelVersion>4.0.0</modelVersion>',
          '  <groupId>com.otto</groupId>',
          '  <artifactId>billing-scale</artifactId>',
          '  <version>1.0.0</version>',
          '  <properties><maven.compiler.release>21</maven.compiler.release></properties>',
          '</project>',
          '',
        ].join('\n'),
      );
    } else {
      writeFileSync(
        join(templateDir, 'build.gradle'),
        "plugins {\n  id 'java'\n}\ngroup = 'com.otto'\nversion = '1.0.0'\n",
      );
    }
    for (let i = 1; i <= 22; i++) {
      const pad = String(i).padStart(2, '0');
      writeFileSync(
        join(pkgDir, `Service${pad}.java`),
        `package com.otto;\n\npublic final class Service${pad} {\n    public int compute${pad}(int input) {\n        return input * ${i} + ${i};\n    }\n}\n`,
      );
    }
  } else if (lang === 'rust') {
    mkdirSync(join(templateDir, 'src'), { recursive: true });
    writeFileSync(
      join(templateDir, 'Cargo.toml'),
      '[package]\nname = "otto_scale_proto"\nversion = "0.1.0"\nedition = "2021"\n',
    );
    const modDecls: string[] = [];
    for (let i = 1; i <= 21; i++) {
      const pad = String(i).padStart(2, '0');
      modDecls.push(`pub mod stage_${pad};`);
      writeFileSync(
        join(templateDir, 'src', `stage_${pad}.rs`),
        `pub fn eval_stage_${pad}(x: i64) -> i64 {\n    x * ${i} + ${i}\n}\n`,
      );
    }
    writeFileSync(
      join(templateDir, 'src', 'lib.rs'),
      `${modDecls.join('\n')}\n\npub fn root_checksum(seed: i64) -> i64 {\n    stage_01::eval_stage_01(seed)\n}\n`,
    );
  } else if (lang === 'typescript') {
    mkdirSync(join(templateDir, 'src'), { recursive: true });
    writeFileSync(
      join(templateDir, 'package.json'),
      JSON.stringify({ name: '@otto/scale-studio', version: '1.0.0', type: 'module' }, null, 2) + '\n',
    );
    writeFileSync(
      join(templateDir, 'tsconfig.json'),
      JSON.stringify(
        {
          compilerOptions: {
            target: 'ES2022',
            module: 'ESNext',
            moduleResolution: 'bundler',
            strict: true,
          },
          include: ['src/**/*.ts'],
        },
        null,
        2,
      ) + '\n',
    );
    for (let i = 1; i <= 22; i++) {
      const pad = String(i).padStart(2, '0');
      writeFileSync(
        join(templateDir, 'src', `module_${pad}.ts`),
        `export interface MetricPoint${pad} {\n  readonly slot: number;\n  readonly value: number;\n}\n\nexport function computeMetric${pad}(input: number): MetricPoint${pad} {\n  return { slot: ${i}, value: input * ${i} + ${i} };\n}\n`,
      );
    }
  } else {
    mkdirSync(join(templateDir, 'analytics'), { recursive: true });
    writeFileSync(
      join(templateDir, 'pyproject.toml'),
      '[project]\nname = "otto-analytics-lab"\nversion = "0.1.0"\nrequires-python = ">=3.11"\n',
    );
    writeFileSync(join(templateDir, 'analytics', '__init__.py'), '"""Analytics package."""\n');
    for (let i = 1; i <= 22; i++) {
      const pad = String(i).padStart(2, '0');
      writeFileSync(
        join(templateDir, 'analytics', `worker_${pad}.py`),
        `def compute_window_${pad}(value: int) -> int:\n    """Return deterministic window metric ${i}."""\n    return value * ${i} + ${i}\n`,
      );
    }
  }

  const git = (...args: string[]) => execFileSync('git', ['-C', templateDir, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'scale@otto.local');
  git('config', 'user.name', 'Otto Scale');
  git('config', 'commit.gpgsign', 'false');
  git('add', '.');
  git('commit', '-q', '-m', `seed ${lang} template repo`);
}

function filePathForTab(repoDir: string, lang: ScaleLang, tabIndexOneBased: number): string {
  const pad = String(tabIndexOneBased).padStart(2, '0');
  if (lang === 'go') return join(repoDir, `service_${pad}.go`);
  if (lang === 'java-maven' || lang === 'java-gradle') {
    return join(repoDir, 'src', 'main', 'java', 'com', 'otto', `Service${pad}.java`);
  }
  if (lang === 'rust') {
    return tabIndexOneBased === 1
      ? join(repoDir, 'src', 'lib.rs')
      : join(repoDir, 'src', `stage_${String(tabIndexOneBased - 1).padStart(2, '0')}.rs`);
  }
  if (lang === 'typescript') return join(repoDir, 'src', `module_${pad}.ts`);
  return join(repoDir, 'analytics', `worker_${pad}.py`);
}

async function sampleScaleCheckpoint(
  ctx: APIRequestContext,
  base: string,
  daemonPid: number,
  phase: string,
  extra: { switchMs?: number; openTabsCount?: number } = {},
): Promise<ScaleCheckpoint> {
  const resRows = processResources(daemonPid);
  const rowMb = (name: string): number => resRows.find((r) => r.process === name)?.rss_mb ?? 0;
  const daemonRssKb = Math.round(rowMb('daemon') * 1024);
  const lspChildrenRssKb = Math.round(rowMb('agent-children') * 1024);
  const browserRssKb = Math.round(rowMb('browser') * 1024);
  const combinedIdeRssKb = daemonRssKb + lspChildrenRssKb + browserRssKb;

  const lspResp = await ctx.get(`${base}/api/v1/ide/lsp/status`);
  expect(lspResp.status(), `GET /api/v1/ide/lsp/status during ${phase}`).toBe(200);
  const lspStatus = (await lspResp.json()) as LspStatusDto;

  return {
    phase,
    atIso: new Date().toISOString(),
    switchMs: extra.switchMs,
    openTabsCount: extra.openTabsCount,
    daemonRssKb,
    lspChildrenRssKb,
    browserRssKb,
    combinedIdeRssKb,
    activeLspCount: lspStatus.active_server_count,
    lspLanguages: lspStatus.servers.map((s) => s.language),
    lspReportedRssKb: Math.round(lspStatus.total_rss_bytes / 1024),
  };
}

test.describe('Desktop IDE — Auto-Save, Modal Lifecycle, Live DB Watch & 7-Project × 50-Repo Scale Gate', () => {
  test.use({
    viewport: { width: 1440, height: 900 },
    isMobile: false,
    hasTouch: false,
    deviceScaleFactor: 2,
  });

  let rootTempDir = '';
  let dbWatchRoot = '';
  let scaleWorkspaceId = '';
  let daemonPid = 0;
  let base = '';
  let ctx: APIRequestContext;
  let respServer: NetServer | null = null;
  let respSockets: Set<Socket> | null = null;
  let redisStore = new Map<string, string>();
  let savedRedisConnId = '';
  const seededProjects: ScaleProjectSeed[] = [];

  test.beforeAll(async () => {
    test.setTimeout(180_000);
    daemonPid = readDaemonPid();
    const apiConn = await apiCtx();
    ctx = apiConn.ctx;
    base = apiConn.base;

    rootTempDir = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-scale-suite-')));

    // 1. Seed DB Watch + Auto-Save + Delve noise workspace
    dbWatchRoot = join(rootTempDir, 'dbwatch-service');
    mkdirSync(join(dbWatchRoot, 'dao'), { recursive: true });
    writeFileSync(join(dbWatchRoot, 'go.mod'), 'module example.com/dbwatchsvc\n\ngo 1.22\n');
    writeFileSync(
      join(dbWatchRoot, 'main.go'),
      [
        'package main',
        '',
        'import "fmt"',
        '',
        'func ComputeSettlement(amount int) int {',
        '\treturn amount * 2',
        '}',
        '',
        'func main() {',
        '\tfmt.Println(ComputeSettlement(21))',
        '}',
        '',
      ].join('\n'),
    );
    writeFileSync(
      join(dbWatchRoot, 'dao', 'session_store.go'),
      [
        'package dao',
        '',
        'const LookupUsersSQL = "SELECT id, email, balance FROM users WHERE active = 1"',
        '',
        'func ScanActiveSessions(rdb interface{ Scan(int, string, int) }) {',
        '\trdb.Scan(0, "session:*", 100)',
        '}',
        '',
        'func AuditCollectionName() string {',
        '\t// db.Collection("audit_events").InsertOne(ctx, payload)',
        '\treturn "audit_events"',
        '}',
        '',
      ].join('\n'),
    );
    // Delve debug artifacts that must NEVER clutter the IDE project explorer
    writeFileSync(join(dbWatchRoot, '__debug_bin'), 'delve-binary-stub');
    writeFileSync(join(dbWatchRoot, '__debug_bin391042'), 'delve-numbered-binary-stub');

    const gitInit = (dir: string) => {
      execFileSync('git', ['-C', dir, 'init', '-q'], { stdio: 'ignore' });
      execFileSync('git', ['-C', dir, 'config', 'user.email', 'ide@otto.local'], { stdio: 'ignore' });
      execFileSync('git', ['-C', dir, 'config', 'user.name', 'Otto IDE'], { stdio: 'ignore' });
      execFileSync('git', ['-C', dir, 'config', 'commit.gpgsign', 'false'], { stdio: 'ignore' });
      execFileSync('git', ['-C', dir, 'add', 'go.mod', 'main.go', 'dao/session_store.go'], {
        stdio: 'ignore',
      });
      execFileSync('git', ['-C', dir, 'commit', '-q', '-m', 'init dbwatch workspace'], {
        stdio: 'ignore',
      });
    };
    gitInit(dbWatchRoot);

    const wsResp = await ctx.post(`${base}/api/v1/workspaces`, {
      data: { name: 'IDE Scale & DB Watch WS', root_path: dbWatchRoot },
    });
    expect(wsResp.ok()).toBeTruthy();
    scaleWorkspaceId = ((await wsResp.json()) as { id: string }).id;

    // Spin up in-process loopback RESP2 server for deterministic Before -> After diffs
    redisStore = new Map<string, string>([
      ['session:1001', '{"player":"ava","balance":50}'],
      ['session:1002', '{"player":"noam","balance":10}'],
      ['session:1003', '{"player":"stale","balance":0}'],
    ]);
    const started = await startMiniRespServer(redisStore);
    respServer = started.server;
    respSockets = started.sockets;

    const connResp = await ctx.post(`${base}/api/v1/workspaces/${scaleWorkspaceId}/connections`, {
      data: {
        name: 'e2e-watch-redis',
        kind: 'redis',
        params: { host: '127.0.0.1', port: started.port, db: 0 },
        secret: null,
        environment: 'dev',
        read_only: false,
      },
    });
    expect(connResp.ok()).toBeTruthy();
    savedRedisConnId = ((await connResp.json()) as { id: string }).id;
    expect(savedRedisConnId.length).toBeGreaterThan(0);

    // 2. Build 6 language templates once, then clone into 7 projects × 50 `.git` repos (350 repos total)
    const templatesRoot = join(rootTempDir, '_templates');
    const uniqueLangs: ScaleLang[] = [
      'go',
      'java-maven',
      'java-gradle',
      'rust',
      'typescript',
      'python',
    ];
    for (const lang of uniqueLangs) {
      populateTemplateRepo(join(templatesRoot, lang), lang);
    }

    const projectsRoot = join(rootTempDir, 'projects');
    for (const spec of SCALE_PROJECTS) {
      const parentDir = join(projectsRoot, spec.key);
      mkdirSync(parentDir, { recursive: true });
      const tplDir = join(templatesRoot, spec.lang);
      const repos: string[] = [];
      for (let r = 1; r <= REPOS_PER_PROJECT; r++) {
        const repoName = `${spec.key}-repo-${String(r).padStart(2, '0')}`;
        const repoPath = join(parentDir, repoName);
        cpSync(tplDir, repoPath, { recursive: true });
        repos.push(repoPath);
      }

      const projResp = await ctx.post(`${base}/api/v1/ide/projects`, {
        data: {
          name: spec.name,
          primary_root: repos[0],
          folders: repos,
          pinned: true,
        },
      });
      expect(projResp.status(), `create named project ${spec.name}`).toBe(200);
      const created = ((await projResp.json()) as { project: { id: string; folders: string[] } })
        .project;
      expect(created.folders).toHaveLength(REPOS_PER_PROJECT);
      seededProjects.push({
        ...spec,
        projectId: created.id,
        parentDir,
        repos,
      });
    }
  });

  test.afterAll(async () => {
    if (respSockets) {
      for (const s of respSockets) s.destroy();
      respSockets.clear();
      respSockets = null;
    }
    if (respServer) {
      await new Promise<void>((resolve) => respServer!.close(() => resolve()));
      respServer = null;
    }
    await ctx?.dispose();
    if (rootTempDir && existsSync(rootTempDir)) {
      rmSync(rootTempDir, { recursive: true, force: true });
    }
  });

  let consoleAndPageErrors: string[] = [];
  let ideHttpErrors: string[] = [];

  function attachStrictMonitors(page: Page): void {
    consoleAndPageErrors = [];
    ideHttpErrors = [];
    page.on('pageerror', (err) => {
      consoleAndPageErrors.push(`pageerror: ${err.message}`);
    });
    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        const text = msg.text();
        if (/favicon\.ico/i.test(text)) return;
        consoleAndPageErrors.push(`console.error: ${text}`);
      }
    });
    page.on('response', (resp) => {
      const url = resp.url();
      if (url.includes('/api/v1/ide/') && resp.status() >= 400) {
        ideHttpErrors.push(`${resp.request().method()} ${url} -> ${resp.status()}`);
      }
    });
  }

  test('1. Modal onclose regression guard, default Auto-Save (debounce/tab-switch/close), Delve __debug_bin filtering, 20-tab LRU cap & Live DB Watch Before->After diffs', async ({
    page,
  }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'Runs on desktop-browser');
    test.setTimeout(150_000);
    attachStrictMonitors(page);

    await page.addInitScript((wsId: string) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
    }, scaleWorkspaceId);

    await page.goto(`/#/ide?root=${encodeURIComponent(dbWatchRoot)}&file=${encodeURIComponent(join(dbWatchRoot, 'main.go'))}`);
    await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText(
      'ComputeSettlement',
      { timeout: 20_000 },
    );
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 420);

    // 1a) Delve `__debug_bin*` artifacts must be hidden from the project tree
    const tree = page.getByTestId('ide-project-tree');
    await expect(tree).toContainText('main.go');
    await expect(tree).toContainText('dao');
    await expect(tree).not.toContainText('__debug_bin');

    // 1b) Open & close all IDE LazyMount modals via UI & Escape with ZERO `b.onclose` errors
    // Modal 1: IdeSettingsModal (`⌘,` button)
    await openIdeMenuItem(page, 'Settings & Project Structure…');
    const dialog = page.locator('[role="dialog"]').first();
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // Modal 2: IdeShareModal (`Share` button)
    await openIdeMenuItem(page, 'Share live session…');
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // Modal 3: IdeBulkRepoPickerModal (`Workspace Switcher` -> `Bulk attach repos…`)
    await page.getByTestId('ide-target-switcher').click();
    const wsPop = page.getByTestId('ide-workspace-popover');
    await expectFullyInViewport(page, wsPop, 'IDE workspace switcher popover');
    await wsPop.getByTestId('ide-switcher-bulk-attach').click();
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // Modal 4: FolderPicker (`Add folder to project` button in tree toolbar)
    await page.getByTestId('ide-add-folder-btn').click();
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // Modal 5: IdeEnvironmentsModal (`More actions` -> `Run configurations…`)
    await page.getByTestId('ide-more-btn').click();
    await page.getByRole('menuitem', { name: /Run configurations/i }).click();
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // Modal 6: IdeComparisonModal (`Show project local history` button)
    await page.getByTestId('ide-tree-local-history-btn').click();
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // Modal 7: IdeSearchEverywhere (⋯ → `Search everywhere…`)
    await openIdeMenuItem(page, 'Search everywhere…');
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden({ timeout: 10_000 });

    // 1c) Default Auto-Save (`autoSave: true` out of the box):
    //     - Saves automatically after idle debounce
    //     - Saves immediately on tab switch
    //     - Closes dirty tab with ZERO discard confirmation dialog and persists edit
    const mainGoPath = join(dbWatchRoot, 'main.go');
    const daoGoPath = join(dbWatchRoot, 'dao', 'session_store.go');

    const cmContent = page.locator('.ide-editor-area .cm-content').first();
    await cmContent.click();
    await page.keyboard.press('Meta+ArrowDown');
    await page.keyboard.type('\n// autosave-idle-verified\n');
    await expect
      .poll(() => readFileSync(mainGoPath, 'utf8'), { timeout: 6_000 })
      .toContain('// autosave-idle-verified');

    // Open dao/session_store.go via tree expand + click
    const daoDirBtn = tree
      .locator('button.tree-item[title="dao"], button.tree-item[title$="/dao"]')
      .first();
    await expect(daoDirBtn).toBeVisible({ timeout: 10_000 });
    if ((await daoDirBtn.getAttribute('aria-expanded')) !== 'true') {
      await daoDirBtn.click();
    }
    const daoFileBtn = tree
      .locator(
        'button.tree-item[title="dao/session_store.go"], button.tree-item[title$="/dao/session_store.go"]',
      )
      .first();
    await expect(daoFileBtn).toBeVisible({ timeout: 10_000 });
    await daoFileBtn.dblclick();
    await expect(cmContent).toContainText('ScanActiveSessions', { timeout: 10_000 });

    // Edit session_store.go and immediately switch back to main.go tab -> flushes on tab switch!
    await cmContent.click();
    await page.keyboard.press('Meta+ArrowDown');
    await page.keyboard.type('\n// autosave-tab-switch-verified\n');
    await page.getByTestId('ide-tab-item').filter({ hasText: 'main.go' }).first().locator('.ide-tab-btn').click();
    await expect
      .poll(() => readFileSync(daoGoPath, 'utf8'), { timeout: 5_000 })
      .toContain('// autosave-tab-switch-verified');

    // Edit main.go and immediately close tab -> NO confirmation dialog & edit flushed to disk!
    await cmContent.click();
    await page.keyboard.press('Meta+ArrowDown');
    await page.keyboard.type('// autosave-close-tab-verified\n');
    await page.getByRole('button', { name: 'Close main.go' }).click();
    await expect(page.locator('[role="dialog"]')).toHaveCount(0);
    await expect
      .poll(() => readFileSync(mainGoPath, 'utf8'), { timeout: 5_000 })
      .toContain('// autosave-close-tab-verified');

    // 1d) Live DB Watch: code-level detection + Capture baseline + live mutation + Before -> After diff
    await expect(cmContent).toContainText('ScanActiveSessions');
    await page.getByTestId('ide-more-btn').click();
    await page.getByRole('menuitem', { name: 'Live DB Watch' }).click();

    const dbPanel = page.getByTestId('ide-dbwatch-panel');
    await expect(dbPanel).toBeVisible({ timeout: 10_000 });

    // Verify code scanner detected Redis `session:*`, SQL `users`, and Mongo `audit_events`
    const detectedBar = page.getByTestId('ide-dbw-detected-bar');
    await expect(detectedBar).toBeVisible({ timeout: 10_000 });
    await expect(detectedBar).toContainText('session:*');
    await expect(detectedBar).toContainText('users');
    await expect(detectedBar).toContainText('audit_events');

    // Select the saved Redis target and click the detected `session:*` chip
    const targetSelect = page.getByTestId('ide-dbw-target-select');
    await targetSelect.selectOption(`saved:${savedRedisConnId}`);
    await detectedBar.locator('.dbw-chip-select').filter({ hasText: 'session:*' }).first().click();
    await expect(page.getByTestId('ide-dbw-object-input')).toHaveValue('session:*');

    // Capture baseline (3 Redis keys: session:1001, session:1002, session:1003)
    await page.getByTestId('ide-dbw-capture-baseline').click();
    await expect(dbPanel).toContainText('Baseline active (3 rows)', { timeout: 10_000 });

    // Mutate live Redis store: modify session:1001, add session:1004, delete session:1003
    redisStore.set('session:1001', '{"player":"ava","balance":175}');
    redisStore.set('session:1004', '{"player":"liam","balance":90}');
    redisStore.delete('session:1003');

    await page.getByTestId('ide-dbw-refresh-diff').click();
    const diffSummary = page.getByTestId('ide-dbw-diff-summary');
    await expect(diffSummary).toBeVisible({ timeout: 10_000 });
    await expect(diffSummary).toContainText('+1 added');
    await expect(diffSummary).toContainText('~1 modified');
    await expect(diffSummary).toContainText('-1 removed');
    await expect(diffSummary).toContainText('1 unchanged');

    const diffTable = page.getByTestId('ide-dbw-diff-table');
    await expect(diffTable).toBeVisible();
    await expect(diffTable).toContainText('{"player":"ava","balance":50}');
    await expect(diffTable).toContainText('{"player":"ava","balance":175}');
    await expect(diffTable).toContainText('session:1004');
    await expect(diffTable).toContainText('session:1003');

    expect(consoleAndPageErrors, 'Zero console or page errors allowed').toEqual([]);
    expect(ideHttpErrors, 'Zero /api/v1/ide/* 4xx/5xx responses allowed').toEqual([]);
  });

  test('2. 7 Projects (2 Go, 2 Java, 1 Rust, 1 TS, 1 Python) × 50 Git Repos (350 total) × 20 Tabs (140 total) Endurance & <= 1.8 GB RSS Gate', async ({
    page,
  }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'Runs on desktop-browser');
    test.setTimeout(420_000);
    attachStrictMonitors(page);

    await page.addInitScript((wsId: string) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
    }, scaleWorkspaceId);

    const checkpoints: ScaleCheckpoint[] = [];
    const baseline = await sampleScaleCheckpoint(ctx, base, daemonPid, '00-pre-scale-baseline');
    checkpoints.push(baseline);

    await page.goto('/#/ide');
    await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });

    // Verify all 7 pinned multi-repo projects are listed in the workspace switcher with `50` repo badges
    await page.getByTestId('ide-target-switcher').click();
    const options = page.getByTestId('ide-named-project-option');
    await expect(options).toHaveCount(SCALE_PROJECTS.length, { timeout: 15_000 });
    for (const proj of seededProjects) {
      await expect(options.filter({ hasText: proj.name })).toContainText(String(REPOS_PER_PROJECT));
    }
    await page.keyboard.press('Escape');

    // Pass 1: Visit all 7 projects, open 20 tabs across 20 distinct repos per project (140 tabs total),
    // verify 21st-tab LRU cap, run LSP operations + edits/auto-save + project tree & dock interactions.
    for (let pIdx = 0; pIdx < seededProjects.length; pIdx++) {
      const proj = seededProjects[pIdx];
      const swStart = performance.now();
      await openNamedProject(page, proj.name);
      await expect(page.getByTestId('ide-target-switcher')).toContainText(
        `${REPOS_PER_PROJECT} repos`,
        { timeout: 20_000 },
      );
      await expect(page.getByTestId('ide-tree-root-node')).toBeVisible({ timeout: 15_000 });
      const switchMs = Math.round(performance.now() - swStart);

      // Open 20 files across repos 01..20 using `#/ide?project=...&file=...` navigation
      // (mounts real CodeMirror + WebSocket `/ws/lsp` + symbols/gutter per active tab).
      for (let t = 1; t <= TABS_PER_PROJECT; t++) {
        const repoDir = proj.repos[t - 1];
        const filePath = filePathForTab(repoDir, proj.lang, t);
        await page.evaluate(
          ({ projId, fp }) => {
            window.location.hash = `#/ide?project=${encodeURIComponent(projId)}&file=${encodeURIComponent(fp)}`;
          },
          { projId: proj.projectId, fp: filePath },
        );
        const baseName = filePath.slice(filePath.lastIndexOf('/') + 1);
        await expect(
          page.getByTestId('ide-tab-item').filter({ hasText: baseName }).first(),
        ).toBeVisible({ timeout: 15_000 });
      }

      const openCountAt20 = await page.getByTestId('ide-tab-item').count();
      expect(openCountAt20, `${proj.name} should hold ${TABS_PER_PROJECT} open tabs`).toBe(
        TABS_PER_PROJECT,
      );

      // Open a 21st file from repo 21 and verify the 20-tab LRU eviction cap keeps `tabs.length <= 20`!
      const twentyFirstFile = filePathForTab(proj.repos[20], proj.lang, 21);
      await page.evaluate(
        ({ projId, fp }) => {
          window.location.hash = `#/ide?project=${encodeURIComponent(projId)}&file=${encodeURIComponent(fp)}`;
        },
        { projId: proj.projectId, fp: twentyFirstFile },
      );
      const twentyFirstBase = twentyFirstFile.slice(twentyFirstFile.lastIndexOf('/') + 1);
      await expect(
        page.getByTestId('ide-tab-item').filter({ hasText: twentyFirstBase }).first(),
      ).toBeVisible({ timeout: 15_000 });
      const cappedCount = await page.getByTestId('ide-tab-item').count();
      expect(cappedCount, `${proj.name} LRU tab cap must bound open tabs at <= 20`).toBeLessThanOrEqual(
        TABS_PER_PROJECT,
      );

      // Exercise real LSP intelligence endpoints (symbols, hover, format) on the active project's primary repo
      const primaryEntry = join(proj.repos[0], proj.entryRelPath);
      const entryText = readFileSync(primaryEntry, 'utf8');
      const targetSpec = {
        kind: 'local',
        workspace_id: scaleWorkspaceId,
        root: proj.repos[0],
        additional_roots: proj.repos.slice(1),
        project_folders: proj.repos,
      };

      const symResp = await ctx.post(`${base}/api/v1/ide/lsp/symbols`, {
        data: { target: targetSpec, path: primaryEntry, content: entryText },
      });
      expect(symResp.status(), `LSP symbols for ${proj.name}`).toBe(200);

      const hoverResp = await ctx.post(`${base}/api/v1/ide/lsp/hover`, {
        data: {
          target: targetSpec,
          path: primaryEntry,
          line: 3,
          column: 6,
          content: entryText,
        },
      });
      expect(hoverResp.status(), `LSP hover for ${proj.name} must never return 422`).toBe(200);

      const fmtResp = await ctx.post(`${base}/api/v1/ide/lsp/format`, {
        data: {
          target: targetSpec,
          path: primaryEntry,
          language: proj.lspLang,
          content: entryText,
        },
      });
      expect(fmtResp.status(), `LSP format for ${proj.name}`).toBe(200);

      // Perform a live CodeMirror edit + verify Auto-Save persists to disk
      const editor = page.locator('.ide-editor-area .cm-content').first();
      await expect(editor).toBeVisible({ timeout: 15_000 });
      await editor.click();
      await page.keyboard.press('Meta+ArrowDown');
      const editStamp = proj.lang === 'python' ? `\n# scale-edit-${proj.key}\n` : `\n// scale-edit-${proj.key}\n`;
      await page.keyboard.type(editStamp);
      await expect
        .poll(() => readFileSync(twentyFirstFile, 'utf8'), { timeout: 6_000 })
        .toContain(`scale-edit-${proj.key}`);

      const cp = await sampleScaleCheckpoint(ctx, base, daemonPid, `01-project-${pIdx + 1}-${proj.key}`, {
        switchMs,
        openTabsCount: cappedCount,
      });
      checkpoints.push(cp);

      expect(
        cp.activeLspCount,
        `Warm LSP pool must cap active servers per owner at <= 3 (at ${proj.name})`,
      ).toBeLessThanOrEqual(3);
      expect(
        cp.combinedIdeRssKb,
        `Combined RSS (ottod + child LSPs + browser) at ${proj.name} must stay <= 1.8 GB (${MAX_COMBINED_RSS_KB} KB)`,
      ).toBeLessThanOrEqual(MAX_COMBINED_RSS_KB);
    }

    // Pass 2: Rapid multi-round cross-language endurance switching across all 7 projects,
    // exercising Search Everywhere, Git, Run, DB Watch, and Terminal docks!
    const visitOrder = [0, 2, 4, 5, 1, 3, 6, 0, 4, 6, 2, 5];
    for (let step = 0; step < visitOrder.length; step++) {
      const proj = seededProjects[visitOrder[step]];
      const swStart = performance.now();
      await openNamedProject(page, proj.name);
      await expect(page.getByTestId('ide-target-switcher')).toContainText(
        `${REPOS_PER_PROJECT} repos`,
        { timeout: 15_000 },
      );
      // Persisted open tabs for this project restore automatically (`<= 20`)
      await expect(page.getByTestId('ide-tab-item').first()).toBeVisible({ timeout: 15_000 });
      const switchMs = Math.round(performance.now() - swStart);

      if (step === 0) {
        // Visit Git dock across 50-repo project
        await openIdeMenuItem(page, 'Git Changes');
        await expect(page.getByTestId('ide-dock-tab-git')).toBeVisible({ timeout: 10_000 });
      } else if (step === 2) {
        // Visit DB Watch dock
        await page.getByTestId('ide-tab-dbwatch').click();
        await expect(page.getByTestId('ide-dbwatch-panel')).toBeVisible({ timeout: 10_000 });
      } else if (step === 4) {
        // Visit Problems & Run tabs
        await page.getByTestId('ide-tab-problems').click();
        await page.getByTestId('ide-tab-run').click();
      } else if (step === 6) {
        // Visit Terminal dock (spawns real PTY shell in active project root)
        await page.getByTestId('ide-tab-terminal').click();
        await sleep(500);
      } else if (step === 8) {
        // Search Everywhere across 50 repos
        await openIdeMenuItem(page, 'Search everywhere…');
        const searchDlg = page.locator('[role="dialog"]').first();
        await expect(searchDlg).toBeVisible({ timeout: 10_000 });
        await page.keyboard.type('stage_05');
        await sleep(250);
        await page.keyboard.press('Escape');
        await expect(searchDlg).toBeHidden({ timeout: 10_000 });
      }

      if (step % 3 === 2 || step === visitOrder.length - 1) {
        const tabsNow = await page.getByTestId('ide-tab-item').count();
        const cp = await sampleScaleCheckpoint(
          ctx,
          base,
          daemonPid,
          `02-endurance-step-${step + 1}-${proj.key}`,
          { switchMs, openTabsCount: tabsNow },
        );
        checkpoints.push(cp);
        expect(cp.activeLspCount).toBeLessThanOrEqual(3);
        expect(cp.combinedIdeRssKb).toBeLessThanOrEqual(MAX_COMBINED_RSS_KB);
      }
    }

    // Final settle check: daemon PID unchanged & alive, RSS within 1.8 GB, 0 errors
    expect(pidIsAlive(daemonPid), 'ottod daemon must remain alive with zero crashes').toBe(true);
    expect(readDaemonPid(), 'ottod PID must not have restarted').toBe(daemonPid);

    const finalCp = await sampleScaleCheckpoint(ctx, base, daemonPid, '03-final-settled');
    checkpoints.push(finalCp);
    expect(finalCp.activeLspCount).toBeLessThanOrEqual(3);
    expect(finalCp.combinedIdeRssKb).toBeLessThanOrEqual(MAX_COMBINED_RSS_KB);

    const peakCombinedRssKb = Math.max(...checkpoints.map((c) => c.combinedIdeRssKb));
    const peakDaemonRssKb = Math.max(...checkpoints.map((c) => c.daemonRssKb));
    const peakLspChildrenRssKb = Math.max(...checkpoints.map((c) => c.lspChildrenRssKb));
    const peakBrowserRssKb = Math.max(...checkpoints.map((c) => c.browserRssKb));
    const maxActiveLsps = Math.max(...checkpoints.map((c) => c.activeLspCount));
    const switchSamples = checkpoints
      .map((c) => c.switchMs)
      .filter((v): v is number => typeof v === 'number');
    const p50SwitchMs =
      switchSamples.length > 0
        ? [...switchSamples].sort((a, b) => a - b)[Math.floor(switchSamples.length * 0.5)]
        : 0;

    const report = {
      generatedAt: new Date().toISOString(),
      projectsCount: seededProjects.length,
      reposPerProject: REPOS_PER_PROJECT,
      totalGitRepos: seededProjects.length * REPOS_PER_PROJECT,
      tabsOpenedPerProject: TABS_PER_PROJECT,
      totalTabsOpenedAcrossProjects: seededProjects.length * TABS_PER_PROJECT,
      budgetCombinedRssKb: MAX_COMBINED_RSS_KB,
      peakCombinedRssKb,
      peakCombinedRssMb: Math.round((peakCombinedRssKb / 1024) * 100) / 100,
      peakDaemonRssKb,
      peakLspChildrenRssKb,
      peakBrowserRssKb,
      maxActiveLspServers: maxActiveLsps,
      medianProjectSwitchMs: p50SwitchMs,
      consoleErrorCount: consoleAndPageErrors.length,
      ideHttpErrorCount: ideHttpErrors.length,
      checkpoints,
    };

    const reportPath =
      process.env.OTTO_IDE_SCALE_REPORT || join(tmpdir(), 'otto-ide-scale-7proj-350repos.json');
    writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
    console.log(
      `[ide-scale-report] projects=7 repos=350 tabs=140 peakCombinedRssMb=${report.peakCombinedRssMb} maxActiveLsp=${maxActiveLsps} p50SwitchMs=${p50SwitchMs}`,
    );

    expect(consoleAndPageErrors, 'Zero console or page errors allowed across 7-project scale run').toEqual([]);
    expect(ideHttpErrors, 'Zero /api/v1/ide/* 4xx/5xx responses allowed across 7-project scale run').toEqual([]);
  });
});
