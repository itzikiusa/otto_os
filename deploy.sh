#!/bin/bash
#
# deploy.sh — compatibility entrypoint. There is ONE deploy script:
# packaging/deploy.sh (rebuild → sign → staged install with an atomic swap →
# relaunch → verify, with a deploy lock, automatic rollback to the previous app
# when verification fails, and a detached launchd finish phase that survives
# the daemon restart). This wrapper only forwards, so muscle memory and older
# docs keep working. Flags: --yes, --dmg, --force-ci, --status, -h.
# The old --status lived here as ~/Library/Logs/Otto/deploy-last.status; the
# outcome is now `packaging/deploy.sh --status` (DEPLOY-RECEIPT line).
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/packaging/deploy.sh" "$@"
