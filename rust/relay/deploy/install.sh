#!/usr/bin/env bash
# Install a locally built binary. No compiler, source tree, or container required.
set -euo pipefail
binary_file="${1:?Usage: install.sh BINARY SERVER_CONFIG SERVICE_UNIT}"
config_file="${2:?Usage: install.sh BINARY SERVER_CONFIG SERVICE_UNIT}"
service_unit="${3:?Usage: install.sh BINARY SERVER_CONFIG SERVICE_UNIT}"
test "$(id -u)" = 0
test -f "$binary_file"
getent group antify-relay >/dev/null || groupadd --system antify-relay
id antify-relay >/dev/null 2>&1 || useradd --system --gid antify-relay --home-dir /var/lib/antify-relay --shell /sbin/nologin antify-relay
install -d -m 0755 /opt/antify-relay/bin
install -d -m 0750 -o root -g antify-relay /etc/antify-relay
install -d -m 0700 -o antify-relay -g antify-relay /var/lib/antify-relay
if test ! -e /etc/antify-relay/server.json; then
    install -m 0640 -o root -g antify-relay "$config_file" /etc/antify-relay/server.json
fi
if test -f /opt/antify-relay/bin/antify-relay; then
    cp -p /opt/antify-relay/bin/antify-relay /opt/antify-relay/bin/antify-relay.previous
fi
install -m 0755 "$binary_file" /opt/antify-relay/bin/antify-relay.next
mv /opt/antify-relay/bin/antify-relay.next /opt/antify-relay/bin/antify-relay
install -m 0644 "$service_unit" /etc/systemd/system/antify-relay.service
systemctl daemon-reload
systemctl enable antify-relay
systemctl restart antify-relay
for attempt in {1..15}; do
    if curl --fail --silent http://127.0.0.1:53319/health; then
        printf '\n'
        systemctl is-active antify-relay
        exit 0
    fi
    sleep 1
done
journalctl -u antify-relay -n 20 --no-pager
exit 1
