# Application telemetry and profiling

Otto usage measures the app itself. The existing Usage Overview and Report still
show coding-agent tokens, model usage and cost.

## Enable local collection

1. Sign in as the root administrator and open **Usage → Otto usage**.
2. Select **Collect application telemetry on this Mac**, then save.
3. If ClickHouse is unavailable, install it from Usage → Overview first. Otto
   downloads and verifies its pinned OpenTelemetry Collector automatically.
4. Wait for **Collecting locally**, then navigate or run an operation. Refresh
   the dashboard after several seconds for the first batch.

Collection defaults off. Measurements go from the authenticated UI and daemon to
a collector listening on loopback, then to the existing embedded ClickHouse.
They are not sent to an external telemetry service. Prompts, SQL text, file paths,
request bodies, credentials and arbitrary application log messages are excluded.
Disabling stops observers, drops queued data, terminates the managed collector
and releases the ClickHouse keep-awake lease. Previously stored history expires
under its retention policy. Existing agent usage settings are independent.

## Read the dashboard

- **Application health:** CPU and resident memory for the daemon, collector and
  ClickHouse. Installed macOS builds also identify their own desktop/WebKit
  processes; development/test daemons do not attribute the running installed app.
  Missing samples mean unavailable. CPU can exceed 100% across multiple cores.
- **Slow operations:** scheduled suggestions, with supporting counts, latency
  percentiles and a trace. Resource alerts are a separate category. Dismissals
  are retained across analysis runs.
- **Measured operations:** counts, errors, p50, p95 and maximum wall duration.
  The table is paginated to keep dashboard rendering bounded. Trace details show
  the browser/server parent relationships and nested operation durations.
- **Native profiling:** with the additional checkbox enabled, capture a short
  macOS sample of the daemon's stacks. Captures have a cooldown, discard paths
  and addresses, and retain the latest sanitized result. This is sampled stack
  evidence, not a claim that slow wall time consumed equivalent CPU time.

UI instrumentation covers module navigation through first paint, lazy chunk
imports, background request queue wait, HTTP response-header arrival, response
read/JSON decoding, supported browser long tasks, and a bounded two-frame responsiveness sample every five seconds. Hidden-window samples are discarded. Server instrumentation
covers matched HTTP routes, database driver/access phases, Git subprocesses,
History lookups and the shared blocking-work boundary. Streaming response
lifetimes and every individual function are not automatically traced. Native
profiles help investigate work below these explicit boundaries. Long-task and
heap APIs differ by webview; unsupported metrics are not invented.

## Retention and daily analysis

Defaults: traces **1 day**, logs **2 days**, metrics **7 days**. Retention settings
apply to raw tables and their minute rollups; queries use bounded time windows.
ClickHouse applies physical expiry asynchronously during merges.

Analysis runs every **24 hours** by default; change the interval and analysis
window in settings, or use **Analyze now**. An operation qualifies only when its
p95 exceeds the configured per-call threshold and it has enough samples. The
remaining candidates are ranked by cumulative latency. A million calls below
0.1 ms do not qualify merely because their total is large. Up to 100 latency
suggestions are produced, plus separately typed resource-spike alerts.
CPU alerts require two consecutive samples above the configured threshold.
Memory alerts detect an increase between samples, rather than treating a stable
large process as a spike. Alerts for the same process have a one-minute cooldown.

Minute materialized views store mergeable quantile states and count/sum/max
states. Dashboard and analysis queries merge those states; they do not average
percentiles or scan all raw traces. Raw traces remain available for bounded
trace-ID drill-down. Suggested actions are evidence for investigation; they do
not change code, start agents, or publish messages automatically.

## Troubleshooting

**Collector starting or unavailable:** check ClickHouse in Usage → Overview.
A first enable needs GitHub access for the collector download. Otto retries a
failed local pipeline with backoff and exposes its state; ordinary app work does
not wait for telemetry.

**Discarded or unacknowledged records:** local queues and retry budgets are deliberately bounded.
The accepted counter acknowledges the local collector, not durable storage. If storage cannot keep up, some measurements are dropped rather than allowing
memory to grow without limit. Investigate collector/ClickHouse CPU and disk space.

**Empty trace:** raw retention may have expired, or its batch has not arrived.
Aggregates cannot reconstruct individual expired traces.

**No suggestions:** enough qualifying measurements may not exist yet. Check the
selected window, threshold and minimum call count. Fast operations are excluded
on purpose.

**Profiles unavailable:** native sampling currently requires macOS and the
separate profiling checkbox. It samples the daemon, not arbitrary processes or
agent command contents. The profiles use Otto's sanitized sampled-stack format,
not the experimental OTLP profiles signal.

## Add an operation boundary

Rust request handlers inherit a task-local trace context. Wrap a meaningful
asynchronous phase with `otto_telemetry::context::measure_result("db.pool.acquire", future)`
when its result indicates success/failure, or `measure` for opaque work. Names
must be static and contain no identifiers or inputs. Outside a traced request,
these wrappers simply run the work. New spawned tasks need an explicit context
scope; task-local context does not automatically cross `tokio::spawn` boundaries.

UI hooks live in `ui/src/lib/telemetry.ts`; their bounded collection runtime is
loaded only after opt-in. Keep operation names and components on the ingestion
allowlist in `crates/otto-server/src/routes/telemetry.rs`. Do not attach a URL,
query, prompt, exception message, or arbitrary component props. The records use
standard OTLP/HTTP JSON and W3C trace context with the upstream
[ClickHouse exporter](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/exporter/clickhouseexporter).
The log signal contains structured operation errors, resource spikes and profile
metadata; it does not copy arbitrary daemon or agent log text.

See [API contract](../contracts/api.md#application-telemetry-and-profiling) and
[verification and load tests](../testing/telemetry.md).
