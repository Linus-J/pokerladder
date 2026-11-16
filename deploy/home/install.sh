#!/usr/bin/env bash
# One-time setup for running Pokerladder on this machine as user services.
# Safe to re-run: existing config files are never overwritten.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
config="$HOME/.config/pokerladder"
units="$HOME/.config/systemd/user"
mkdir -p "$config" "$units"
chmod 700 "$config"

env_file="$config/pokerladder.env"
if [[ ! -e "$env_file" ]]; then
    password="$(openssl rand -hex 24)"
    umask 077
    cat >"$env_file" <<EOF
# Pokerladder server settings (see .env.example in the repository).
DATABASE_URL=postgres://pokerladder:$password@127.0.0.1:55433/pokerladder
POSTGRES_PASSWORD=$password
BIND_ADDR=127.0.0.1:8080
PUBLIC_URL=https://pokerladder.app
# Every request arrives from cloudflared on localhost: rate limits need the real IP.
TRUSTED_PROXY_HEADER=CF-Connecting-IP
# Replace RESEND_API_KEY with a Resend API key (sending access, pokerladder.app).
SMTP_URL=smtps://resend:RESEND_API_KEY@smtp.resend.com:465
MAIL_FROM=Pokerladder <noreply@pokerladder.app>
STATIC_DIR=$repo/web/dist
RUST_LOG=info,sqlx=warn
EOF
    echo "wrote $env_file (generated a database password; add the Resend key)"
else
    echo "kept existing $env_file"
fi

tunnel_file="$config/tunnel.env"
if [[ ! -e "$tunnel_file" ]]; then
    umask 077
    echo "TUNNEL_TOKEN=PASTE_TOKEN_FROM_CLOUDFLARE" >"$tunnel_file"
    echo "wrote $tunnel_file (paste the tunnel token)"
else
    echo "kept existing $tunnel_file"
fi

for unit in "$repo"/deploy/home/systemd/*; do
    ln -sf "$unit" "$units/$(basename "$unit")"
done
chmod +x "$repo"/deploy/home/*.sh
systemctl --user daemon-reload
echo "linked units into $units; next steps are in deploy/home/README.md"
