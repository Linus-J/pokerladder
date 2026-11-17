# Running Pokerladder on one machine (home server)

The game server, Postgres (in Docker) and a Cloudflare Tunnel run as **systemd user
services** for the account that owns the checkout. Nothing listens on the network:
the server binds `127.0.0.1:8080`, Postgres `127.0.0.1:55433`, and `cloudflared`
makes an outbound connection to Cloudflare, which serves `https://pokerladder.app`.

Suitable for a private beta. `systemctl --user stop/restart pokerladder` (and so
`update.sh`) is graceful: no new matches start, players see a notice, and matches
in progress play to the end first (up to `DRAIN_TIMEOUT_SECS`, default 45 minutes).
A reboot or shutdown of the PC does **not** wait that long (systemd kills user
services after about two minutes), so before rebooting run
`systemctl --user stop pokerladder` and wait for it to return. A crash, power cut or
forced kill abandons matches in progress, unrated.
Before seasons with prizes, move to a VPS (same setup with Caddy instead of the
tunnel).

## One-time setup

1. Packages (needs sudo):
   ```sh
   sudo pacman -S --needed docker docker-compose cloudflared
   sudo systemctl enable --now docker
   sudo usermod -aG docker "$USER"     # then log out and back in (or reboot)
   loginctl enable-linger "$USER"      # user services run without a login session
   ```
   Membership of the `docker` group is equivalent to root on this machine.
2. Cloudflare Zero Trust → Networks → Tunnels → **Create a tunnel** (cloudflared).
   Name it `pokerladder`, skip the install step, and add a **public hostname**:
   `pokerladder.app` → service `HTTP` → `localhost:8080`. Copy the tunnel token.
3. Resend → API Keys → create a key with **sending access** for `pokerladder.app`.
4. `deploy/home/install.sh` writes `~/.config/pokerladder/pokerladder.env` (with a
   generated database password) and `tunnel.env`, and links the units. Then edit:
   - `pokerladder.env`: replace `RESEND_API_KEY` in `SMTP_URL`.
   - `tunnel.env`: set `TUNNEL_TOKEN`.
5. Build (if not done) and start:
   ```sh
   cargo build --release -p pokerladder-server
   (cd web && npm ci && npm run build)
   systemctl --user enable --now pokerladder-db pokerladder pokerladder-tunnel pokerladder-backup.timer
   ```

## Day to day

| Task | Command |
|---|---|
| Logs | `journalctl --user -u pokerladder -f` |
| Status | `systemctl --user status pokerladder pokerladder-db pokerladder-tunnel` |
| Deploy `main` | `deploy/home/update.sh` (waits for matches in progress) |
| Stop now, abandoning matches | `systemctl --user kill -s INT pokerladder` while it is draining |
| Backup now | `systemctl --user start pokerladder-backup` |
| psql | `docker exec -it pokerladder-prod-db-1 psql -U pokerladder` |
| Wipe ranks and history (keeps accounts) | `deploy/home/reset-ladder.sh` (backs up first; refuses mid-match) |

Backups go to `~/backups/pokerladder` (newest 14). Copy them off this machine too,
e.g. with `rclone` to cloud storage; a backup on the same disk doesn't survive the
disk. Restore into an empty database with
`docker exec -i pokerladder-prod-db-1 pg_restore -U pokerladder -d pokerladder --clean < FILE`.

## Settings that matter

- `TRUSTED_PROXY_HEADER=CF-Connecting-IP`: every request reaches the server from
  `cloudflared` on localhost. Without it, all players share one rate-limit bucket.
- `BIND_ADDR` must stay on `127.0.0.1`, or anyone on the network could connect
  directly and forge that header.
- `PUBLIC_URL=https://pokerladder.app`: the server refuses to start without
  `SMTP_URL` when this is https.
- Disable sleep/suspend on this machine while it hosts games.
