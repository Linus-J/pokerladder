#!/usr/bin/env bash
# Pulls main, rebuilds, and restarts the server. Restarting abandons matches in
# progress (they go unrated), so run this when nobody is playing.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"
git pull --ff-only
cargo build --release -p pokerladder-server
(cd web && npm ci && npm run build)
systemctl --user restart pokerladder.service
systemctl --user --no-pager status pokerladder.service | head -5
