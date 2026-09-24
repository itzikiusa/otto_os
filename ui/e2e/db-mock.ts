import type { APIRequestContext, Page, Route } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// A Docker-free DB Explorer: a real connection PROFILE on the isolated test
// daemon (so the sidebar, tabs, access checks and persistence are all real),
// with every engine call under `/connections/{id}/db/*` answered by
// `page.route` from a small in-memory "shop" schema. Lets layout / builder /
// grid specs run on any machine — the engine drivers are covered by the
// `db-sweep-*` specs against the live stack.
//
// The profile points at a closed loopback port, so nothing ever reaches a real
// database even if a route slips past the mock.
// ─────────────────────────────────────────────────────────────────────────────

export type MockEngine = 'mysql' | 'postgres' | 'clickhouse';

interface MockCol {
  name: string;
  type: string;
  nullable?: boolean;
  key?: string;
}
interface MockTable {
  name: string;
  kind: 'table' | 'view';
  pk: string[];
  cols: MockCol[];
  fks?: { name: string; columns: string[]; ref_table: string; ref_columns: string[] }[];
  rows: number;
}

export const MOCK_TABLES: MockTable[] = [
  {
    name: 'orders',
    kind: 'table',
    pk: ['id'],
    rows: 240,
    cols: [
      { name: 'id', type: 'bigint', key: 'PRI' },
      { name: 'customer_id', type: 'int', key: 'MUL' },
      { name: 'status', type: 'varchar(20)' },
      { name: 'total', type: 'decimal(10,2)' },
      { name: 'currency', type: 'char(3)' },
      { name: 'created_at', type: 'datetime' },
      { name: 'shipped_at', type: 'datetime', nullable: true },
      { name: 'notes', type: 'text', nullable: true },
      { name: 'meta', type: 'json', nullable: true },
      { name: 'is_gift', type: 'tinyint(1)' },
      { name: 'items', type: 'int' },
      { name: 'region', type: 'varchar(32)' },
    ],
    fks: [{ name: 'fk_orders_customer', columns: ['customer_id'], ref_table: 'customers', ref_columns: ['id'] }],
  },
  {
    name: 'customers',
    kind: 'table',
    pk: ['id'],
    rows: 120,
    cols: [
      { name: 'id', type: 'int', key: 'PRI' },
      { name: 'email', type: 'varchar(255)', key: 'UNI' },
      { name: 'name', type: 'varchar(120)' },
      { name: 'country', type: 'char(2)' },
      { name: 'created_at', type: 'datetime' },
      { name: 'lifetime_value', type: 'decimal(12,2)', nullable: true },
    ],
  },
  {
    name: 'order_items',
    kind: 'table',
    pk: ['id'],
    rows: 400,
    cols: [
      { name: 'id', type: 'bigint', key: 'PRI' },
      { name: 'order_id', type: 'bigint', key: 'MUL' },
      { name: 'sku', type: 'varchar(40)', key: 'MUL' },
      { name: 'qty', type: 'int' },
      { name: 'price', type: 'decimal(10,2)' },
    ],
    fks: [
      { name: 'fk_items_order', columns: ['order_id'], ref_table: 'orders', ref_columns: ['id'] },
      { name: 'fk_items_product', columns: ['sku'], ref_table: 'products', ref_columns: ['sku'] },
    ],
  },
  {
    name: 'products',
    kind: 'table',
    pk: ['sku'],
    rows: 60,
    cols: [
      { name: 'sku', type: 'varchar(40)', key: 'PRI' },
      { name: 'title', type: 'varchar(200)' },
      { name: 'price', type: 'decimal(10,2)' },
      { name: 'stock', type: 'int' },
    ],
  },
  {
    name: 'v_daily_sales',
    kind: 'view',
    pk: [],
    rows: 30,
    cols: [
      { name: 'day', type: 'date' },
      { name: 'orders', type: 'bigint' },
      { name: 'revenue', type: 'decimal(14,2)' },
    ],
  },
];

const STATUSES = ['paid', 'shipped', 'pending', 'refunded', 'cancelled'];
const REGIONS = ['eu-west', 'us-east', 'ap-south', 'us-west'];
const NAMES = ['Ada Lovelace', 'Grace Hopper', 'Linus Torvalds', 'Margaret Hamilton', 'Ken Thompson', 'Barbara Liskov'];

/** A deterministic value for (table, column, row). */
function cell(t: MockTable, c: MockCol, r: number): unknown {
  const n = r + 1;
  if (c.nullable && (n * 7 + c.name.length) % 5 === 0) return null;
  switch (`${t.name}.${c.name}`) {
    case 'orders.status':
      return STATUSES[n % STATUSES.length];
    case 'orders.currency':
      return n % 3 === 0 ? 'EUR' : 'USD';
    case 'orders.region':
      return REGIONS[n % REGIONS.length];
    case 'orders.notes':
      return n % 2 ? 'Leave at the front desk' : 'Gift wrap, no invoice in the box please';
    case 'orders.meta':
      return { channel: n % 2 ? 'web' : 'ios', coupon: n % 4 ? null : 'SPRING10' };
    case 'customers.email':
      return `user${n}@example.com`;
    case 'customers.name':
      return NAMES[n % NAMES.length];
    case 'customers.country':
      return ['US', 'DE', 'IL', 'GB', 'FR'][n % 5];
    case 'products.title':
      return `Product ${n}`;
  }
  if (c.name === 'sku') return `SKU-${String(1000 + (n % 60)).padStart(5, '0')}`;
  if (c.type.startsWith('decimal')) return ((n * 37.13) % 900 + 10).toFixed(2);
  if (c.type.startsWith('tinyint')) return n % 4 === 0 ? 1 : 0;
  if (c.type.includes('int')) return c.name === 'id' ? n : ((n * 13) % 97) + 1;
  if (c.type === 'datetime') return `2026-0${1 + (n % 9)}-${String(1 + (n % 28)).padStart(2, '0')} 1${n % 10}:2${n % 10}:00`;
  if (c.type === 'date') return `2026-09-${String(1 + (n % 28)).padStart(2, '0')}`;
  return `${c.name}-${n}`;
}

export interface MockOptions {
  engine?: MockEngine;
  /** Artificial latency for `query` (ms) — makes loading states observable. */
  queryDelay?: number;
  /** Artificial latency for `object` (the editability probe + Structure). */
  objectDelay?: number;
  /** Statements seen by the mocked `query` route, in order. */
  seen?: string[];
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/** Seed the mocked connection profile and return its id. */
export async function seedMockDbConnection(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
  name = 'mock-shop',
  engine: MockEngine = 'mysql',
): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, {
    data: {
      name,
      kind: engine,
      // Port 9 (discard) on loopback: closed on macOS — a request that escapes
      // the mock fails fast instead of reaching any real database.
      params: { host: '127.0.0.1', port: 9, user: 'mock', db: 'shop' },
      secret: 'mock',
      environment: 'dev',
      read_only: false,
    },
  });
  if (!r.ok()) throw new Error(`seed mock connection → ${r.status()} ${await r.text()}`);
  return ((await r.json()) as { id: string }).id;
}

function tablePath(t: MockTable): string {
  return `db:shop/${t.kind}:${t.name}`;
}

function detailOf(t: MockTable) {
  return {
    name: t.name,
    kind: t.kind,
    columns: t.cols.map((c) => ({
      name: c.name,
      data_type: c.type,
      nullable: !!c.nullable,
      key: c.key ?? null,
      default: null,
      extra: c.name === 'id' && t.kind === 'table' ? 'auto_increment' : null,
    })),
    primary_key: t.pk,
    indexes: t.pk.length
      ? [{ name: 'PRIMARY', columns: t.pk, unique: true, method: 'BTREE' }]
      : [],
    foreign_keys: (t.fks ?? []).map((f) => ({ ...f, ref_schema: 'shop' })),
    ddl: `CREATE TABLE \`${t.name}\` (…)`,
    row_count: t.rows,
  };
}

/** Rows for a statement: the FROM table's columns (or the listed columns). */
function resultFor(statement: string): Record<string, unknown> {
  const m = /\bfrom\s+[`"]?(\w+)[`"]?(?:\s*\.\s*[`"]?(\w+)[`"]?)?/i.exec(statement);
  const name = m ? (m[2] ?? m[1]) : null;
  const t = MOCK_TABLES.find((x) => x.name === name);
  if (!t) {
    return {
      columns: [{ name: 'result', type_hint: 'int' }],
      rows: [[1]],
      stats: { duration_ms: 2, row_count: 1 },
      truncated: false,
    };
  }
  const limit = /\blimit\s+(\d+)/i.exec(statement);
  const n = Math.min(t.rows, limit ? Number(limit[1]) : t.rows);
  return {
    columns: t.cols.map((c) => ({ name: c.name, type_hint: c.type.toUpperCase() })),
    rows: Array.from({ length: n }, (_, r) => t.cols.map((c) => cell(t, c, r))),
    stats: { duration_ms: 14, row_count: n },
    truncated: false,
    auto_limited: limit ? null : 1000,
  };
}

function json(route: Route, body: unknown): Promise<void> {
  return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
}

/** Answer every `/connections/{connId}/db/*` call from the mock schema. */
export async function mockDbRoutes(page: Page, connId: string, opts: MockOptions = {}): Promise<void> {
  const engine = opts.engine ?? 'mysql';
  await page.route(new RegExp(`/connections/${connId}/db/`), async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const op = url.pathname.split('/db/')[1] ?? '';
    const body = (() => {
      try {
        return (req.postDataJSON() ?? {}) as Record<string, any>;
      } catch {
        return {} as Record<string, any>;
      }
    })();
    switch (op) {
      case 'capabilities':
        return json(route, {
          engine,
          sql: true,
          joins: true,
          transactions: false,
          multi_statement: true,
          cancel: true,
          explain: true,
          default_port: engine === 'postgres' ? 5432 : engine === 'clickhouse' ? 8123 : 3306,
          schema_levels: ['database', 'table', 'column'],
          query_language: 'sql',
        });
      case 'schema':
        return json(route, [
          { id: 'db:shop', label: 'shop', kind: 'database', has_children: true },
          { id: 'db:analytics', label: 'analytics', kind: 'database', has_children: true },
        ]);
      case 'schema/children': {
        const path = String(body.path ?? '');
        if (path === 'db:shop')
          return json(route, [
            { id: 'db:shop/folder:tables', label: 'Tables', kind: 'folder', detail: '4', has_children: true },
            { id: 'db:shop/folder:views', label: 'Views', kind: 'folder', detail: '1', has_children: true },
          ]);
        if (path === 'db:shop/folder:tables' || path === 'db:shop/folder:views') {
          const kind = path.endsWith('views') ? 'view' : 'table';
          return json(
            route,
            MOCK_TABLES.filter((t) => t.kind === kind).map((t) => ({
              id: tablePath(t),
              label: t.name,
              kind: t.kind,
              detail: body.counts ? String(t.rows) : undefined,
              has_children: true,
            })),
          );
        }
        const t = MOCK_TABLES.find((x) => tablePath(x) === path);
        if (t)
          return json(
            route,
            t.cols.map((c) => ({ id: `${path}/column:${c.name}`, label: c.name, kind: 'column', detail: c.type, has_children: false })),
          );
        return json(route, []);
      }
      case 'object': {
        if (opts.objectDelay) await sleep(opts.objectDelay);
        const t = MOCK_TABLES.find((x) => tablePath(x) === body.path);
        if (!t) return route.fulfill({ status: 404, contentType: 'application/json', body: '{"error":"not found"}' });
        return json(route, detailOf(t));
      }
      case 'query': {
        const stmt = String(body.statement ?? '');
        opts.seen?.push(stmt);
        if (opts.queryDelay) await sleep(opts.queryDelay);
        return json(route, resultFor(stmt));
      }
      case 'test':
        return json(route, { ok: true, latency_ms: 3, message: 'ok', server_version: engine === 'postgres' ? '16.4' : '8.0.36' });
      case 'completion':
        return json(route, { items: [] });
      case 'completion/refresh':
        return json(route, {});
      case 'cancel':
        return json(route, { status: 'not_running' });
      case 'search-objects':
        return json(route, { hits: [], truncated: false, scanned: 1, supported: true });
      case 'schema-graph':
        return json(route, { schema: 'shop', tables: [], edges: [], relationships: true, truncated: false });
      case 'close':
        return json(route, {});
      default:
        // history / saved queries etc. live on the real test daemon.
        return route.continue();
    }
  });
}
