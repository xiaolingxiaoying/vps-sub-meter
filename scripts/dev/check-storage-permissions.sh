#!/usr/bin/env bash
# Real Linux UID/GID checks in a disposable chroot, without modifying host
# accounts or /etc. Usage: sudo bash check-storage-permissions.sh <Linux sbctl>
set -euo pipefail
if [ "${1:-}" != --in-namespace ]; then
  binary=${1:?usage: check-storage-permissions.sh /absolute/path/to/sbctl}
  [ "$(id -u)" = 0 ] || { echo 'requires root for an isolated mount namespace' >&2; exit 2; }
  exec unshare --mount --fork bash "$0" --in-namespace "$(realpath "$binary")"
fi
binary=${2:?binary}
mount --make-rprivate /
work=$(mktemp -d /tmp/sbctl-permissions.XXXXXX)
chmod 0755 "$work"
cleanup() {
  if mountpoint -q "$work/proc"; then
    umount "$work/proc" || { echo "kept sandbox after unmount failure: $work" >&2; return; }
  fi
  for directory in lib64 lib sbin bin; do
    if mountpoint -q "$work/usr/$directory"; then
      umount "$work/usr/$directory" || { echo "kept sandbox after unmount failure: $work" >&2; return; }
    fi
  done
  rm -rf "$work"
}
trap cleanup EXIT
mkdir -p "$work"/{usr/bin,usr/sbin,usr/lib,usr/lib64,usr/local/bin,etc/sbctl,etc/pam.d,var/lib/sbctl,tmp,proc}
mount --bind /proc "$work/proc"
mount -o remount,bind,ro "$work/proc"
for directory in bin sbin lib lib64; do
  mount --bind "/usr/$directory" "$work/usr/$directory"
  mount -o remount,bind,ro "$work/usr/$directory"
  ln -s "usr/$directory" "$work/$directory"
done
printf 'root:x:0:0:root:/root:/bin/sh\nsbctl:x:65000:65000:sbctl:/:/usr/sbin/nologin\nsing-box:x:65001:65001:sing-box:/:/usr/sbin/nologin\n' >"$work/etc/passwd"
printf 'root:x:0:\nsbctl:x:65000:\nsing-box:x:65001:\nsbctl-cert:x:65002:sbctl,sing-box\n' >"$work/etc/group"
cp "$binary" "$work/usr/local/bin/sbctl"
printf '#!/bin/sh\nexit 0\n' >"$work/usr/local/bin/sing-box"
chmod 0755 "$work/usr/local/bin/"*
chroot "$work" /usr/local/bin/sbctl config init --mode external-proxy \
  --subscription-host sub.example.test --interface ens3 --protocol anytls \
  --sing-box-bin /usr/local/bin/sing-box >/dev/null
printf 'acceptance-only SMTP secret\n' >"$work/etc/sbctl/email.toml"
printf '{"daily":null,"reminder":null}\n' >"$work/var/lib/sbctl/email-state.json"
mkdir -p "$work/etc/sbctl/overrides"
printf '{}\n' >"$work/etc/sbctl/overrides/sing-box-server.json"
chmod 0600 "$work/etc/sbctl/overrides/sing-box-server.json"
# Simulate the ownership left behind by the old recursive preparation.
chown 65000:65000 "$work/etc/sbctl/email.toml" "$work/var/lib/sbctl/email-state.json"
chmod 0600 "$work/etc/sbctl/email.toml" "$work/var/lib/sbctl/email-state.json"
mkdir -p "$work/var/lib/sbctl/acme-webroot/.well-known/acme-challenge"
chmod 0700 "$work/var/lib/sbctl/acme-webroot" "$work/var/lib/sbctl/acme-webroot/.well-known" "$work/var/lib/sbctl/acme-webroot/.well-known/acme-challenge"
printf 'challenge-body\n' >"$work/var/lib/sbctl/acme-webroot/.well-known/acme-challenge/permission-token"
chroot "$work" /usr/local/bin/sbctl regenerate >/dev/null
# Reopen the delegated lock as root after preparation. With
# fs.protected_regular=2, O_CREAT on that existing lock must not be used.
chroot "$work" /usr/local/bin/sbctl regenerate >/dev/null
for relative in etc/sbctl/email.toml var/lib/sbctl/email-state.json; do
  [ "$(stat -c '%u:%g:%a' "$work/$relative")" = 0:0:600 ]
  if chroot "$work" /usr/bin/setpriv --reuid=65000 --regid=65000 --clear-groups \
    /usr/bin/test -r "/$relative"; then echo "service can read $relative" >&2; exit 1; fi
  if chroot "$work" /usr/bin/setpriv --reuid=65000 --regid=65000 --clear-groups \
    /usr/bin/rm "/$relative" 2>/dev/null; then echo "service can remove $relative" >&2; exit 1; fi
done
[ "$(stat -c '%u:%g:%a' "$work/etc/sbctl/overrides/sing-box-server.json")" = 0:0:600 ]
chroot "$work" /usr/bin/setpriv --reuid=65000 --regid=65000 --clear-groups \
  /usr/bin/test -r /etc/sbctl/config.toml
chroot "$work" /usr/bin/setpriv --reuid=65000 --regid=65000 --clear-groups \
  /usr/bin/test -r /var/lib/sbctl/artifacts/subscription-uri.txt
chroot "$work" /usr/bin/setpriv --reuid=65000 --regid=65000 --clear-groups \
  /usr/bin/touch /var/lib/sbctl/daemon-can-write
chroot "$work" /usr/bin/setpriv --reuid=65000 --regid=65000 --clear-groups \
  /usr/bin/test -r /var/lib/sbctl/acme-webroot/.well-known/acme-challenge/permission-token
chroot "$work" /usr/bin/setpriv --reuid=65001 --regid=65001 --clear-groups \
  /usr/bin/test -r /etc/sing-box/config.json
chroot "$work" /usr/bin/setpriv --reuid=65001 --regid=65001 --clear-groups \
  /usr/bin/test -r /var/lib/sbctl/certificates/sub.example.test/key.pem
echo 'Linux storage ownership, private mail, daemon writes and certificate access passed'
