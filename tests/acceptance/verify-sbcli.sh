#!/bin/sh
# Real 1.14.2 gRPC/IPC smoke in the isolated privileged acceptance container.
set -eu
work=$(mktemp -d)
cli="$work/sbcli"
cp "${SBCLI_BIN:-/opt/sbtui-client/sbcli}" "$cli"
chmod 0755 "$cli"
cleanup() {
  "$cli" --data-dir "$work/data" daemon stop >/dev/null 2>&1 || true
  [ -z "${http_pid:-}" ] || kill "$http_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM
mkdir -p "$work/data/core"
if [ -n "${SING_BOX_1142:-}" ]; then
  cp "$SING_BOX_1142" "$work/data/core/sing-box"
else
  curl -fL --retry 3 https://github.com/SagerNet/sing-box/releases/download/v1.14.2/sing-box-1.14.2-linux-amd64.tar.gz -o "$work/core.tar.gz"
  echo "a684484d7477d1437282ee411f4d131d0340aaad60a7868841ebd5d87dd8a0c6  $work/core.tar.gz" | sha256sum -c -
  tar -xzf "$work/core.tar.gz" -C "$work"
  cp "$work/sing-box-1.14.2-linux-amd64/sing-box" "$work/data/core/sing-box"
fi
chmod 0755 "$work/data/core/sing-box"
sb() { "$cli" --data-dir "$work/data" --json "$@"; }
python3 - "$work/http-port" <<'PY' &
import http.server,sys
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self): self.send_response(204); self.end_headers()
    def log_message(self,*args): pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
open(sys.argv[1],'w').write(str(server.server_port))
server.serve_forever()
PY
http_pid=$!
for _ in $(seq 1 50); do [ ! -s "$work/http-port" ] || break; sleep 0.1; done
port=$(cat "$work/http-port")
cat > "$work/profile.json" <<EOF
{"log":{"level":"debug"},"outbounds":[{"type":"direct","tag":"direct"},{"type":"direct","tag":"direct2"},{"type":"urltest","tag":"auto","outbounds":["direct","direct2"],"url":"http://127.0.0.1:$port/","interval":"10m"},{"type":"selector","tag":"proxy","outbounds":["auto","direct"],"default":"direct"}],"route":{"final":"proxy"},"dns":{"servers":[{"type":"local","tag":"local"}]}}
EOF
sb profile import "$work/profile.json" --name fixture
sb config check
sb start
sb status | jq -e '.core_running and .core_runtime_version == "1.14.2" and (.settings.ingress.system_proxy == false)'
sb run >/dev/null 2>"$work/conflict" && exit 1 || test "$?" -eq 5
sb traffic | jq -e '.available'
sb connection list | jq -e '.connections | type == "array"'
sb outbound list | jq -e 'length >= 4'
sb group select proxy direct
sb group urltest auto
sb config mode-enable --proxy proxy --direct direct --proxy-dns local --direct-dns local | jq -e '.pending'
sb config apply
sb mode set global | jq -e '.routing_effect == "configuration-dependent"'
sb mode show | jq -e '.mode == "Global" and .observed'
sb reload
sb logs > "$work/logs"
sb connection close --all
sb restart
if [ "${SBCLI_ACCEPT_TUN:-1}" = 1 ]; then
  sb tun enable
  sb status | jq -e '.core_running and (.settings.ingress.tun == true) and (.settings.ingress.system_proxy == false) and ([.inbounds[].kind] | index("mixed") != null) and ([.inbounds[].kind] | index("tun") != null)'
  sb tun disable
fi
sb stop
sb daemon status | jq -e '.daemon_running and (.core_running == false)'
sb daemon stop
echo 'sbcli real-core acceptance passed'
