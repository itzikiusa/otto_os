// Agent UI control — Message Brokers (Kafka) handlers (`otto.ui_brokers_*`).
// The Brokers page (`#/brokers`, part of the Connections pane) keeps its view
// state in components — the cluster sub-tab in BrokersPage, the open topic in
// TopicsTab, the peek form in TopicDetail — so each binds a page port (see
// pagePort.ts) and the handlers drive the page through it: the user watches
// the tab switch, the topic open and the peek run in their own grid.
//
// Producing is `local_write` (per the design): an attributed confirm the user
// can remember for the cluster; a guarded cluster (prod or read-only) always
// gets the page's existing danger confirm on top and sends `confirm: true`.

import { tick } from 'svelte';
import { registerUiCommands, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { api } from '../api/client';
import { brokers } from '../stores/brokers.svelte';
import { ws } from '../stores/workspace.svelte';
import { confirmer } from '../confirm.svelte';
import { toasts } from '../toast.svelte';
import type {
  BrokerCluster,
  ConsumeResp,
  GroupSummary,
  MessageHeader,
  ProduceReq,
  TopicSummary,
} from '../api/types';
import type { ClusterView } from '../../modules/brokers/types';
import { agentLabel, asUiError, capList, dismissOnAbort, createPagePort, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

/** BrokersPage: the cluster sub-tab strip. */
export const brokersPagePort = createPagePort<{ setTab(view: ClusterView): void }>('the Message Brokers page');

/** TopicsTab (per cluster): which topic's detail is open. */
export const brokersTopicsPort = createPagePort<{
  clusterId: string;
  open(topic: string): void;
}>('the Topics tab');

export interface PeekOpts {
  start?: 'latest' | 'beginning';
  partition?: number;
  limit?: number;
  key_filter?: string;
  value_filter?: string;
}

/** TopicDetail (per topic): run a peek with the agent's options in the
 *  user's form, and prefill the produce form so the user sees what's sent. */
export const brokersTopicPort = createPagePort<{
  clusterId: string;
  topic: string;
  peek(o: PeekOpts): Promise<ConsumeResp | null>;
  showProduce(o: { key?: string; value: string; partition?: number }): void;
  produced(): void;
}>('the topic view');

const VIEWS: ClusterView[] = ['overview', 'topics', 'groups', 'schema', 'replay', 'alerts'];

async function clusterFor(key: string, ctx: UiCommandCtx): Promise<BrokerCluster> {
  if (brokers.clusters.length === 0 && ws.currentId) await brokers.load(ws.currentId);
  if (ctx.signal.aborted) throw new UiCommandError('cancelled_by_user', 'Cancelled');
  return resolveByIdOrName(brokers.clusters, key, (c) => c.id, (c) => c.name, 'Kafka cluster');
}

const guarded = (c: BrokerCluster): boolean => c.read_only || c.environment === 'prod';

/** Show the Brokers page with `c` open on `view`. */
async function showCluster(c: BrokerCluster, view: ClusterView, ctx: UiCommandCtx): Promise<void> {
  if (router.parts[0] !== 'brokers') router.go('brokers');
  const page = await brokersPagePort.get(ctx.signal);
  brokers.select(c.id);
  // BrokersPage resets its tab to Overview when the selection changes — let
  // that effect flush first, then switch.
  await tick();
  page.setTab(view);
  await tick();
}

async function showTopic(c: BrokerCluster, topic: string, ctx: UiCommandCtx) {
  await showCluster(c, 'topics', ctx);
  const topics = await waitFor(
    () => {
      const p = brokersTopicsPort.peek();
      return p && p.clusterId === c.id ? p : null;
    },
    ctx.signal,
    10_000,
    'the Topics tab',
  );
  topics.open(topic);
  const detail = await waitFor(
    () => {
      const p = brokersTopicPort.peek();
      return p && p.clusterId === c.id && p.topic === topic ? p : null;
    },
    ctx.signal,
    10_000,
    'the topic view',
  );
  await tick(); // TopicDetail resets its form on a topic change
  return detail;
}

function summarizeMessages(r: ConsumeResp) {
  return {
    truncated: r.truncated,
    masked: !!r.masked,
    partitions: r.partitions,
    messages: r.messages.slice(0, 200).map((m) => ({
      partition: m.partition,
      offset: m.offset,
      timestamp_ms: m.timestamp_ms,
      key: m.key?.text ?? null,
      value: m.value?.text ?? null,
      format: m.value?.format ?? null,
      headers: m.headers,
      size_bytes: m.size_bytes,
    })),
  };
}

registerUiCommands('connections', {
  async brokers_list_clusters(_args, ctx) {
    if (router.parts[0] !== 'brokers') router.go('brokers');
    if (ws.currentId) await brokers.load(ws.currentId);
    void highlightWhenReady(ctx, '.clusters, [aria-label="Kafka cluster views"]');
    return {
      clusters: brokers.clusters.map((c) => ({
        id: c.id,
        name: c.name,
        bootstrap_servers: c.bootstrap_servers,
        environment: c.environment,
        read_only: c.read_only,
      })),
    };
  },

  async brokers_open(args: { cluster: string; view?: ClusterView }, ctx) {
    const c = await clusterFor(args.cluster, ctx);
    const view = args.view ?? 'overview';
    if (!VIEWS.includes(view)) throw new UiCommandError('invalid_args', `Unknown view “${view}” (one of ${VIEWS.join(', ')}).`);
    await showCluster(c, view, ctx);
    void highlightWhenReady(ctx, '[aria-label="Kafka cluster views"]');
    return { cluster: { id: c.id, name: c.name, environment: c.environment }, view };
  },

  async brokers_list_topics(args: { cluster: string; query?: string; include_internal?: boolean }, ctx) {
    const c = await clusterFor(args.cluster, ctx);
    await showCluster(c, 'topics', ctx);
    try {
      const all = await api.get<TopicSummary[]>(`/brokers/clusters/${c.id}/topics`);
      const q = (args.query ?? '').toLowerCase();
      const list = all
        .filter((t) => args.include_internal || !t.internal)
        .filter((t) => !q || t.name.toLowerCase().includes(q))
        .sort((a, b) => a.name.localeCompare(b.name));
      const { items, total, truncated } = capList(list);
      return { cluster: c.name, total, truncated, topics: items };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async brokers_list_groups(args: { cluster: string }, ctx) {
    const c = await clusterFor(args.cluster, ctx);
    await showCluster(c, 'groups', ctx);
    try {
      const groups = await api.get<GroupSummary[]>(`/brokers/clusters/${c.id}/groups`);
      const { items, total, truncated } = capList(groups);
      return { cluster: c.name, total, truncated, groups: items };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async brokers_open_topic(args: { cluster: string; topic: string }, ctx) {
    const c = await clusterFor(args.cluster, ctx);
    await showTopic(c, args.topic, ctx);
    return { cluster: c.name, topic: args.topic };
  },

  async brokers_peek(args: { cluster: string; topic: string } & PeekOpts, ctx) {
    const c = await clusterFor(args.cluster, ctx);
    const detail = await showTopic(c, args.topic, ctx);
    const r = await detail.peek({
      start: args.start,
      partition: args.partition,
      limit: Math.min(Math.max(args.limit ?? 50, 1), 200),
      key_filter: args.key_filter,
      value_filter: args.value_filter,
    });
    if (!r) throw new UiCommandError('failed', `Couldn't read messages from “${args.topic}” (the page shows why).`);
    return { cluster: c.name, topic: args.topic, ...summarizeMessages(r) };
  },

  async brokers_produce(
    args: { cluster: string; topic: string; value: string; key?: string; partition?: number; headers?: MessageHeader[] },
    ctx,
  ) {
    const c = await clusterFor(args.cluster, ctx);
    const detail = await showTopic(c, args.topic, ctx);
    // The produce form shows exactly what will be sent while the user decides.
    detail.showProduce({ key: args.key, value: args.value, partition: args.partition });
    void highlightWhenReady(ctx, '.produce, [data-tab="produce"]');
    const who = agentLabel(ctx.agent);
    const where = `topic “${args.topic}” on ${c.name}${c.environment === 'prod' ? ' (PRODUCTION)' : ''}`;
    // A guarded cluster never offers/honours the session memory, and adds the
    // page's own danger confirm on top.
    let ok = await ctx.confirmWrite({
      what: `Produce a message (${args.value.length} bytes; shown in the Produce form)`,
      where,
      connId: `broker:${c.id}`,
      verb: 'Produce',
      guarded: guarded(c),
    });
    if (ok && guarded(c)) {
      ctx.progress(`Waiting for you to confirm producing to ${args.topic}`, true);
      ok = await dismissOnAbort(ctx.signal, confirmer.ask(`${who} wants to produce to “${args.topic}” on guarded cluster “${c.name}”.`, {
        title: 'Produce to guarded cluster',
        confirmLabel: 'Produce',
        danger: true,
      }));
    }
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined producing the message.');
    const req: ProduceReq = {
      partition: args.partition ?? null,
      key: args.key ?? null,
      value: args.value,
      headers: args.headers?.length ? args.headers : undefined,
      confirm: guarded(c),
    };
    try {
      const r = await api.post<{ partition: number; offset: number }>(
        `/brokers/clusters/${c.id}/topics/${encodeURIComponent(args.topic)}/produce`,
        req,
      );
      toasts.success(`Produced to partition ${r.partition} @ offset ${r.offset}`, who);
      detail.produced();
      return { cluster: c.name, topic: args.topic, partition: r.partition, offset: r.offset };
    } catch (e) {
      throw asUiError(e);
    }
  },
});
