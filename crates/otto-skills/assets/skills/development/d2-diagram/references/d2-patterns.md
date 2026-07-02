# D2 Patterns

These are safe patterns for common diagrams. Adapt labels and IDs to the project.

## Basic architecture

```d2
direction: right

user: "Player/Admin"

frontend: {
  label: "Frontend"
  web: "Web App"
}

backend: {
  label: "Backend"
  api: "API Service"
  worker: "Worker"
}

data: {
  label: "Data"
  db: "MySQL"
  cache: "Redis"
}

external: {
  label: "External"
  style.stroke-dash: 4
  provider: "Payment Provider"
}

user -> frontend.web: uses
frontend.web -> backend.api: HTTPS API
backend.api -> data.db: reads/writes
backend.api -> data.cache: session/cache
backend.api -> external.provider: payment request
backend.worker -> data.db: async processing
```

## Sequence diagram

```d2
login_flow: {
  shape: sequence_diagram

  player: "Player"
  web: "Web App"
  api: "Admission API"
  redis: "Redis"
  mysql: "MySQL"

  player -> web: submit credentials
  web -> api: POST /login
  api -> mysql: validate player credentials
  api -> redis: create session token
  api -> mysql: write login_history
  api -> web: return JWT/session
  web -> player: redirect to lobby

  failure: {
    api -> mysql: invalid credentials
    api -> web: 401 Unauthorized
  }
}
```

## ERD / SQL table sketch

```d2
users: {
  shape: sql_table
  id: int {constraint: primary_key}
  email: varchar
  created_at: timestamp
}

sessions: {
  shape: sql_table
  id: int {constraint: primary_key}
  user_id: int {constraint: foreign_key}
  token_hash: varchar
  expires_at: timestamp
}

users.id -> sessions.user_id: "1:N"
```

## Data pipeline

```d2
direction: right

producer: "Game/Payment Service"
kafka: "Kafka Topic\ntransactions"
dedup: "Dedup Processor"
clickhouse: "ClickHouse\nraw_hourly_transactions"
materialized_views: "Materialized Views"
grafana: "Grafana"

producer -> kafka: publishes transaction event
kafka -> dedup: consumes batch/stream
dedup -> clickhouse: inserts unique rows
clickhouse -> materialized_views: triggers aggregation
materialized_views -> grafana: queried by dashboards
```

## Troubleshooting chain

```d2
direction: right

client: "Client"
cf: "Cloudflare"
tunnel: "Cloudflare Tunnel"
ingress: "K8s Traefik"
router: "Internal Router"
service: "Service"

client -> cf: request
cf -> tunnel: proxied request
tunnel -> ingress: forwards
ingress -> router: routes
router -> service: upstream call
service -> router: response body?
router -> ingress: maybe empty body

evidence: |md
  # Evidence
  - Intermittent 200 with empty body
  - Frequency: rare
  - Need correlation IDs at every hop
|
```
