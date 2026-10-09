#!/usr/bin/env sh
set -eu

installer_template=/usr/local/lib/sbctl-acceptance/install.sh
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "bootstrap acceptance failure: $*" >&2; exit 1; }

# Test-only installer: production templates contain no usable default key.
installer="$work/install.sh"
python3 - "$installer_template" "$installer" <<'PY'
from pathlib import Path
import sys
pem = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAJH+I4WMkKYa3EH63BKmD4SGG0ml6OSe35rQuwrNkJys=\n-----END PUBLIC KEY-----"
Path(sys.argv[2]).write_text(Path(sys.argv[1]).read_text().replace("@SBCTL_RELEASE_PUBLIC_KEY_PEM@", pem))
PY
chmod 0755 "$installer"
mkdir -p "$work/bin"
printf '#!/bin/sh\nexit 0\n' > "$work/bin/apt-get"
chmod 0755 "$work/bin/apt-get"

printf '#!/bin/sh\nprintf "%%s\\n" "$@" > /tmp/sbctl-bootstrap-arguments\n' > "$work/sbctl"
printf '#!/bin/sh\nexit 0\n' > "$work/sing-box"
chmod 0755 "$work/sbctl" "$work/sing-box"

sbctl_signer=${SBCTL_BIN:-/opt/sbctl/sbctl}

# Writes a signed manifest for `artifact` into `manifest`.
sign_manifest() {
  artifact=$1
  manifest=$2
  sha=$(sha256sum "$artifact" | awk '{print $1}')
  cat > "$manifest" <<EOF
{"schema":1,"sbctl":{"version":"0.0.0","url":"file://$artifact","sha256":"$sha"},"sing_box":{"version":"0.0.0","url":"file://$work/sing-box","sha256":"$(sha256sum "$work/sing-box" | awk '{print $1}')"},"sing_box_compatibility":[{"min":"0.0.0","max":"0.0.0"}]}
EOF
  # The bootstrap installer authenticates manifests before downloading either
  # artifact. Sign this local fixture with the development-only test key.
  "$sbctl_signer" release sign \
    --manifest "$manifest" \
    --private-key /usr/local/lib/sbctl-acceptance/dev-signing-key.hex \
    --output "$manifest.signed"
  mv "$manifest.signed" "$manifest"
}

# Scenario 1: a fresh host. The installer forwards the operator's flags to the
# installed binary unchanged.
sign_manifest "$work/sbctl" "$work/manifest-amd64.json"
PATH="$work/bin:$PATH" SBCTL_MANIFEST_URL="file://$work/manifest-{arch}.json" "$installer" \
  --mode ip-fallback \
  --subscription-host 127.0.0.1 \
  --proxy-host 127.0.0.1 \
  --http-port 2081 \
  --reality-decoy-sni www.cloudflare.com \
  --disable-protocol vmess-websocket \
  --disable-protocol hysteria2 \
  --disable-protocol tuic \
  --disable-protocol anytls

grep -Fx -- '--http-port' /tmp/sbctl-bootstrap-arguments >/dev/null
grep -Fx -- '2081' /tmp/sbctl-bootstrap-arguments >/dev/null

# Scenario 2: a host that already has a deployment. The replacement decision has
# to be made before the management binary changes, or cancelling the install
# leaves a new binary in front of an old configuration.
cat > "$work/existing-sbctl" <<'EOF'
#!/bin/sh
# Emulates the real read-only preflight: `sbctl install` with no arguments and a
# non-terminal stdin reports the existing deployment and changes nothing.
if [ "$#" -eq 1 ] && [ "$1" = "install" ]; then
  echo 'Existing deployment detected (etc/sbctl/config.toml, var/lib/sbctl/ownership); `sbctl install` only creates a fresh deployment and will not modify it.' >&2
  exit 2
fi
printf '%s\n' "$@" >> /tmp/sbctl-bootstrap-arguments
exit 0
EOF
chmod 0755 "$work/existing-sbctl"
sign_manifest "$work/existing-sbctl" "$work/manifest-existing-amd64.json"

printf 'known-good sbctl\n' > /usr/local/bin/sbctl
chmod 0755 /usr/local/bin/sbctl
before=$(sha256sum /usr/local/bin/sbctl | awk '{print $1}')

# Non-interactive: the installer must refuse, and must not have replaced the
# management binary while deciding. The exit status is asserted exactly: a
# timeout (124) would mean the installer blocked on a prompt nobody could
# answer, which is a failure, not a refusal.
set +e
PATH="$work/bin:$PATH" SBCTL_MANIFEST_URL="file://$work/manifest-existing-{arch}.json" \
  timeout 60 "$installer" </dev/null >"$work/keep.out" 2>&1
keep_status=$?
set -e
[ "$keep_status" -eq 2 ] \
  || fail "the installer should have refused with exit 2 on a non-interactive host, got $keep_status"
grep -F -- '交互终端' "$work/keep.out" >/dev/null \
  || fail 'the non-interactive refusal did not explain that an interactive terminal is required'
after=$(sha256sum /usr/local/bin/sbctl | awk '{print $1}')
[ "$before" = "$after" ] || fail 'the installer replaced the management binary before the replacement decision'

# Interactive: choosing "keep the existing deployment and exit" must also leave
# the binary untouched.
# Send the answer only after the prompt appears. Piping into script(1) closes
# its input before startup completes and can lose the answer or terminate the
# child before it reaches the prompt on a CI runner.
PATH="$work/bin:$PATH" SBCTL_MANIFEST_URL="file://$work/manifest-existing-{arch}.json" \
  python3 - "$installer" "$work/keep-interactive.out" <<'PY'
import errno
import os
from pathlib import Path
import pty
import select
import signal
import sys
import time

pid, terminal = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], [sys.argv[1]])

output = bytearray()
answered = False
deadline = time.monotonic() + 60
try:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            os.kill(pid, signal.SIGKILL)
            raise RuntimeError("interactive keep-and-exit timed out")
        if not select.select([terminal], [], [], remaining)[0]:
            continue
        try:
            chunk = os.read(terminal, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not chunk:
            break
        output.extend(chunk)
        if not answered and "请选择 [1]: ".encode() in output:
            os.write(terminal, b"1\n")
            answered = True
finally:
    os.close(terminal)
    Path(sys.argv[2]).write_bytes(output)
    _, status = os.waitpid(pid, 0)
    if not answered or os.waitstatus_to_exitcode(status) != 0:
        sys.stderr.buffer.write(output)
        raise RuntimeError("interactive keep-and-exit did not finish successfully")
PY
after=$(sha256sum /usr/local/bin/sbctl | awk '{print $1}')
[ "$before" = "$after" ] || fail 'choosing to keep the existing deployment still replaced the management binary'
grep -F -- 'sbctl update' "$work/keep-interactive.out" >/dev/null \
  || fail 'the keep-and-exit path did not point at sbctl update'

echo 'bootstrap acceptance passed'
