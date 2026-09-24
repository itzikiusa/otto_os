---
id: brokers
title: Message Brokers
group: Infrastructure
route: brokers
summary: Browse Kafka clusters, including private ones behind an SSH bastion, and peek, produce, inspect lag and replay messages.
---

## What it's for

Message Brokers is Otto's Kafka viewer. Connect a cluster, including a
private one such as AWS MSK reached through an SSH bastion. Then you can:

- browse topics and partitions
- peek at messages and produce new ones
- check consumer-group lag and reset offsets
- edit topic configs and read a Schema Registry
- replay messages from a dead-letter topic
- watch cluster health and throughput

Kafka clusters live in the same tree as your other connections. Click a Kafka
row in [Connections](#/walkthroughs/connections) to open it as a tab there.
To use the full-page viewer, right-click the row and choose **Open in Message
Brokers page**. Message Brokers has no sidebar entry of its own.

## Getting started

1. In **Connections**, click **New Kafka cluster**, the button next to **+**
   above the tree. On the Message Brokers page, use **Add cluster** (**+**) or
   **Add a cluster**.
2. Enter a **Name** and **Bootstrap servers**, for example
   `broker1:9092,broker2:9092`. A missing port defaults to 9092.
3. Pick **Security**: PLAINTEXT, SSL, SASL_PLAINTEXT or SASL_SSL. For SASL,
   choose PLAIN, SCRAM-SHA-256 or SCRAM-SHA-512 and enter the username and
   password.
4. Optional: add a **Schema registry URL** (for schemas and Avro decoding) and
   a **Metrics URL** (a Prometheus endpoint for broker CPU and RAM).
5. For a private cluster, tick **SSH tunnel** and fill in the tunnel host,
   port, user and identity file.
6. Set the **Environment** (Dev, Staging or Production), and optionally an
   **Accent color** and **Read-only**. Save.
7. Open the cluster and click **Test** in its header. The **Overview** tab
   loads first.

## Everything it can do

### Clusters and navigation

- **Tabs.** Open several clusters at once. Each one is a tab, marked with its
  accent-colored dot.
- **Six views per cluster:** Overview, Topics, Consumer Groups, Schema
  Registry, Replay and Lag Alerts.
- **Header badges** show the environment, a `read-only` pill, and a **Tunnel**
  pill for tunnelled clusters. The Tunnel pill reads "Connecting…" while the
  tunnel warms up and turns green when it's ready. Selecting a tunnelled
  cluster starts warming its tunnel in the background.
- **Sections.** Clusters share the Connections folder tree:
  - **New section**, and on a folder's right-click menu **New sub-section**,
    **Rename…** and **Delete**.
  - Drag clusters onto folders and folders onto folders. Drop on
    **Ungrouped** to move something back to the top level.
- **Right-click a cluster** (Message Brokers page) for **Open in tab**,
  **Test**, **Edit…** and **Remove**.
- **Resizable lists.** Drag the dividers of the cluster list and the
  consumer-group list to resize them. Double-click a divider to reset it.

### Overview

- Cards for brokers, topics (with the internal-topic count), partitions,
  consumer groups and throughput (messages per second, plus the total).
- **Under-replicated** partitions and **Leader skew** (how unevenly partition
  leaders are spread) are highlighted when they need attention.
- The cluster ID.
- A live throughput chart that refreshes about every 4 seconds.
- A card for each broker with its ID, host and number of partition leaders.
  CPU and RAM bars appear when a Metrics URL is set. Redpanda, JVM and
  `process_*` metrics are understood.

### Topics

- **Search topics…**, **Show internal** (internal topics are hidden by
  default) and a cleanup-policy filter.
- Columns: Partitions, RF (replication factor), Count and **Msg/s**. Counts
  load for the visible page only. Msg/s updates every 5 seconds. Size always
  shows "—", because this Kafka client can't report on-disk size.
- 50 topics per page. A retry button appears if some message counts fail to
  load.
- **New topic**: enter a name, the number of partitions and the replication
  factor.
- Click a topic to open its detail views: **Messages**, **Partitions**,
  **Config** and **Produce**.
- **Delete topic** asks you to type the topic name first.

### Peek at messages

- **Start from:** Latest, From beginning, From offset or From time. You can
  read one partition or all of them.
- **Limit:** 1 to 5,000 messages.
- **Decode as:** Auto, JSON, UTF-8, Protobuf, Avro, Hex or Base64. Auto
  decodes Confluent-framed Avro when a Schema Registry is set.
- **Filter key…** runs on the server. Tick **From start** to search older
  messages. **Filter value…** matches the decoded value.
- **Live · 1m** adds new messages every minute and keeps the latest 500.
- **Mask** makes the server hide sensitive values (emails, tokens and keys)
  before the messages leave the daemon.
- Each row shows its partition, offset, a position bar (how far through the
  partition it sits), key, time and size.
- **Message detail** shows the key, value, headers and schema ID. Switch
  between **Raw** and **Decoded**, **Copy** the message as JSON, or use **To
  agent** to send it to a running agent with a redacted preview first.
- **Export** the results as **JSON** or **CSV**.
- Peeking never commits offsets or affects your real consumers.

### Produce, partitions and config

- **Produce:** a key, a value, a partition (or Auto), and headers (**+ Add**).
  You can also send a **Tombstone** (null value) or mark the key or value as
  **Base64**.
- **Partitions:** each partition's leader, replicas, in-sync replicas, low and
  high offsets, and message count.
- **Config:** every setting with its source. To change one, enter its name
  (for example `retention.ms`) and a value, then click **Set**.

### Consumer groups

- The group list shows each group's state and member count.
- Group detail shows:
  - its members, with client, host and assignments
  - committed offsets, end offsets and lag per partition (tick **Sort by
    lag**)
  - total lag per topic
- **Reset offsets:** choose Earliest, Latest, Specific offset or From
  timestamp, for all topics or just one. **Preview** shows the current and
  target offsets and the lag change without committing anything. **Reset**
  asks you to type the group name first.
- If the broker's access rules deny group access, a banner explains that the
  `DescribeGroup` permission is needed. Otto doesn't keep retrying.

### Schema Registry

- A list of subjects with their latest version, type and ID. Open one to see
  its schema.
- **Versions & Compat:** see each version, compare two versions side by side,
  and check a candidate schema for compatibility with the latest version.

### Replay and lag alerts

- **DLQ / Replay** re-publishes messages from a source topic (such as a
  dead-letter topic) to a target topic. Choose the last N messages, an offset
  range on one partition, or everything since a time.
- A replay can optionally change the key or add a header. Otto saves a record
  of every replayed message, showing where it came from and where it landed.
- **Lag Alerts** saves thresholds for topic and consumer-group pairs. Use
  **Add alert**, or delete one from its row.

### For agents

Agents can use Otto's MCP tools to list clusters, topics and consumer groups,
read a topic and peek at messages. Producing a message is approval-gated.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| ↵ | Open the focused cluster tab or cluster row |

See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts) for the global
keys.

## Tips and limits

- **Production and read-only guard.** A cluster is guarded when its
  environment is Production or it's marked read-only:
  - Producing and changing a config ask for confirmation. Replays always ask,
    guarded or not.
  - Deleting a topic and resetting offsets always ask you to type the name.
  - Creating a topic on a guarded cluster goes through without an extra
    prompt, so double-check the name.
  - Reading is never blocked. Every write is recorded in an audit log.
- **Supported sign-in.** SASL supports username and password only. Kerberos,
  OAUTHBEARER and AWS IAM aren't supported. For MSK, use TLS or SASL/SCRAM.
- **SSH tunnels** use your ssh-agent or key file. There's no SSH password.
  The bastion must allow TCP forwarding (`AllowTcpForwarding yes`).
- **Why tunnels just work.** Otto runs one SOCKS tunnel plus a Kafka-aware
  proxy that rewrites broker addresses. Every broker in a private cluster is
  reachable, and so are a private Schema Registry and Metrics URL.
- **Skip TLS certificate verification** is for self-signed brokers only.
- **Secrets.** SASL and Schema Registry passwords are stored in the macOS
  Keychain. When you edit a cluster, leave a password blank to keep it.
  Deleting a cluster removes its secrets.
- **Limits.** Peeking returns at most 5,000 messages. A key-filtered scan
  checks at most 50,000 records.
- **Missing permissions.** Consumer-group views need `DescribeGroup` access
  on the broker. Cleanup-policy details need `DESCRIBE_CONFIGS`; without it,
  that column stays empty.
- **Lag alerts** are saved thresholds. This build lists them as Active but
  doesn't check them against live lag or notify you. Watch **Consumer Groups**
  for real lag.
- **Access.** Clusters are shared across workspaces. Viewing needs the
  Database feature, and changes need Database edit access.

## Related

- [Connections](#/walkthroughs/connections)
- [Database Explorer](#/walkthroughs/database)
- [Agents](#/walkthroughs/agents)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
