#!/usr/bin/env bash
# Autonomous DB-Explorer E2E runner: waits for the Docker daemon, brings up the
# seeded dev stack, then runs all four engines' #[ignore] e2e suites.
# Idempotent + re-runnable. Exit 7 = daemon never appeared (relaunch later).
set -uo pipefail
cd /Users/itziklavon/otto_os
COMPOSE=(docker compose -f dev/dbviewer/docker-compose.yml)

echo "[watcher] nudging Docker Desktop (best-effort)..."
open -a Docker >/dev/null 2>&1 || true
echo "[watcher] waiting for docker daemon (up to ~8m)..."
UP=0
for i in $(seq 1 100); do
  if docker ps >/dev/null 2>&1; then UP=1; echo "[watcher] daemon UP after ~$((i*5))s"; break; fi
  sleep 5
done
[ "$UP" -ne 1 ] && { echo "[watcher] RESULT=DAEMON_DOWN"; exit 7; }

echo "[watcher] bringing up stack..."
"${COMPOSE[@]}" up -d 2>&1 | tail -6

echo "[watcher] waiting for healthchecks..."
for _ in $(seq 1 36); do
  healthy=$(docker ps --filter name=otto-dbv --format '{{.Status}}' | grep -c healthy)
  [ "${healthy:-0}" -ge 4 ] && break
  sleep 5
done

# ClickHouse exits if its seed ever failed; recreate it if it isn't running.
if ! docker ps --filter name=otto-dbv-clickhouse --filter status=running -q | grep -q .; then
  echo "[watcher] recreating clickhouse..."
  docker rm -f otto-dbv-clickhouse >/dev/null 2>&1
  "${COMPOSE[@]}" up -d clickhouse 2>&1 | tail -3
  sleep 20
fi
docker ps --filter name=otto-dbv --format '{{.Names}}  {{.Status}}'

echo "[watcher] running E2E (OTTO_DBV_E2E=1)..."
OTTO_DBV_E2E=1 cargo test -p otto-dbviewer -- --ignored --nocapture 2>&1 | tail -120
echo "[watcher] RESULT=E2E_DONE"
