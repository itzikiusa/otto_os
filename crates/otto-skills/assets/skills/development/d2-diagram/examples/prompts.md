# Example Prompts

## Repo architecture

Create D2 diagrams for this repo. Start with a system context diagram and a component map. Render to SVG if possible. Keep source in docs/diagrams.

## Login flow

Create a D2 sequence diagram for the login flow. Include Redis session creation, DB login history, token validation, and failure paths.

## ClickHouse dedup

Create a D2 dataflow diagram for the ClickHouse dedup pipeline. Show staging, dedup check, final table insert, materialized views, and where duplicates can leak.

## Cloudflare empty body debug

Create a D2 troubleshooting diagram for the intermittent 200 empty-body issue across Cloudflare Tunnel, Kubernetes Traefik, internal router, and service.

## ERD

Create a D2 ERD from these SQL migrations. Only include primary keys, foreign keys, important indexes, and high-value domain fields.
