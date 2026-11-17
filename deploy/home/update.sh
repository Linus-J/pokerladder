#!/usr/bin/env bash
# Pulls main, rebuilds, and restarts the server. The restart waits for matches in
# progress to finish (no new ones start meanwhile; players see a notice), so it can
# take up to DRAIN_TIMEOUT_SECS (default 45 min) if a match is running.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"
git pull --ff-only
cargo build --release -p pokerladder-server
(cd web && npm ci && npm run build)
echo "restarting: waiting for any matches in progress to finish..."
systemctl --user restart pokerladder.service
systemctl --user --no-pager status pokerladder.service | head -5
