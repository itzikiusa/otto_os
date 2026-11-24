// Pure model of the Connections list (ConnectionsPage): one row shape for
// saved profiles and Kafka clusters, the words each row says, the search /
// filter, and the grouping into sections. No Svelte, no fetches — unit-tested
// in ui/unit/connectionRows.test.ts.

import type { BrokerCluster, Connection, ConnectionSection, Id } from '../../lib/api/types';
import type { ConnCheck } from '../../lib/stores/connectionList.svelte';
import { resourceWord, type StatusWord } from '../../lib/statusWords.ts';
import { KIND_LABEL } from '../../lib/database/kindLabel.ts';

export { KIND_LABEL };

export type RowKind = Connection['kind'] | 'kafka';

/** The filter menu's kind order. */
export const KIND_ORDER: RowKind[] = ['mysql', 'postgres', 'clickhouse', 'mongodb', 'redis', 'ssh', 'kafka', 'custom'];

export interface ListItem {
  id: Id;
  name: string;
  kind: RowKind;
  /** "orders-db.internal:3306", "bastion → prod-mysql", "broker-1:9092". */
  host: string;
  sectionId: Id | null;
  environment: Connection['environment'];
  readOnly: boolean;
  lastOpened: string | null;
  conn?: Connection;
  cluster?: BrokerCluster;
}

function str(v: unknown): string {
  return v == null ? '' : String(v);
}

/** Where a profile points, in one short phrase. A database behind an SSH
 *  tunnel reads "bastion → db-host". */
export function hostOf(c: Connection): string {
  const p = (c.params ?? {}) as Record<string, unknown>;
  let host = str(p.host || p.uri || p.url || p.conn_string || p.path);
  // A URI may carry credentials; show only where it points.
  host = host.replace(/^[a-z+]+:\/\/(?:[^@/]*@)?/i, '').replace(/[/?].*$/, '');
  const port = p.port != null && p.port !== '' ? `:${str(p.port)}` : '';
  const target = host ? `${host}${host.includes(':') ? '' : port}` : '';
  const ssh = (p.ssh ?? null) as Record<string, unknown> | null;
  const via = str(ssh?.host || p.jump);
  return via && target ? `${via} → ${target}` : target || via;
}

export function fromConnection(c: Connection): ListItem {
  return {
    id: c.id,
    name: c.name,
    kind: c.kind,
    host: hostOf(c),
    sectionId: c.section_id,
    environment: c.environment,
    readOnly: c.read_only,
    lastOpened: c.last_opened_at ?? null,
    conn: c,
  };
}

export function fromCluster(cl: BrokerCluster): ListItem {
  return {
    id: cl.id,
    name: cl.name,
    kind: 'kafka',
    host: cl.bootstrap_servers.split(',')[0]?.trim() ?? '',
    sectionId: cl.section_id ?? null,
    environment: cl.environment,
    readOnly: cl.read_only,
    lastOpened: null,
    cluster: cl,
  };
}

/** "PLATFORM / STG" — the folder path of a section id ('' when none/unknown). */
export function sectionPath(sections: readonly ConnectionSection[], id: Id | null): string {
  if (!id) return '';
  const byId = new Map(sections.map((s) => [s.id, s]));
  const parts: string[] = [];
  let cur = byId.get(id);
  for (let guard = 0; cur && guard < 20; guard++) {
    parts.unshift(cur.name);
    cur = cur.parent_id ? byId.get(cur.parent_id) : undefined;
  }
  return parts.join(' / ');
}

const MIN = 60_000;

/** A good result older than this is no longer evidence: "Not checked". */
export const CHECK_FRESH_MS = 30 * MIN;

const AUTH_RE = /access denied|authenticat|password|auth failed|not authori[sz]ed|permission denied \(publickey|login failed|sasl/i;

export interface RowWord extends StatusWord {
  /** "tunnel refused" — why it can't connect (line 3), else null. */
  reason: string | null;
  /** "14 ms" for a fresh good check, else null. */
  latency: string | null;
  /** Tooltip: what was observed and when. */
  tip: string;
}

/** The row's state word from the last known check and its age — never from
 *  a test fired because the list rendered. `ago` formats the check's time
 *  (the page passes the app's one relative-time formatter, lib/stores/now). */
export function rowWord(check: ConnCheck | undefined, now: number, ago: (at: number) => string): RowWord {
  if (!check) return { ...resourceWord(null), reason: null, latency: null, tip: 'Not checked yet — open it or use Test' };
  const age = Math.max(0, now - check.at);
  const when = ago(check.at);
  if (!check.ok) {
    const msg = (check.message ?? '').trim();
    const state = AUTH_RE.test(msg) ? 'auth' : 'error';
    return { ...resourceWord(state), reason: shortReason(msg), latency: null, tip: `${msg || 'The server gave no reason'} (${when})` };
  }
  if (age > CHECK_FRESH_MS) return { ...resourceWord(null), reason: null, latency: null, tip: `Last connected ${when}` };
  const latency = check.latencyMs != null ? `${Math.round(check.latencyMs)} ms` : null;
  return { ...resourceWord('connected'), reason: null, latency, tip: `Connected ${when}${check.serverVersion ? ` · ${check.serverVersion}` : ''}` };
}

/** First clause of a driver error, short enough for line 3. */
export function shortReason(msg: string): string | null {
  const m = msg.replace(/\s+/g, ' ').trim();
  if (!m) return null;
  const first = m.split(/(?<=[a-z)])[.:;] |\n/i)[0] ?? m;
  return first.length > 60 ? `${first.slice(0, 57)}…` : first;
}

export interface ListFilter {
  q: string;
  /** 'all' or one kind. */
  kind: string;
  /** null = every section; '' = ungrouped only. */
  section: Id | null;
}

export function matches(item: ListItem, f: ListFilter, sections: readonly ConnectionSection[]): boolean {
  if (f.kind !== 'all' && item.kind !== f.kind) return false;
  if (f.section !== null) {
    if (f.section === '' ? !!item.sectionId && sections.some((s) => s.id === item.sectionId) : !inSection(item.sectionId, f.section, sections)) return false;
  }
  const q = f.q.trim().toLowerCase();
  if (!q) return true;
  return [item.name, item.kind, KIND_LABEL[item.kind], item.host, sectionPath(sections, item.sectionId)].some((s) => s.toLowerCase().includes(q));
}

/** Whether `id` is `target` or nested under it. */
function inSection(id: Id | null, target: Id, sections: readonly ConnectionSection[]): boolean {
  const parent = new Map(sections.map((s) => [s.id, s.parent_id]));
  for (let cur = id, guard = 0; cur && guard < 20; guard++) {
    if (cur === target) return true;
    cur = parent.get(cur) ?? null;
  }
  return false;
}

export interface ListGroup {
  key: string;
  /** "PLATFORM / STG", "Ungrouped" — null when there is only one
   *  plain group (no sections at all): no heading then. */
  label: string | null;
  items: ListItem[];
  /** The section behind a group (its ⋯ menu), null for Ungrouped. */
  section: ConnectionSection | null;
}

const byName = (a: ListItem, b: ListItem): number => a.name.localeCompare(b.name);

/** Sections in tree order (parents before children, by position then name). */
export function orderedSections(sections: readonly ConnectionSection[]): ConnectionSection[] {
  const out: ConnectionSection[] = [];
  const walk = (parent: Id | null): void => {
    for (const s of sections
      .filter((x) => (x.parent_id ?? null) === parent)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))) {
      out.push(s);
      walk(s.id);
    }
  };
  walk(null);
  return out;
}

/**
 * The list's groups: one per section (tree order, the path as its heading),
 * then "Ungrouped". With no sections at all it is one heading-less group.
 * Empty sections are listed too (so they can be renamed or deleted) unless a
 * search or filter is narrowing the list.
 */
export function groupItems(items: readonly ListItem[], sections: readonly ConnectionSection[], narrowed: boolean): ListGroup[] {
  const groups: ListGroup[] = [];
  const known = new Set(sections.map((s) => s.id));
  const loose = items.filter((i) => !i.sectionId || !known.has(i.sectionId)).sort(byName);
  if (sections.length === 0) {
    groups.push({ key: 'all', label: null, items: loose, section: null });
    return groups;
  }
  for (const s of orderedSections(sections)) {
    const inIt = items.filter((i) => i.sectionId === s.id).sort(byName);
    if (inIt.length === 0 && narrowed) continue;
    groups.push({ key: s.id, label: sectionPath(sections, s.id), items: inIt, section: s });
  }
  if (loose.length > 0) groups.push({ key: 'ungrouped', label: 'Ungrouped', items: loose, section: null });
  return groups;
}
