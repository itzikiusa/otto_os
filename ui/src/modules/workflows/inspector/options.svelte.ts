// The node inspector's picker lists (repositories, swarms, DB connections,
// broker clusters/topics, product stories, Jira accounts), so no form asks a
// person to type an opaque id the UI never shows. One instance per page: each
// list is fetched ONCE per key (workspace, swarm or cluster) — pending or
// failed — because the forms' effects re-run per keystroke, and used to
// re-fetch per character after a failure. `rows(key)` is null while loading or
// after a failure; the forms then fall back to a free-text input so a saved id
// stays editable, and `retry(key)` asks again.
import { api } from '../../../lib/api/client';
import { listIssueAccounts } from '../../../lib/api/issues';
import { loadErrorText } from '../../../lib/loadError';
import type { BrokerCluster, Connection, IssueAccount, Repo, TopicSummary } from '../../../lib/api/types';
import type { ProductStory } from '../../../lib/api/types/product';

export interface Named {
  id: string;
  name: string;
}

export class OptionCache<T> {
  #rows = $state<Record<string, T[]>>({});
  #errors = $state<Record<string, string>>({});
  #tried = new Set<string>();
  readonly #fetch: (key: string) => Promise<T[]>;

  constructor(fetch: (key: string) => Promise<T[]>) {
    this.#fetch = fetch;
  }

  rows(key: string): T[] | null {
    return this.#rows[key] ?? null;
  }

  error(key: string): string | null {
    return this.#errors[key] ?? null;
  }

  loading(key: string): boolean {
    return !!key && !this.#rows[key] && !this.#errors[key];
  }

  /** Fetch `key`'s list unless it was already tried. Call from an effect. */
  ensure(key: string): void {
    if (!key || this.#tried.has(key)) return;
    this.#tried.add(key);
    this.#fetch(key)
      .then((rows) => {
        this.#rows = { ...this.#rows, [key]: rows };
      })
      .catch((e) => {
        this.#errors = { ...this.#errors, [key]: loadErrorText(e) };
      });
  }

  retry(key: string): void {
    this.#tried.delete(key);
    const rest = { ...this.#errors };
    delete rest[key];
    this.#errors = rest;
    this.ensure(key);
  }
}

/** Database Explorer kinds (lib/stores/database `DB_KINDS`) — a `db_query`
 *  node runs against one of these, never an ssh/custom connection. */
const DB_QUERY_KINDS = new Set(['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse']);

export class InspectorOptions {
  readonly repos = new OptionCache<Repo>((wsId) => api.get<Repo[]>(`/workspaces/${wsId}/repos`));
  readonly swarms = new OptionCache<Named>((wsId) =>
    api.get<Named[]>(`/workspaces/${wsId}/swarm/swarms`).then((l) => l.map((x) => ({ id: x.id, name: x.name }))),
  );
  readonly swarmProjects = new OptionCache<Named>((sid) =>
    api
      .get<{ projects: Named[] }>(`/swarm/swarms/${encodeURIComponent(sid)}`)
      .then((d) => d.projects.map((x) => ({ id: x.id, name: x.name }))),
  );
  readonly dbConnections = new OptionCache<Connection>((wsId) =>
    api.get<Connection[]>(`/workspaces/${wsId}/connections`).then((all) => all.filter((c) => DB_QUERY_KINDS.has(c.kind))),
  );
  readonly brokerClusters = new OptionCache<BrokerCluster>((wsId) =>
    api.get<BrokerCluster[]>(`/workspaces/${wsId}/brokers/clusters`),
  );
  readonly brokerTopics = new OptionCache<TopicSummary>((cid) =>
    api.get<TopicSummary[]>(`/brokers/clusters/${encodeURIComponent(cid)}/topics`),
  );
  readonly stories = new OptionCache<ProductStory>((wsId) =>
    api.get<ProductStory[]>(`/workspaces/${wsId}/product/stories`),
  );
  /** Issue accounts are per user, not per workspace: one key. */
  readonly accounts = new OptionCache<IssueAccount>(() => listIssueAccounts());
}

/** A full ULID — a free-text id complete enough to look up. */
export const isUlid = (s: string): boolean => /^[0-9A-HJKMNP-TV-Z]{26}$/i.test(s);
