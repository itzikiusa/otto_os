# Diagram Selection Guide

Use this guide to choose the right D2 diagram for the task.

## Quick matrix

| User asks for | Use | Output |
|---|---|---|
| “How is this system structured?” | Architecture / C4-style | Containers, services, stores, external systems |
| “What happens when X?” | Sequence | Ordered participant interactions |
| “How does data move?” | Data flow | Producers, processors, stores, consumers |
| “What tables/entities exist?” | ERD | Entities/tables and relationships |
| “How is it deployed?” | Infrastructure | Edge, cluster, services, pods, storage |
| “Why is this failing?” | Troubleshooting | Request path, evidence, hypotheses, probes |
| “What depends on what?” | Dependency graph | Modules/packages/imports/build edges |
| “What states can it be in?” | State machine | States, transitions, triggers |
| “Show everything” | Multi-diagram set | Context + sequence + data/deployment |

## Prefer multiple focused diagrams when

- More than 12–15 nodes are needed.
- Both runtime flow and deployment topology are requested.
- Data model and request flow are both important.
- User asks for onboarding documentation.

## Default diagram set for repo onboarding

1. `system-context.d2` — What the service is and what it talks to.
2. `component-map.d2` — Main modules/packages/components.
3. `primary-request-sequence.d2` — Most important request flow.
4. `data-flow.d2` — DB/cache/queue/event movement.
5. `deployment-topology.d2` — Runtime infra, if manifests exist.
