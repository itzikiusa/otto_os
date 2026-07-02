# Repo Discovery Heuristics

Use these heuristics when deriving D2 diagrams from source code.

## Go services

Inspect:

- `cmd/**/main.go`
- `main.go`
- `internal/**`
- `pkg/**`
- router setup: Fiber, Gin, Gorilla Mux, chi, net/http
- clients: Redis, Kafka, ClickHouse, MySQL, Mongo, S3/R2, SQS
- config structs and env loading
- test/integration folders

Look for:

- Routes and handlers
- Middleware chain
- Repository/store interfaces
- Producers/consumers
- Cron/workers
- Outbox patterns
- Dependency injection/wiring

## Java Spring Boot / WebFlux

Inspect:

- `Application.java`
- `@RestController`, `@Controller`, `RouterFunction`
- `@Service`, `@Repository`, `@Component`
- WebClient/Feign clients
- Kafka/SQS/ActiveMQ listeners
- JPA entities and migrations
- `application.yml`, `bootstrap.yml`

Look for:

- API endpoints
- Reactive flows and external calls
- DB repositories
- Resilience4j/circuit breaker boundaries
- Scheduler/Quartz jobs

## Angular / frontend

Inspect:

- `angular.json`
- routes/modules/components
- API services
- auth guards/interceptors
- state management/store

Look for:

- User journeys
- API calls
- screens and route transitions

## Infra

Inspect:

- `docker-compose.yml`
- `Dockerfile`
- Kubernetes manifests
- Helm charts
- Terraform
- ArgoCD Application manifests
- Jenkins pipelines
- Cloudflare Worker/Tunnel config if present

Look for:

- Runtime containers
- Ingress/service routing
- environment dependencies
- secrets references, but never copy secret values
- external networks/providers

## Database

Inspect:

- SQL migrations
- ClickHouse DDL
- JPA entities
- Mongo collection access code
- repository interfaces

Look for:

- Primary write owner
- Read-heavy paths
- Aggregations/materialized views
- Dedup/idempotency keys
- foreign-key or conceptual relations

## Eventing

Inspect:

- Kafka topic constants/config
- consumers/producers
- retry/DLQ logic
- outbox/inbox tables
- protobuf/Avro/JSON schemas

Look for:

- event names
- producers and consumers
- retry path
- idempotency keys
- retention assumptions
