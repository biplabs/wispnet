#!/bin/sh
# Run with sudo as the owner of the existing Wispnet user services.
set -eu
[ "$(id -u)" = 0 ] || { echo 'Run this script with sudo.' >&2; exit 1; }
operator=${SUDO_USER:?Run using sudo from the existing Wispnet account}
[ "$operator" != root ] || exit 1
operator_home=$(getent passwd "$operator" | cut -d: -f6)
operator_uid=$(id -u "$operator")
operator_group=$(id -gn "$operator")
case "$operator_home" in /*) ;; *) exit 1 ;; esac
bundle=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
config="$operator_home/.config/wispnet-public/Caddyfile"
unit=wispnet-public-https.service
system_unit="/etc/systemd/system/$unit"
[ ! -e "$system_unit" ] || { echo "$system_unit already exists; refusing to overwrite it." >&2; exit 1; }
userctl() {
    runuser -u "$operator" -- env XDG_RUNTIME_DIR="/run/user/$operator_uid" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$operator_uid/bus" systemctl --user "$@"
}
as_owner() {
    runuser -u "$operator" -- env HOME="$operator_home" XDG_DATA_HOME="$operator_home/.local/share/wispnet-public-https" XDG_CONFIG_HOME="$operator_home/.local/share/wispnet-public-https/config" "$@"
}
userctl is-active --quiet "$unit"
backup=$(mktemp -d /var/backups/wispnet-caddy-443.XXXXXX)
chmod 700 "$backup"
cp -p "$config" "$backup/Caddyfile.old"
install -m 644 "$bundle/Caddyfile" "$backup/Caddyfile.new"
# Validate as the actual service account, using the existing certificate storage.
check_config=$(mktemp "$operator_home/.config/wispnet-public/Caddyfile.443-check.XXXXXX")
trap 'rm -f "$check_config"' EXIT
install -o "$operator" -g "$operator_group" -m 600 "$backup/Caddyfile.new" "$check_config"
as_owner /usr/bin/caddy validate --config "$check_config" --adapter caddyfile
rm -f "$check_config"
trap - EXIT
cat > "$backup/$unit" <<EOF
[Unit]
Description=Wispnet public HTTPS and packet relay front end
Wants=network-online.target
After=network-online.target
[Service]
User=$operator
Group=$operator_group
Environment=HOME=$operator_home
Environment=XDG_DATA_HOME=$operator_home/.local/share/wispnet-public-https
Environment=XDG_CONFIG_HOME=$operator_home/.local/share/wispnet-public-https/config
ExecStart=/usr/bin/caddy run --config $config --adapter caddyfile
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
Restart=on-failure
RestartSec=3
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=$operator_home/.local/share/wispnet-public-https
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
UMask=0077
MemoryMax=256M
TasksMax=128
LimitNOFILE=4096
[Install]
WantedBy=multi-user.target
EOF
systemd-analyze verify "$backup/$unit"
rollback() {
    trap - EXIT HUP INT TERM
    systemctl disable --now "$unit" || true
    rm -f "$system_unit"
    cp -p "$backup/Caddyfile.old" "$config"
    systemctl daemon-reload
    userctl enable --now "$unit"
    echo "Activation failed; restored the original user service. Backup: $backup" >&2
}
trap rollback EXIT
trap 'exit 1' HUP INT TERM
userctl disable --now "$unit"
install -o "$operator" -g "$operator_group" -m 600 "$backup/Caddyfile.new" "$config"
install -m 644 "$backup/$unit" "$system_unit"
systemctl daemon-reload
systemctl enable --now "$unit"
healthy=false
for attempt in $(seq 1 20); do
    if curl --noproxy '*' --fail --silent --show-error --max-time 3 --resolve relay.wispnet.app:443:127.0.0.1 https://relay.wispnet.app/ping >/dev/null 2>&1 &&
       curl --noproxy '*' --fail --silent --show-error --max-time 3 --resolve relay.wispnet.app:8443:127.0.0.1 https://relay.wispnet.app:8443/ping >/dev/null 2>&1 &&
       curl --noproxy '*' --fail --silent --show-error --max-time 3 --resolve 'relay.wispnet.app:443:[::1]' https://relay.wispnet.app/ping >/dev/null 2>&1; then
        healthy=true
        break
    fi
    sleep 1
done
[ "$healthy" = true ]
trap - EXIT HUP INT TERM
# Keep the old 8443 firewall allowance during the router transition.
if command -v ufw >/dev/null 2>&1 && ufw status | grep -q '^Status: active'; then
    ufw allow 443/tcp
fi
echo "Caddy verified on IPv4 443/8443 and IPv6 443. Backup: $backup"
echo 'The router may now change TCP 443 -> 8443 to TCP 443 -> 443.'
echo 'Caddy is now managed with sudo systemctl (not systemctl --user).'
