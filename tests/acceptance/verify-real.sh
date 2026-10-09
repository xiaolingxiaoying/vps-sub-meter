#!/usr/bin/env sh
set -eu

sbctl=${SBCTL_BIN:-/usr/local/bin/sbctl}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "real acceptance failure: $*" >&2; exit 1; }
contains() { printf '%s' "$1" | grep -F -- "$2" >/dev/null || fail "expected output to contain: $2"; }

. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) fail "unexpected distribution: $ID" ;;
esac

interface=$(ip -o route show default 2>/dev/null | awk 'NR == 1 { print $5 }')
[ -n "$interface" ] || interface=eth0

# The stub accepts generated configurations and stays alive when supervised by
# systemd. It is deliberately local so this test does not depend on a sing-box
# release or on external network access.
fake_sing_box="$work/sing-box"
cat >"$fake_sing_box" <<'EOF'
#!/bin/sh
case "${1:-}" in
  check) exit 0 ;;
  run) while :; do sleep 3600; done ;;
  *) exit 0 ;;
esac
EOF
chmod 0755 "$fake_sing_box"

install_output=$(
  "$sbctl" install \
    --mode external-proxy \
    --subscription-host sub.example.test \
    --interface "$interface" \
    --reality-decoy-sni www.cloudflare.com \
    --sing-box-bin "$fake_sing_box"
)
contains "$install_output" '启用协议: vless-reality, vmess-websocket, hysteria2, tuic, anytls'

systemctl is-active --quiet sbctl.service || fail 'sbctl.service is not active'
systemctl is-active --quiet sing-box.service || fail 'sing-box.service is not active'
id sbctl >/dev/null 2>&1 || fail 'dedicated sbctl account was not created'
sbctl_shell=$(getent passwd sbctl | cut -d: -f7)
[ "$sbctl_shell" = /usr/sbin/nologin ] || fail 'sbctl account must not permit interactive login'
service_user=$(systemctl show -p User --value sbctl.service)
[ "$service_user" = sbctl ] || fail 'sbctl.service does not run as sbctl'
service_pid=$(systemctl show -p MainPID --value sbctl.service)
runtime_user=$(ps -o user= -p "$service_pid" | tr -d '[:space:]')
[ "$runtime_user" = sbctl ] || fail 'sbctl.service process is not running as the sbctl account'

credential=$(sed -n 's/^subscription_credential = "\([^"]*\)"/\1/p' /etc/sbctl/config.toml)
[ -n "$credential" ] || fail 'subscription credential was not persisted'
for format in sing-box.json clash.yaml uri; do
  response=$(curl --silent --show-error --include --retry 5 --retry-connrefused --retry-delay 1 \
    "http://127.0.0.1:2080/sub/$credential/$format")
  contains "$response" 'HTTP/1.1 200 OK'
  contains "$response" 'subscription-userinfo:'
done

status=$($sbctl status)
contains "$status" 'sbctl.service: active'
contains "$status" 'sing-box.service: active'

$sbctl restart >/dev/null
systemctl is-active --quiet sbctl.service || fail 'sbctl.service did not recover after restart'
systemctl is-active --quiet sing-box.service || fail 'sing-box.service did not recover after restart'

# A candidate that passes `sing-box check` and exits on start must fail the
# update, restore the running binary, and leave the unit stable. This is the
# Ubuntu VPS 2026-09-22 report section 6.13 regression: a single `is-active`
# probe used to commit the crashing candidate while Restart=on-failure looped
# it.
broken_sing_box="$work/broken-sing-box"
cat >"$broken_sing_box" <<'EOF'
#!/bin/sh
case "${1:-}" in
  check) exit 0 ;;
  run) exit 1 ;;
  *) exit 0 ;;
esac
EOF
chmod 0755 "$broken_sing_box"
before_hash=$(sha256sum /usr/local/bin/sing-box | awk '{print $1}')
if $sbctl sing-box update --artifact "$broken_sing_box" >"$work/broken-update.log" 2>&1; then
  cat "$work/broken-update.log" >&2 || true
  fail 'sing-box update accepted a candidate that crashes on start'
fi
after_hash=$(sha256sum /usr/local/bin/sing-box | awk '{print $1}')
[ "$before_hash" = "$after_hash" ] || fail 'failed sing-box update did not restore the previous binary'
systemctl is-active --quiet sing-box.service || fail 'sing-box.service did not recover after the rejected update'
settled_restarts=$(systemctl show -p NRestarts --value sing-box.service)
sleep 4
later_restarts=$(systemctl show -p NRestarts --value sing-box.service)
[ "$settled_restarts" = "$later_restarts" ] \
  || fail "sing-box.service kept restarting after the rollback: $settled_restarts -> $later_restarts"
response=$(curl --silent --show-error --include --retry 5 --retry-connrefused --retry-delay 1 \
  "http://127.0.0.1:2080/sub/$credential/uri")
contains "$response" 'HTTP/1.1 200 OK'

test -f /etc/ufw/user.rules || {
  mkdir -p /etc/ufw
  printf 'firewall\n' >/etc/ufw/user.rules
}
printf 'proxy\n' >/etc/nginx.conf
$sbctl uninstall >/dev/null
test -f /etc/sbctl/config.toml || fail 'default uninstall removed persistent data'
test -d /var/backups/sbctl || fail 'default uninstall did not preserve a backup'
test "$(cat /etc/ufw/user.rules)" = firewall || fail 'uninstall changed firewall data'
test "$(cat /etc/nginx.conf)" = proxy || fail 'uninstall changed unrelated data'

# A release artifact must accept the public IP fallback port option advertised by
# the bootstrap installer and expose its lower-security HTTP subscription.
$sbctl uninstall --purge >/dev/null
# Production bootstrap installs the management binary before invoking `install`.
# The acceptance artifact lives outside that managed path so it survives purge.
install -m 0755 "$sbctl" /usr/local/bin/sbctl
ip_install_output=$(
  "$sbctl" install \
    --mode ip-fallback \
    --subscription-host 127.0.0.1 \
    --proxy-host 127.0.0.1 \
    --http-port 2081 \
    --interface "$interface" \
    --reality-decoy-sni www.cloudflare.com \
    --protocol-sni www.bing.com \
    --sing-box-bin "$fake_sing_box"
)
contains "$ip_install_output" '启用协议: vless-reality, vmess-websocket, hysteria2, tuic, anytls'
systemctl is-active --quiet sbctl.service || fail 'IP fallback sbctl.service is not active'
systemctl is-active --quiet sing-box.service || fail 'IP fallback sing-box.service is not active'

credential=$(sed -n 's/^subscription_credential = "\([^"]*\)"/\1/p' /etc/sbctl/config.toml)
response=$(curl --silent --show-error --include --retry 5 --retry-connrefused --retry-delay 1 \
  "http://127.0.0.1:2081/sub/$credential/uri")
contains "$response" 'HTTP/1.1 200 OK'
# A no-domain deployment uses self-signed certificates plus the disguised SNI,
# so all five protocols must be advertised rather than only the Reality one.
for scheme in 'vless://' 'vmess://' 'hysteria2://' 'tuic://' 'anytls://'; do
  contains "$response" "$scheme"
done

# Direct mode: systemd itself owns TCP 80/443 through sbctl-http.socket and
# passes both listeners to the non-root sbctl service via LISTEN_FDS.
$sbctl uninstall --purge >/dev/null
install -m 0755 "$sbctl" /usr/local/bin/sbctl
certificate_directory=/etc/letsencrypt/live/sub.example.test
mkdir -p "$certificate_directory"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=sub.example.test" \
  -keyout "$certificate_directory/privkey.pem" \
  -out "$certificate_directory/fullchain.pem" >/dev/null 2>&1
# The Direct TLS listener runs as the non-root sbctl account. A Certbot deploy
# hook grants the service account certificate access after renewal (ticket 09);
# seed the equivalent grant here so this test exercises the socket-activated
# TLS path instead of failing on a root-only private key.
chown -R sbctl:sbctl "$certificate_directory"
chmod 0640 "$certificate_directory/privkey.pem"
direct_install_output=$(
  "$sbctl" install \
    --mode direct \
    --subscription-host sub.example.test \
    --interface "$interface" \
    --reality-decoy-sni www.cloudflare.com \
    --sing-box-bin "$fake_sing_box"
)
contains "$direct_install_output" '启用协议: vless-reality, vmess-websocket, hysteria2, tuic, anytls'
systemctl is-active --quiet sbctl-http.socket || fail 'sbctl-http.socket is not active'
systemctl is-active --quiet sbctl.service || fail 'Direct sbctl.service is not active'
systemctl is-active --quiet sing-box.service || fail 'Direct sing-box.service is not active'
service_user=$(systemctl show -p User --value sing-box.service)
[ "$service_user" = sing-box ] || fail 'sing-box.service does not run as sing-box'
id sing-box >/dev/null 2>&1 || fail 'dedicated sing-box account was not created'
sing_box_shell=$(getent passwd sing-box | cut -d: -f7)
[ "$sing_box_shell" = /usr/sbin/nologin ] || fail 'sing-box account must not permit interactive login'
service_pid=$(systemctl show -p MainPID --value sing-box.service)
runtime_user=$(ps -o user= -p "$service_pid" | tr -d '[:space:]')
[ "$runtime_user" = sing-box ] || fail 'sing-box.service process is not running as the sing-box account'

# The generated units must be valid for the host systemd: Ubuntu 22.04 used to
# report `Unknown key name 'Sockets' in section 'Unit', ignoring` for the
# Direct service unit.
verify_output=$(systemd-analyze verify \
  /etc/systemd/system/sbctl.service \
  /etc/systemd/system/sbctl-http.socket \
  /etc/systemd/system/sing-box.service \
  /etc/systemd/system/sbctl-accounting-reset.service \
  /etc/systemd/system/sbctl-accounting-reset.timer 2>&1 || true)
printf '%s' "$verify_output" | grep -q 'Unknown key name' \
  && fail "systemd reported an unknown unit key: $verify_output"

direct_credential=$(sed -n 's/^subscription_credential = "\([^"]*\)"/\1/p' /etc/sbctl/config.toml)
direct_response=$(mktemp)
if ! curl --silent --show-error --retry 5 --retry-connrefused --retry-delay 1 --insecure \
  --resolve sub.example.test:443:127.0.0.1 \
  "https://sub.example.test/sub/$direct_credential/uri" >"$direct_response"; then
  systemctl --no-pager status sbctl.service >&2 || true
  journalctl --no-pager -u sbctl.service -n 30 >&2 || true
  fail 'Direct HTTPS did not serve the subscription through the systemd socket'
fi
grep -F 'vless://' "$direct_response" >/dev/null \
  || fail 'Direct HTTPS did not serve the subscription through the systemd socket'
rm -f "$direct_response"
direct_token="real-acceptance-token"
mkdir -p /var/lib/sbctl/acme-webroot/.well-known/acme-challenge
printf 'real-challenge-body' > "/var/lib/sbctl/acme-webroot/.well-known/acme-challenge/$direct_token"
challenge=$(curl --silent --show-error --retry 5 --retry-connrefused --retry-delay 1 \
  "http://127.0.0.1:80/.well-known/acme-challenge/$direct_token")
[ "$challenge" = 'real-challenge-body' ] || fail 'Direct HTTP-01 challenge did not serve through the systemd socket'

# A failed update must restore the pinned certificate and the Certbot hook with
# the ownership and mode the deployment depends on. The rollback used to rewrite
# every managed path as `sbctl:sbctl 0600`, which left the `sing-box` account
# unable to read the certificate its listeners present and left Certbot unable
# to execute the hook that re-pins a renewal.
pinned_key=/var/lib/sbctl/certificates/sub.example.test/privkey.pem
deploy_hook=/etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook
[ -f "$pinned_key" ] || fail 'the Direct install did not pin the subscription certificate'
[ -x "$deploy_hook" ] || fail 'the Direct install did not install an executable deploy hook'
key_before=$(stat -c '%a %U:%G' "$pinned_key")
hook_before=$(stat -c '%a %U:%G' "$deploy_hook")

rollback_digest=$(sha256sum "$fake_sing_box" | awk '{print $1}')
rollback_sbctl=${SBCTL_TEST_BIN:-/opt/sbctl-test/sbctl}
test -x "$rollback_sbctl" || fail 'the rollback fixture needs a separate test-signing binary'
printf '{"schema":1,"sbctl":{"version":"0.0.2","sha256":"%s"},"sing_box":{"version":"1.12.0","sha256":"%s"},"sing_box_compatibility":[{"min":"1.12.0","max":"1.12.0"}]}' \
  "$rollback_digest" "$rollback_digest" > "$work/rollback.unsigned.json"
"$rollback_sbctl" release sign \
  --manifest "$work/rollback.unsigned.json" \
  --private-key /usr/local/lib/sbctl-acceptance/dev-signing-key.hex \
  --output "$work/rollback-manifest.json"
# A failing pre-start drop-in makes the health check fail without masking the
# locally installed unit (systemctl refuses to mask a file under /etc). Use the
# test-signing binary for this fixture: the production binary must reject the
# public fixture signature before an update can reach its health check.
failure_drop_in=/etc/systemd/system/sbctl.service.d/acceptance-failure.conf
mkdir -p "$(dirname "$failure_drop_in")"
printf '[Service]\nExecStartPre=/bin/false\n' > "$failure_drop_in"
clear_failure() {
  rm -f "$failure_drop_in"
  rmdir "$(dirname "$failure_drop_in")" 2>/dev/null || true
  systemctl daemon-reload
}
trap 'clear_failure; rm -rf "$work"' EXIT
systemctl daemon-reload
if "$rollback_sbctl" update --manifest "$work/rollback-manifest.json" \
  --sbctl-artifact "$fake_sing_box" --sing-box-artifact "$fake_sing_box" \
  >"$work/rollback-update.out" 2>&1; then
  fail 'an update whose health check fails must not be accepted'
fi
grep -F 'update failed: service health check failed:' "$work/rollback-update.out" >/dev/null \
  || fail "the update did not reach its health check: $(cat "$work/rollback-update.out")"
clear_failure
trap 'rm -rf "$work"' EXIT

key_after=$(stat -c '%a %U:%G' "$pinned_key")
hook_after=$(stat -c '%a %U:%G' "$deploy_hook")
[ "$key_before" = "$key_after" ] \
  || fail "the update rollback changed the pinned private key ($key_before -> $key_after)"
[ "$hook_before" = "$hook_after" ] \
  || fail "the update rollback changed the deploy hook ($hook_before -> $hook_after)"
[ -x "$deploy_hook" ] || fail 'the update rollback removed the deploy hook executable bit'

$sbctl uninstall --purge >/dev/null

echo "real sbctl acceptance passed on $ID $VERSION_ID"
