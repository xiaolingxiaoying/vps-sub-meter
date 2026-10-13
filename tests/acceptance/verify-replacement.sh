#!/usr/bin/env sh
# Run only in a disposable systemd acceptance container: replaces host services.
set -eu
candidate=${SBCTL_BIN:?set SBCTL_BIN to the candidate binary outside managed paths}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "replacement acceptance failure: $*" >&2; exit 1; }

# A foreign service has no sbctl ownership marker, reproducing leftover units
# that ordinary uninstall intentionally preserves.
cat > /etc/systemd/system/sing-box.service <<'EOF'
[Unit]
Description=foreign deployment fixture
[Service]
ExecStart=/bin/sleep infinity
[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now sing-box.service nginx.service
printf '#!/bin/sh\nexit 1\n' > "$work/rejecting-kernel"
printf '#!/bin/sh\nexit 0\n' > "$work/accepting-kernel"
chmod 0755 "$work/rejecting-kernel" "$work/accepting-kernel"

# Failure after cleanup must recover files, enablement links and runtime state.
set +e
"$candidate" install --replace-existing --subscription-host sub.example.test \
  --interface eth0 --reality-decoy-sni www.cloudflare.com \
  --sing-box-bin "$work/rejecting-kernel" --no-start > "$work/failure.log" 2>&1
status=$?
set -e
[ "$status" -eq 2 ] || fail "expected failed kernel check, got $status"
grep -F '新安装失败，旧部署已从备份恢复' "$work/failure.log" >/dev/null \
  || { cat "$work/failure.log"; fail 'missing recovery diagnostic'; }
grep -F 'foreign deployment fixture' /etc/systemd/system/sing-box.service >/dev/null
systemctl is-active --quiet sing-box.service
systemctl is-enabled --quiet sing-box.service
systemctl is-active --quiet nginx.service
systemctl is-enabled --quiet nginx.service

# Successful replacement leaves unrelated nginx configuration untouched while
# disabling its port-owning service. --no-start avoids ACME/network dependencies.
before=$(sha256sum /etc/nginx/nginx.conf)
"$candidate" install --replace-existing --subscription-host sub.example.test \
  --interface eth0 --reality-decoy-sni www.cloudflare.com \
  --sing-box-bin "$work/accepting-kernel" --no-start > "$work/success.log" 2>&1 \
  || { cat "$work/success.log"; fail 'replacement installation failed'; }
[ "$before" = "$(sha256sum /etc/nginx/nginx.conf)" ] || fail 'nginx configuration changed'
! systemctl is-active --quiet nginx.service || fail 'nginx still active'
! systemctl is-enabled --quiet nginx.service || fail 'nginx still enabled'
! systemctl is-active --quiet sing-box.service || fail 'old sing-box still active'
grep -F 'managed by sbctl' /etc/systemd/system/sing-box.service >/dev/null
[ ! -e /etc/systemd/system/multi-user.target.wants/sing-box.service ] \
  || fail 'old enablement link survived'
test -f /etc/sbctl/config.toml
find /var/backups/sbctl/reinstall -name services.json -print | grep . >/dev/null
echo 'replacement systemd acceptance passed'
