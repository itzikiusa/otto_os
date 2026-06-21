# Message Brokers (Kafka viewer)

A Conduktor/Confluent-class Kafka viewer built into Otto: connect Kafka clusters
— including **AWS MSK in a private VPC reached over an SSH bastion** — to browse
topics, peek and produce messages, inspect consumer-group lag, edit topic
configs, view a Schema Registry, and watch a live Overview of
brokers/partitions/throughput plus broker CPU/RAM. It speaks PLAINTEXT/TLS and
SASL (PLAIN / SCRAM-SHA-256 / SCRAM-SHA-512), enforces production / read-only
write-guards, and — because librdkafka has no SOCKS support and cannot override
the *advertised* broker addresses it learns from a cluster — runs an
**in-process, Kafka-aware reverse proxy** so an entire private cluster is
reachable through a single SSH tunnel.

This document is the end-user and operator reference for the feature. The Rust
API contract in [`docs/contracts/api.md`](../contracts/api.md) (section *Message
Brokers (Kafka viewer)*) is authoritative; the TypeScript types in
`ui/src/lib/api/types.ts` mirror it.

---

## 1. Overview & where it lives

The feature is reached from the **Message Brokers** area of the Otto UI. Its
left rail is a sectioned, Conduktor-style **sidebar** of saved cluster profiles;
opening a cluster gives you a **multi-tab** workspace (Overview / Topics /
Consumer Groups / Schema Registry / Replay / Lag Alerts). Each open cluster is a
tab in a tabstrip, so you can flip between clusters without losing context.

| Concern | Location |
|---|---|
| Daemon crate (Kafka driver, service, proxy) | `crates/otto-brokers/` (`rdkafka` / librdkafka) |
| SSH-tunnel helper (shared with DB Explorer) | `crates/otto-ssh/` |
| Kafka-aware proxy (wire rewrites) | `crates/otto-brokers/src/proxy/` (`protocol.rs` pure, `runtime.rs` async) |
| HTTP router | `crates/otto-brokers/src/http.rs` |
| Service façade | `crates/otto-brokers/src/service.rs` |
| Domain / API DTOs | `crates/otto-brokers/src/types.rs` |
| Schema Registry client | `crates/otto-brokers/src/schema_registry.rs` |
| Metrics (throughput + Prometheus scrape) | `crates/otto-brokers/src/metrics.rs` |
| UI module | `ui/src/modules/brokers/` (`BrokersPage`, `ClusterForm`, `TopicsTab`, `TopicDetail`, `GroupsTab`, `SchemaTab`, `OverviewTab`, `ReplayPanel`, `LagAlertsPanel`, …) |
| Persistence | SQLite migrations `0040_brokers`, `0047_broker_ssh`, `0048_broker_cluster_sections`, `0052_broker_audit`, `0055_broker_ops` |
| Secrets | macOS Keychain refs `broker-{id}` (SASL) / `broker-sr-{id}` (Schema Registry) |

### Architecture at a glance

```
Svelte UI ──HTTP+WS──▶ ottod (127.0.0.1:7700) ──▶ otto-brokers service
                                                      ├─ rdkafka (librdkafka) ──▶ Kafka brokers
                                                      ├─ Schema Registry client (reqwest)
                                                      └─ metrics scraper (Prometheus)
                       (private cluster) ───────────▶ otto-ssh (ssh -D SOCKS5) + Kafka-aware proxy
```

The daemon owns a per-cluster connection pool (a 30-minute-TTL `rdkafka` client,
the schema-registry client, and — for tunnelled clusters — the SSH tunnel). The
pool is evicted whenever a cluster profile is edited or deleted.

### Roles & ownership

- **Reads** (list/get cluster, overview, metrics, topics, peek, groups, schema,
  sections) require workspace **Viewer**.
- **Cluster management and mutations** (create/update/delete cluster, create/
  delete topic, alter configs, produce, reset offsets, replay, sections) require
  workspace **Editor**.
- **Global clusters** (`workspace_id = null`) are a shared infrastructure
  library visible in every workspace and are **managed by root only** — a
  non-root Editor gets `403` on management calls against them. In the current
  build, clusters created through the API are created workspace-independent
  (global library, like DB connections); the path workspace is used only to
  authorize the caller.

---

## 2. Adding a cluster

Open the **Add cluster** form (the `+` button on the *Clusters* sidebar header,
or the **Add a cluster** button on the empty state). Required fields are **Name**
and **Bootstrap servers**; everything else depends on how the cluster
authenticates.

- **Name** — display label (e.g. `prod-kafka`).
- **Bootstrap servers** — comma-separated `host:port` list
  (`broker1:9092,broker2:9092`). A missing port defaults to `9092`.
- **Security** — transport + auth scheme (see the matrix below).
- **Schema registry URL** *(optional)* — enables automatic Avro decode and the
  Schema Registry tab (e.g. `http://schema-registry:8081`).
- **Metrics URL** *(optional)* — a Prometheus exposition endpoint for broker
  CPU/RAM (e.g. Redpanda `http://broker:9644/public_metrics`, or a Kafka JMX
  exporter).
- **SSH tunnel** *(optional)* — bastion to reach a private cluster (see §4).
- **Environment** — `dev` / `staging` / `prod`. `prod` arms the write-guard.
- **Accent color** *(optional)* — a hex color for the sidebar dot/tab.
- **Read-only** — arms the write-guard regardless of environment.

After saving, use **Test** in the cluster header (or right-click → Test). The
test endpoint **never returns 5xx**: a bad config comes back as
`{ ok: false, message: "<error>" }` rather than an HTTP error, so connection
problems surface as a clear toast.

### Auth matrix (Security ↔ what to fill in)

| Security (`security_protocol`) | librdkafka `security.protocol` | Transport | Fields you must provide |
|---|---|---|---|
| `plaintext` | `PLAINTEXT` | TCP, no TLS | bootstrap only |
| `ssl` | `SSL` | TLS | bootstrap; optional **Skip TLS verify** |
| `sasl_plaintext` | `SASL_PLAINTEXT` | TCP + SASL | bootstrap, **SASL mechanism**, **SASL username**, **SASL password** |
| `sasl_ssl` | `SASL_SSL` | TLS + SASL | bootstrap, **SASL mechanism**, **SASL username**, **SASL password**; optional **Skip TLS verify** |

**SASL mechanisms** (username/password only — no Kerberos/GSSAPI/OAuth in this
version):

| UI label | Wire value (`sasl_mechanism`) | librdkafka `sasl.mechanism` |
|---|---|---|
| PLAIN | `plain` | `PLAIN` |
| SCRAM-SHA-256 | `scram_sha_256` | `SCRAM-SHA-256` |
| SCRAM-SHA-512 | `scram_sha_512` | `SCRAM-SHA-512` |

> The wire strings are exactly `scram_sha_256` / `scram_sha_512` (underscore
> before the digits). The form sends these verbatim; sending the wrong casing
> yields a `422`.

**Skip TLS certificate verification** appears only for the TLS protocols (`ssl`
/ `sasl_ssl`). Use it for self-signed brokers; it sets librdkafka's
`enable.ssl.certificate.verification=false`. Signatures are still checked — only
chain validity and hostname matching are skipped.

### Secret handling (create vs edit)

Passwords are **write-only**. They are stored in the macOS Keychain
(`broker-{id}` for SASL, `broker-sr-{id}` for the registry); the API only ever
returns `has_sasl_password` / `has_sr_password` booleans, never the value. The
update (PATCH) semantics are deliberate:

- **Absent** password field → keep the stored secret (the form shows
  `•••••• (unchanged)` as the placeholder).
- **Non-empty** value → replace the secret.
- **Empty string** → clear the secret.

The same three-state PATCH rule governs the optional `ssh` tunnel and the
`section_id`: **absent = keep**, `null` = clear/ungroup, an object/id = set.
`environment` and `read_only` are likewise preserved when omitted, so a routine
edit never silently disarms the guard.

---

## 3. The sidebar: sections, multi-tab, drag-and-drop

The sidebar groups cluster profiles into **sections** (folders) that nest into a
tree. Clusters that aren't filed appear under **Ungrouped**.

- **New section** — the folder button on the sidebar header (or right-click a
  section → *New sub-section*).
- **Move a cluster** — drag it onto a section header, or drop it on *Ungrouped*
  to remove it from its section.
- **Reparent a section** — drag the section header onto another (you can't nest
  a section inside itself or a descendant; the UI blocks that).
- **Rename / Delete** — right-click a section. Deleting a section cascades to its
  sub-sections; the clusters inside fall back to **Ungrouped** (they are never
  deleted).
- **Collapse** — each section header has a caret; collapse state is local to your
  view.

Global clusters (`workspace_id = null`) are not assignable to a section and
always render as Ungrouped.

**Multi-tab:** selecting a cluster opens it as a tab in the tabstrip; multiple
clusters can be open at once. The colored dot uses the cluster's accent color.
On a phone the sidebar and the cluster content stack as two independently
collapsible, scrollable sections.

Sections are workspace-scoped and stored in `broker_cluster_sections`
(migration `0048`).

---

## 4. MSK / private cluster over an SSH bastion (deep dive)

This is the headline capability: reach a Kafka cluster that lives on a private
network (classically **AWS MSK in a VPC**) through a single SSH bastion, with no
VPN and no client-side `/etc/hosts` surgery.

### Why a plain SSH `-L` forward is not enough

A Kafka client first asks any reachable broker for cluster **Metadata**. The
broker replies with the **advertised listener** addresses of *every* broker
(e.g. `b-1.mycluster.kafka.us-east-1.amazonaws.com:9094`,
`b-2.…:9094`, …). The client then dials those advertised addresses directly. So:

- A single `ssh -L localport:broker:9094` forward maps **one** local port to
  **one** broker. The moment the client follows the advertised metadata to a
  *second* broker (or to the group coordinator via `FindCoordinator`), it tries
  to open a connection to a private DNS name it can't resolve or route to, and
  the operation hangs/fails.
- **librdkafka has no SOCKS support**, and **no setting to override the
  advertised broker addresses** it learns at runtime. So neither "just use a
  SOCKS proxy" nor "rewrite the bootstrap list" is sufficient on its own.

### How Otto solves it: one SOCKS tunnel + an in-process Kafka-aware proxy

When a cluster profile carries an `ssh` tunnel, the daemon (per cluster):

1. **Opens one `ssh -D` SOCKS5 tunnel** to the bastion
   (`otto_ssh::SshTunnel::open_socks` → `ssh -N -D 127.0.0.1:<port> user@host`).
   The tunnel uses the system `ssh` client, so it honours your **ssh-agent**,
   `~/.ssh/config`, and `known_hosts`. Auth is **key/agent only** (provide an
   identity file or rely on the agent — there is no SSH password field, so there
   is no SSH secret in the Keychain). The tunnel is brought up with
   `BatchMode=yes`, `ExitOnForwardFailure=yes`, `ConnectTimeout=10`, and a
   `ServerAliveInterval=15` keep-alive, and is considered ready once the local
   SOCKS port accepts a connection (≈12 s budget).

2. **Binds a local TCP listener per real broker** and hands librdkafka a
   **local** bootstrap list (`127.0.0.1:<localport>,…`). librdkafka talks
   **plaintext to these local listeners**.

3. **Forwards each accepted connection to the real broker through SOCKS**
   (`tokio_socks::Socks5Stream`, with *remote* DNS resolution so the private
   broker hostname resolves on the bastion's side, not yours). When the cluster
   uses TLS, the proxy **originates TLS to the broker itself** with the correct
   SNI (`tokio-rustls`), terminating it at the proxy. The local hop to
   librdkafka stays plaintext.

4. **Rewrites broker addresses on the fly.** As Kafka frames flow back from the
   broker, the proxy parses two response types and replaces every advertised
   broker `host:port` with `127.0.0.1:<localport>`, spinning up a new local
   listener for any broker it hasn't seen yet:
   - **`Metadata`** responses (the broker list).
   - **`FindCoordinator`** responses (the group/transaction coordinator
     address).

5. **Clamps `ApiVersions`.** The proxy clamps the `ApiVersions` response so
   librdkafka requests `Metadata ≤ v8` and `FindCoordinator ≤ v2`. Those
   versions are the **non-flexible** wire format (classic int32 array counts,
   int16-length strings), which keeps the address-rewriting logic simple and
   robust. The rewrites are pure, byte-level, big-endian frame surgery
   (`proxy/protocol.rs`), exhaustively unit-tested; on any parse error the proxy
   forwards the original bytes unchanged rather than corrupting the stream.

6. **Routes the Schema Registry and metrics scrape through the same tunnel.**
   Both ride the SOCKS proxy (`socks5h://127.0.0.1:<port>`), so a private
   registry/metrics endpoint is reachable too. When reached via the tunnel, the
   registry client intentionally **skips the SSRF guard** — the private endpoint
   is the whole point and the bastion credentials are the authority.

TLS detail: the proxy→broker hop validates against the bundled webpki roots
(which include Amazon's CA, so **MSK TLS certs validate out of the box**). With
**Skip TLS verify** set, an accept-any verifier is installed (chain/hostname
unchecked; signatures still verified).

The connection pool keeps the tunnel alive for the life of the pooled client and
re-opens it automatically if the `ssh` child dies. In the Overview, the broker
addresses you see are **reverse-mapped back to the real broker host:port** (not
the internal `127.0.0.1:<local>` the client actually talks to).

### Step-by-step: connect to AWS MSK over a bastion

1. In the MSK console, copy the cluster's **bootstrap brokers** string for the
   auth you'll use — TLS (`:9094`) or SASL/SCRAM (`:9096`).
2. Make sure you can already `ssh` to the bastion from a terminal (agent or key
   loaded). The bastion's `sshd` must allow TCP forwarding
   (`AllowTcpForwarding yes`) — required for the `-D` SOCKS dynamic forward.
3. **Add cluster** in Otto:
   - **Bootstrap servers** = the MSK broker string (the private
     `b-N.<cluster>.kafka.<region>.amazonaws.com:<port>` hosts).
   - **Security** = `SSL` for TLS-only MSK, or `SASL_SSL` + **SCRAM-SHA-512**
     with the username/password from the MSK SCRAM secret.
   - **Environment** = `prod` if it is production (arms the guard).
4. Tick **SSH tunnel** and fill in:
   - **Tunnel host** — the bastion's public DNS/IP (e.g. an EC2 instance in the
     VPC).
   - **Port** — `22` by default.
   - **Tunnel user** — e.g. `ec2-user`.
   - **Identity file** *(optional)* — leave blank to use your ssh-agent, or
     **Browse…** to a private key.
5. **Save**, then **Test**. On success the cluster header shows a **Tunnel**
   pill that turns green when the SOCKS proxy is warm (selecting a tunnelled
   cluster fires Test in the background to warm it).
6. Open **Overview / Topics** — you're now browsing the private cluster through
   the single tunnel.

### Step-by-step: a plain (directly reachable) cluster

1. **Add cluster** → **Name** + **Bootstrap servers** (`broker1:9092,…`).
2. Pick **Security** (`PLAINTEXT` for a local dev broker; `SASL_SSL` +
   PLAIN/SCRAM for a secured one) and fill any SASL credentials.
3. *(Optional)* add a **Schema registry URL** and a **Metrics URL**.
4. **Save → Test → open Overview.** No SSH section needed.

---

## 5. Browsing topics & the Overview

### Overview tab

`GET /brokers/clusters/{id}/overview` returns a `ClusterOverview`: the broker
list (id, host, port, rack, controller flag, partition-leader count), topic
count (with internal topics broken out), partition count, consumer-group count,
and two health signals:

- **`under_replicated_partitions`** — partitions whose ISR count is below their
  replica count.
- **`leadership_imbalance`** — coefficient of variation of leader counts across
  brokers (`0.0` = perfectly balanced; higher = more skewed; omitted with < 2
  brokers).

Alongside, `GET /brokers/clusters/{id}/metrics` (a `ClusterMetrics`) is polled
every ~4 s and drives the throughput chart and broker CPU/RAM:

- **Throughput** is derived from the **high-watermark delta** between calls (a
  cluster-wide watermark sweep, fanned across 16 worker threads). The sweep is
  expensive over a tunnel, so a fresh value is cached for ~8 s and reused
  between polls.
- **Broker CPU/RAM** comes from the optional Prometheus `metrics_url`
  (`prometheus_available` is `false` when none is set). It understands Redpanda
  (`redpanda_*`), JVM (`jvm_memory_bytes_*`), and `process_*` metric names.

### Topics tab

`GET /brokers/clusters/{id}/topics` returns the topic list as a **metadata-only**
pass (name, partitions, replication factor, internal flag) — fast even on large
clusters. **Message counts and the cleanup policy are loaded lazily**: the table
batch-fetches stats for the visible page via
`POST /brokers/clusters/{id}/topics/stats` (`{ names: [...] }`, ≤ 500 names),
which fans counts across the shared watermark thread pool. A **Msg/s** column
shows the production rate derived server-side from the high-watermark delta
between two consecutive stats polls (the table re-polls visible rows every 5 s;
`—` until a second sample lands).

- **Search**, **Show internal** (internal topics — `__*`, `_schemas`,
  `_redpanda*` — are hidden by default), and a **cleanup-policy** filter.
- **New** creates a topic (`CreateTopicReq`: name, partitions, replication
  factor, optional configs). On a guarded cluster the form auto-includes
  `confirm`.
- The on-disk **Size** column shows `—` (not exposed by this Kafka client).

Click a row for **Topic detail** (`GET …/topics/{topic}`, a `TopicDetail`): a
**Messages** view (peek/produce), a **Partitions** view (per-partition leader,
replicas, ISR, low/high watermarks, message count), a **Config** view, and a
**Produce** form.

---

## 6. Peek & produce

### Peek (consume)

`POST /brokers/clusters/{id}/topics/{topic}/consume` with a `ConsumeReq`. Peeking
uses a **throwaway consumer group** (`otto-peek-…`) with **`enable.auto.commit =
false`** and a manual partition assignment, so it **never commits offsets or
disturbs your real consumers**.

- **`start`** — a tagged union: `{type:"beginning"}`, `{type:"latest"}` (the last
  `limit` messages — the default), `{type:"offset", offset}` (requires a single
  `partition`), or `{type:"timestamp", timestamp_ms}` (first message at/after the
  epoch-ms).
- **`partition`** — a single partition, or all partitions when omitted.
- **`limit`** — default `50`, clamped to `1…5000`.
- **`key_filter`** — case-insensitive substring on the raw key bytes, applied
  **server-side during the scan** (so the `limit` quota counts only matches);
  set `find_from_beginning: true` to scan from the earliest offset so the filter
  can reach old messages. A filtered scan is capped at 50 000 raw records.
- **`value_filter`** — case-insensitive substring on the **decoded** value
  (applied after decoding).
- **`decode`** — `auto` (default) | `json` | `utf8` | `hex` | `base64` |
  `protobuf` | `avro`. **`auto`** tries JSON → UTF-8 → schemaless Protobuf
  wire-decode → hex, and decodes **Confluent-framed Avro** automatically when a
  Schema Registry is configured (the magic byte + 4-byte schema id are
  recognized, the schema fetched by id and cached). Each decoded payload also
  carries `raw_base64`, so the UI can re-render a message as hex/base64 on
  demand.
- **`mask`** — when `true`, every key, value, and header value is run through
  `otto_core::redact` **server-side** before the response leaves the daemon (raw
  PII never reaches the client); the response is flagged `masked: true` and the
  UI shows a badge.

The response (`ConsumeResp`) includes the decoded `messages`, the per-partition
`low/high` watermark `partitions`, and `truncated: true` if the limit/timeout was
hit before draining the range. The detail view also supports an incremental
**live-tail** that polls only offsets past the max seen per partition into a
capped ring buffer.

### Produce

`POST /brokers/clusters/{id}/topics/{topic}/produce` with a `ProduceReq` (Editor;
guarded clusters require `confirm`). Fields: optional `partition` (else the
broker chooses), `key`, `value`, `headers` (`MessageHeader[]`), and the
`key_base64` / `value_base64` flags (interpret the input as base64 bytes). A
**tombstone** is produced by sending an **empty-string `value` with
`value_base64: false`**. The response is `{ partition, offset }`. Every produce
writes an audit row (see §10).

---

## 7. Consumer groups & lag

- `GET /brokers/clusters/{id}/groups` → `GroupSummary[]` (group id, state,
  protocol type, member count).
- `GET /brokers/clusters/{id}/groups/{group}` → `GroupDetail`: members (with
  client id, host, and assigned topic-partitions), per-partition offsets
  (`current_offset`, `high_watermark`, `lag`), and `total_lag`. The UI sorts by
  lag, shows per-topic lag subtotals, and color-codes the state.

**ACL handling.** If the broker's ACLs deny consumer-group access
(`GroupAuthorizationFailed`), the call returns a distinguishable **`403`** with a
clear message ("grant DescribeGroup …"), the result is **negative-cached** per
cluster so Otto stops re-probing (and stops spamming the broker with denials),
and the Groups tab shows a banner instead of an error. Topic browsing/peeking
still works because peek uses manual partition assignment (no group needed). The
cache is cleared on a cluster edit/delete so a re-grant re-probes.

### Offset reset

`POST /brokers/clusters/{id}/groups/{group}/reset` with a `GroupResetReq`
(Editor; guarded clusters require `confirm`). The body's `mode` is
`earliest` | `latest` | `offset` (with `offset`) | `timestamp` (with
`timestamp_ms`), optionally scoped to one `topic`. Otto fetches the group's
committed offsets to discover which partitions to reset, resolves each target
offset per the mode, and writes them back.

- **Dry-run:** add `?dry_run=true` to preview — it returns per-partition
  `current_offset` → `target_offset` and a `lag_delta`, plus
  `total_lag_before` / `total_lag_after`, **without committing anything**. A
  dry-run is accepted with `confirm=false` (nothing is written), while the real
  reset on a guarded cluster needs `confirm=true`. Resets write an audit row.

### Lag alerts (operator workflow)

The **Lag Alerts** tab manages per-cluster `{topic, group, threshold}` alerts
(`GET`/`POST /brokers/clusters/{id}/lag-alerts`,
`DELETE …/lag-alerts/{alert_id}`). A breach (`breach_lag`) is surfaced when the
most-recently-evaluated lag for that group+topic exceeds the threshold. Stored in
the broker-ops tables (migration `0055`).

---

## 8. Topic configs

- `GET /brokers/clusters/{id}/topics/{topic}/configs` → `TopicConfigEntry[]`
  (name, value, source, `is_default`, `is_sensitive`, `is_read_only`).
- `PUT …/topics/{topic}/configs` with an `AlterConfigsReq`
  (`{ configs: [{name, value}], confirm? }`, Editor; guarded clusters require
  `confirm`). The change **merges over the existing dynamic overrides** and
  returns the refreshed config list. Alters write an audit row.

Some clusters/principals can't read configs (e.g. MSK without
`DESCRIBE_CONFIGS`); in that case the cleanup-policy enrichment on the topics
table degrades gracefully (the column simply shows nothing) rather than failing
the whole listing.

---

## 9. Schema Registry

When a cluster profile has a **Schema registry URL**, the **Schema Registry** tab
talks to a Confluent-compatible registry (optionally with basic-auth username/
password, and over the SSH SOCKS tunnel for private registries):

- `GET …/schema-registry/subjects` → `SchemaSubject[]` (subject, latest version,
  id, type, schema text). Returns `400` if no registry is configured on the
  cluster.
- `GET …/schema-registry/subjects/{subject}/versions` → version history
  (`SchemaVersion[]`).
- `GET …/schema-registry/subjects/{subject}/versions/{version}` → a specific
  version's detail (`{version}` may be `latest`).
- `POST …/schema-registry/subjects/{subject}/compatibility` → check a candidate
  schema against the subject (`CompatCheckReq` → `CompatCheckResp`:
  `{ compatible, messages }`). This is a registry **read** endpoint, so Viewer is
  sufficient.

The registry client also powers automatic Avro decode in the peek view: it
fetches schemas by id and caches them (registries are append-only, so ids are
immutable).

---

## 10. Guards (production / read-only)

A cluster is **write-guarded** when `environment = "prod"` **or**
`read_only = true`. On a guarded cluster, every **mutation** — create/delete
topic, alter configs, produce, reset offsets, replay — is rejected with **`403`**
unless the request carries an explicit **`confirm`** (`confirm=true` in the body,
or `?confirm=true` on the query for `DELETE` topic). The UI sets `confirm`
automatically and shows the guard state (the `prod` env badge and a `read-only`
pill in the cluster header). The guard exactly mirrors the DB Explorer's
write-gate. Reads are never guarded.

Every successful write on any cluster (guarded or not) is recorded to the
`broker_write_audit` table (migration `0052`) with the cluster id, the
authenticated user id, the operation, and a small JSON detail. Replays
additionally persist per-message evidence rows (migration `0055`).

---

## 11. API / contract reference

Authoritative table: [`docs/contracts/api.md`](../contracts/api.md) →
*Message Brokers (Kafka viewer)*. Summary (all under `/api/v1`; reads = ws
viewer, mutations = ws editor, globals = root):

| Method & path | Auth | Body | Response |
|---|---|---|---|
| GET `/workspaces/{wid}/brokers/clusters` | viewer | — | `BrokerCluster[]` (workspace + global) |
| POST `/workspaces/{wid}/brokers/clusters` | editor | `UpsertClusterReq` | `BrokerCluster` (201) |
| GET `/brokers/clusters/{id}` | viewer | — | `BrokerCluster` |
| PATCH `/brokers/clusters/{id}` | editor | `UpsertClusterReq` (PATCH semantics) | `BrokerCluster` |
| DELETE `/brokers/clusters/{id}` | editor | — | 204 (deletes Keychain secrets too) |
| POST `/brokers/clusters/{id}/test` | editor | — | `TestClusterResp` (never 5xx; `ok:false` on failure) |
| GET `/brokers/clusters/{id}/overview` | viewer | — | `ClusterOverview` |
| GET `/brokers/clusters/{id}/metrics` | viewer | — | `ClusterMetrics` |
| GET `/brokers/clusters/{id}/topics` | viewer | — | `TopicSummary[]` (metadata-only) |
| POST `/brokers/clusters/{id}/topics` | editor | `CreateTopicReq` | `TopicSummary` (201; 409 if exists) |
| GET `/brokers/clusters/{id}/topics/{topic}` | viewer | — | `TopicDetail` |
| GET `/brokers/clusters/{id}/topics/{topic}/stats` | viewer | — | `TopicStats` (lazy count + cleanup policy + msg/s) |
| POST `/brokers/clusters/{id}/topics/stats` | viewer | `BatchStatsReq` (≤500) | `Record<string,TopicStats>` |
| DELETE `/brokers/clusters/{id}/topics/{topic}?confirm=B` | editor | — | 204 |
| GET `/brokers/clusters/{id}/topics/{topic}/configs` | viewer | — | `TopicConfigEntry[]` |
| PUT `/brokers/clusters/{id}/topics/{topic}/configs` | editor | `AlterConfigsReq` | `TopicConfigEntry[]` |
| POST `/brokers/clusters/{id}/topics/{topic}/consume` | viewer | `ConsumeReq` | `ConsumeResp` |
| POST `/brokers/clusters/{id}/topics/{topic}/produce` | editor | `ProduceReq` | `ProduceResp` |
| GET `/brokers/clusters/{id}/groups` | viewer | — | `GroupSummary[]` (403 on ACL denial) |
| GET `/brokers/clusters/{id}/groups/{group}` | viewer | — | `GroupDetail` |
| POST `/brokers/clusters/{id}/groups/{group}/reset[?dry_run=true]` | editor | `GroupResetReq` | `GroupDetail` (or `DryRunResp`) |
| POST `/brokers/clusters/{id}/replay` | editor | `ReplayReq` | `ReplayResp` (201) |
| GET `/brokers/clusters/{id}/schema-registry/subjects` | viewer | — | `SchemaSubject[]` (400 if none) |
| GET `…/subjects/{subject}/versions` | viewer | — | `SchemaVersion[]` |
| GET `…/subjects/{subject}/versions/{version}` | viewer | — | `SchemaVersionDetail` |
| POST `…/subjects/{subject}/compatibility` | viewer | `CompatCheckReq` | `CompatCheckResp` |
| GET / POST `/brokers/clusters/{id}/lag-alerts` | viewer / editor | `NewLagAlertReq` | `LagAlert[]` / `LagAlert` (201) |
| DELETE `/brokers/clusters/{id}/lag-alerts/{alert_id}` | editor | — | 204 |
| GET / POST `/workspaces/{wid}/brokers/cluster-sections` | viewer / editor | `UpsertSectionReq` | `BrokerClusterSection[]` / one (201) |
| PATCH `/brokers/cluster-sections/{id}` | editor | `UpsertSectionReq` (rename) | `BrokerClusterSection` |
| DELETE `/brokers/cluster-sections/{id}` | editor | — | 204 (descendants cascade; clusters → ungrouped) |
| POST `/brokers/cluster-sections/{id}/move` | editor | `MoveSectionReq` | `BrokerClusterSection` |

`UpsertClusterReq` (key fields): `name`, `bootstrap_servers`, `security_protocol`,
`sasl_mechanism`, `sasl_username`, `sasl_password` (write-only), `tls_skip_verify`,
`schema_registry_url`, `schema_registry_username`, `schema_registry_password`
(write-only), `metrics_url`, `color`, `ssh` (three-state), `section_id`
(three-state), `environment`, `read_only`.

`SshTunnelConfig`: `{ host, port = 22, user, identity_file? }` — key/agent auth
only.

---

## 12. Capabilities & limitations

**Supported:** PLAINTEXT/SSL/SASL_PLAINTEXT/SASL_SSL; SASL PLAIN, SCRAM-SHA-256,
SCRAM-SHA-512; cluster overview + health (under-replicated partitions, leadership
imbalance); topic browse/create/delete; lazy counts + msg/s; peek with multi-
format decode (incl. Confluent Avro) and key/value filters and live-tail;
produce (incl. headers, base64, tombstones); consumer-group lag, offset reset
(+ dry-run); topic config view/edit; Schema Registry browse + version history +
compatibility check; throughput + broker CPU/RAM; DLQ-style replay; lag alerts;
MSK/private clusters over an SSH bastion; multi-tab + sectioned sidebar.

**Not supported / known limits:**

- SASL is **username/password only** — no Kerberos/GSSAPI, no OAUTHBEARER, no
  AWS IAM (`AWS_MSK_IAM`) auth. For MSK, use TLS or **SASL/SCRAM**.
- SSH tunnel auth is **key/agent only** (no SSH password); the bastion `sshd`
  must allow TCP forwarding for the `-D` SOCKS forward.
- **On-disk topic size** is not exposed (the column shows `—`).
- Peek is capped at **5000 messages** per call (and 50 000 scanned raw records
  when a key filter is active).
- Batch topic-stats is capped at **500 names** per call.
- Consumer-group features require broker ACLs (`DescribeGroup` /
  `FindCoordinator`); browsing/peeking does not.
- Reading topic configs needs `DESCRIBE_CONFIGS`; without it, cleanup-policy
  enrichment is silently skipped.
- Over a tunnel, cluster-wide sweeps (throughput, batch counts) are slower; the
  throughput total is cached ~8 s between metrics polls.

---

## 13. Security

- **Secrets in the Keychain.** SASL and Schema Registry passwords live only in
  the macOS Keychain (`broker-{id}` / `broker-sr-{id}`). The DB stores opaque
  refs; the API returns only `has_*_password` booleans. Deleting a cluster
  deletes its Keychain secrets too.
- **No SSH secret.** SSH tunnels use key/agent auth via the system `ssh` client,
  so there is no SSH password to store. The tunnel honours your ssh-agent,
  `~/.ssh/config`, and `known_hosts`.
- **SSRF guard on the registry/metrics URLs.** User-supplied registry/metrics
  URLs are resolved + classified before connecting and redirect hops are
  re-validated, so a low-privileged caller can't steer the daemon at loopback /
  RFC1918 / `169.254.169.254`. This guard is **intentionally skipped when the
  request rides the SSH tunnel** — a private endpoint is the point and the
  bastion credentials are the authority.
- **TLS.** The proxy→broker TLS validates against bundled webpki roots
  (including Amazon's CA, so MSK validates by default); **Skip TLS verify**
  accepts any certificate chain/hostname (signatures still verified) for
  self-signed brokers.
- **Write-guards + audit.** Production/read-only mutations need explicit
  `confirm`; every write is audited (`broker_write_audit`), and replays record
  per-message evidence.
- **PII masking.** Peek supports `mask: true` to redact key/value/header text
  server-side before it leaves the daemon.
- **Loopback by default.** `ottod` listens on `127.0.0.1:7700` unless a network
  listener is explicitly enabled.

---

## 14. Troubleshooting

**Test fails / can't connect (no tunnel).**
- Check the **bootstrap servers** host/port and your network path to them.
- For SASL, confirm the **mechanism** matches the broker (PLAIN vs SCRAM-256 vs
  SCRAM-512) and the username/password are correct.
- For TLS against a self-signed broker, tick **Skip TLS verify**.
- Remember **Test never returns 5xx** — read the `message` in the toast; it is
  the underlying librdkafka error.

**Connects, but topics/metadata hang or partial (private cluster).**
This is the classic **advertised-listener** problem. Confirm the cluster has an
**SSH tunnel** configured. With the tunnel, advertised broker addresses are
rewritten automatically — without it, the client follows the brokers' private
advertised DNS names and stalls.

**SSH tunnel won't come up.**
- `ssh tunnel exited early` / auth errors → make sure `ssh user@host` works from
  a terminal first (the tunnel runs `BatchMode=yes`, so it won't prompt). Load
  the key into your agent or set the **Identity file**.
- `did not become ready within 12s` → the bastion may be unreachable, or its
  `sshd` disallows forwarding. The `-D` SOCKS forward needs
  `AllowTcpForwarding yes` on the bastion.

**SASL errors (`Authentication failed`, `SaslAuthenticationException`).**
- Wrong mechanism is the usual cause — MSK SCRAM is typically **SCRAM-SHA-512**
  on port `9096`.
- Re-enter the password (it's write-only; an empty field keeps the old one, a
  blank submit clears it).
- For MSK **IAM** auth: not supported — switch the cluster to SCRAM or TLS.

**TLS handshake fails over the tunnel.**
- For a self-signed/internal CA, tick **Skip TLS verify**. The proxy uses the
  broker hostname as SNI; a hostname mismatch surfaces as a handshake error
  unless verify is skipped.

**Consumer Groups tab shows an access banner / 403.**
The broker's ACLs deny group access. Grant `DescribeGroup` (and
`FindCoordinator`) to the principal. Otto caches the denial to avoid hammering
the broker; **edit the cluster** (any change) to clear the cache and re-probe
after a re-grant.

**Schema Registry tab is empty or 400.**
Configure a **Schema registry URL** on the cluster. A `400` means none is set; a
connection error usually means the URL/credentials are wrong or the registry is
unreachable (configure it on the tunnel side if it's private).

**Message counts show `…` / `—` / Msg/s is `—`.**
`…` = stats are loading; `—` = the count fetch failed (use **Retry counts**) or
the value isn't available. Msg/s needs **two** samples, so it's `—` on the first
poll.

---

## 15. Related docs

- [`./connections-ssh-sftp.md`](./connections-ssh-sftp.md) — the shared
  `otto-ssh` tunnel model (local-forward vs SOCKS), ssh-agent/`known_hosts`
  behavior, and SFTP over the same connection.
- [`./database-explorer.md`](./database-explorer.md) — the sibling feature whose
  write-gate, environment/read-only guards, and SSH-tunnel patterns this viewer
  mirrors (MongoDB/Atlas also use a SOCKS tunnel for the same advertised-host
  reason).
- [`../contracts/api.md`](../contracts/api.md) — authoritative HTTP contract.
- [`../../README.md`](../../README.md) — the full feature tour.
</content>
</invoke>
