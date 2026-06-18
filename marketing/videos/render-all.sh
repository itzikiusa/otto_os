#!/bin/bash
set -e
for id in Intro Settings Shortcuts AgentMode Connections GitPr Product; do
  echo "=== rendering $id ==="
  npx remotion render src/index.ts "$id" "out/$id.mp4" --log=error --jpeg-quality=90 || echo "FAILED $id"
done
echo "=== done ==="; ls -lh out/
