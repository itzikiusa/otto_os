#!/bin/bash
# Render every Otto walkthrough → out/<Id>.mp4. (Or: node render-all.mjs)
set -e
IDS="Intro Sessions Git Review Product Connections Database Brokers Swarm Channels UsageInsights Skills Workflows Plugins Vault TeamMobile Platform Outro"
mkdir -p out
for id in $IDS; do
  echo "=== rendering $id ==="
  npx remotion render src/index.ts "$id" "out/$id.mp4" --log=error --jpeg-quality=92 || echo "FAILED $id"
done
echo "=== done ==="; ls -lh out/
