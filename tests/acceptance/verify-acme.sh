#!/usr/bin/env bash
set -euo pipefail
fail() { echo "real ACME acceptance failure: $*" >&2; exit 1; }
. /etc/os-release
systemctl stop nginx
install -m 0755 /opt/sbctl/sbctl /usr/local/bin/sbctl
/opt/test-core/sing-box version
certbot --version
mkdir -p /etc/letsencrypt
printf 'server = https://pebble:14000/dir\nno-random-sleep-on-renew = true\n' >/etc/letsencrypt/cli.ini
curl --fail --silent --show-error --cacert "$REQUESTS_CA_BUNDLE" \
  --retry 10 --retry-connrefused --retry-delay 1 https://pebble:15000/roots/0 >/tmp/pebble-issuance-ca.pem
interface=$(ip -o route show default | awk 'NR == 1 { print $5 }')
sbctl install --mode external-proxy --subscription-host sub.example.test \
  --interface "$interface" \
  --reality-decoy-sni www.cloudflare.com --protocol-sni www.bing.com \
  --sing-box-bin /opt/test-core/sing-box >/dev/null
credential=$(sed -n 's/^subscription_credential = "\([^"]*\)"/\1/p' /etc/sbctl/config.toml)
before=$(sha256sum /etc/sbctl/config.toml)
artifacts_before=$(sha256sum /var/lib/sbctl/artifacts/*)
# The CA resolves this name to its own container, where TCP 80 is closed. This
# is a real unsuccessful HTTP-01 validation, rather than a Certbot stub failure.
if sbctl config switch-mode --mode direct --subscription-host failure.example.test \
  --certificate-mode domain --protocol-sni failure.example.test --certbot-email test@example.com \
  --sing-box-bin /usr/local/bin/sing-box >/tmp/acme-failure.log 2>&1; then
  fail 'a failed HTTP-01 validation committed the mode switch'
fi
grep -F 'Certbot failed' /tmp/acme-failure.log >/dev/null || { cat /tmp/acme-failure.log >&2; fail 'switch did not reach real Certbot validation'; }
grep -F '旧部署已恢复' /tmp/acme-failure.log >/dev/null || { cat /tmp/acme-failure.log >&2; fail 'missing recovery result'; }
[ "$(sha256sum /etc/sbctl/config.toml)" = "$before" ] || fail 'failure changed configuration'
[ "$(sha256sum /var/lib/sbctl/artifacts/*)" = "$artifacts_before" ] || fail 'failure changed artifacts'
systemctl is-active --quiet sing-box.service
systemctl is-active --quiet sbctl.service
curl --fail --silent "http://127.0.0.1:2080/sub/$credential/uri" >/dev/null
echo 'real HTTP-01 failure and mode rollback passed'

sbctl config switch-mode --mode direct --certificate-mode domain --protocol-sni sub.example.test \
  --certbot-email test@example.com --sing-box-bin /usr/local/bin/sing-box
systemctl is-active --quiet sbctl-http.socket
systemctl is-active --quiet sing-box.service
sing-box check -c /etc/sing-box/config.json
https_subscription() {
  curl --fail --silent --show-error --retry 5 --retry-all-errors --retry-delay 1 \
    --cacert /tmp/pebble-issuance-ca.pem --resolve "$1:443:127.0.0.1" \
    "https://$1/sub/$credential/uri"
}
https_subscription sub.example.test | grep 'vless://' >/dev/null
live=/etc/letsencrypt/live/sub.example.test/fullchain.pem
pinned=/var/lib/sbctl/certificates/sub.example.test/fullchain.pem
cmp "$live" "$pinned"
key=/var/lib/sbctl/certificates/sub.example.test/privkey.pem
[ "$(stat -c '%a %U:%G' "$key")" = '640 root:sbctl-cert' ] || fail 'incorrect pinned key permissions'
echo 'real Certbot issuance and verified HTTPS passed'

# Exercise actual encrypted proxy bytes, independently of subscription rendering.
mkdir -p /tmp/sbctl-acme-origin
printf 'real-anytls-data-plane' >/tmp/sbctl-acme-origin/body.txt
jq '{log: {level: "warn"}, inbounds: [{type: "socks", listen: "127.0.0.1", listen_port: 28081}],
  outbounds: [.inbounds[] | select(.type == "anytls") |
    {type: "anytls", tag: "proxy", server: "127.0.0.1", server_port: .listen_port,
     password: .users[0].password, tls: {enabled: true, server_name: "sub.example.test",
       certificate_path: "/tmp/pebble-issuance-ca.pem"}}], route: {final: "proxy"}}' \
  /etc/sing-box/config.json >/tmp/sbctl-acme-client.json
sing-box check -c /tmp/sbctl-acme-client.json
python3 -m http.server 28080 --bind 127.0.0.1 --directory /tmp/sbctl-acme-origin >/tmp/sbctl-acme-origin.log 2>&1 &
origin_pid=$!
sing-box run -c /tmp/sbctl-acme-client.json >/tmp/sbctl-acme-client.log 2>&1 &
client_pid=$!
trap 'kill "$client_pid" "$origin_pid" 2>/dev/null || true' EXIT
body=$(curl --fail --silent --show-error --retry 5 --retry-all-errors --retry-delay 1 \
  --noproxy '' --socks5-hostname 127.0.0.1:28081 http://127.0.0.1:28080/body.txt) \
  || { cat /tmp/sbctl-acme-client.log >&2; fail 'real AnyTLS proxy request failed'; }
[ "$body" = 'real-anytls-data-plane' ] || fail 'real proxy response mismatch'
kill "$client_pid" "$origin_pid"
wait "$client_pid" "$origin_pid" 2>/dev/null || true
trap - EXIT
rm -f /tmp/sbctl-acme-client.json
echo 'real sing-box AnyTLS data transfer with verified TLS passed'

old_fingerprint=$(openssl x509 -in "$pinned" -noout -fingerprint -sha256)
old_pid=$(systemctl show -p MainPID --value sbctl.service)
# force-renewal forces a new order even though this test certificate is fresh.
# The managed deploy hook, not this script, must publish the new pinned copy.
certbot renew --cert-name sub.example.test --force-renewal --non-interactive
cmp "$live" "$pinned"
[ "$(openssl x509 -in "$pinned" -noout -fingerprint -sha256)" != "$old_fingerprint" ] || fail 'renewal did not replace the pinned certificate'
sleep 2
https_subscription sub.example.test >/dev/null
served_fingerprint=$(printf '' | openssl s_client -connect 127.0.0.1:443 -servername sub.example.test 2>/dev/null | openssl x509 -noout -fingerprint -sha256)
[ "$served_fingerprint" = "$(openssl x509 -in "$pinned" -noout -fingerprint -sha256)" ] || fail 'HTTPS still serves the old certificate'
[ "$(systemctl show -p MainPID --value sbctl.service)" = "$old_pid" ] || fail 'certificate reload restarted the subscription process'
sbctl certificate renew >/dev/null
echo 'real Certbot renewal, deploy hook and live TLS reload passed'

sbctl config switch-mode --mode external-proxy --listen-port 2080 >/dev/null
sbctl config switch-mode --mode ip-fallback --subscription-host 127.0.0.1 --http-port 2081 --certificate-mode self-signed >/dev/null
sbctl config switch-mode --mode direct --subscription-host sub.example.test --certificate-mode domain --protocol-sni sub.example.test >/dev/null
https_subscription sub.example.test >/dev/null
sing-box check -c /etc/sing-box/config.json
sbctl uninstall --purge >/dev/null
test -f "$live" || fail 'uninstall removed Certbot issuance history'
install -m 0755 /opt/sbctl/sbctl /usr/local/bin/sbctl
sbctl install --mode direct --subscription-host fresh.example.test --interface "$interface" \
  --reality-decoy-sni www.cloudflare.com \
  --sing-box-bin /opt/test-core/sing-box >/dev/null
credential=$(sed -n 's/^subscription_credential = "\([^"]*\)"/\1/p' /etc/sbctl/config.toml)
https_subscription fresh.example.test >/dev/null
sing-box check -c /etc/sing-box/config.json
sbctl uninstall --purge >/dev/null
echo "real Certbot/sing-box ACME acceptance passed on $ID $VERSION_ID"
