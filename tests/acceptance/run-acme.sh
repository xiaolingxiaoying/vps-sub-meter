#!/usr/bin/env bash
# Real Certbot, HTTP-01 validation and sing-box in disposable systemd containers.
set -euo pipefail
repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
artifact=${SBCTL_ARTIFACT:?set SBCTL_ARTIFACT to a production Linux sbctl}
kernel=${SING_BOX_ARTIFACT:?set SING_BOX_ARTIFACT to a real Linux sing-box}
test -f "$artifact"
test -f "$kernel"
artifact=$(CDPATH= cd -- "$(dirname "$artifact")" && pwd)/$(basename "$artifact")
kernel=$(CDPATH= cd -- "$(dirname "$kernel")" && pwd)/$(basename "$kernel")
pebble_image=${PEBBLE_IMAGE:-ghcr.io/letsencrypt/pebble@sha256:ddf230642b1a584f519f32e347de1b05a6e4c1f6c35c1863b33effeab5f78199}
docker_artifact=$artifact
docker_kernel=$kernel
docker_config=$repository_root/tests/acceptance/pebble.json
docker_repository_root=$repository_root
if command -v cygpath >/dev/null 2>&1; then
  docker_artifact=$(cygpath -w "$artifact")
  docker_kernel=$(cygpath -w "$kernel")
  docker_config=$(cygpath -w "$docker_config")
  docker_repository_root=$(cygpath -w "$repository_root")
fi
export MSYS_NO_PATHCONV=1
for image in ${ACME_BASE_IMAGES:-debian:12-slim ubuntu:22.04 ubuntu:24.04}; do
  base="sbctl-acceptance-$(printf '%s' "$image" | tr ':/' '--')"
  tag="$base-acme"
  docker build --build-arg "BASE_IMAGE=$image" -f "$docker_repository_root/tests/acceptance/Dockerfile" -t "$base" "$docker_repository_root"
  docker build --build-arg "BASE_IMAGE=$base" -f "$docker_repository_root/tests/acceptance/Dockerfile.acme" -t "$tag" "$docker_repository_root"
  network="sbctl-acme-$(date +%s)-$$"
  container="$network-host"
  ca="$network-ca"
  cleanup() {
    docker rm -f "$container" "$ca" >/dev/null 2>&1 || true
    docker network rm "$network" >/dev/null 2>&1 || true
    rm -f -- "$repository_root/.scratch/acceptance-bin/$network-transport-ca.pem"
  }
  trap cleanup EXIT INT TERM
  docker network create "$network" >/dev/null
  docker run -d --name "$ca" --network "$network" --network-alias pebble --network-alias failure.example.test \
    -e PEBBLE_VA_NOSLEEP=1 -e PEBBLE_AUTHZREUSE=0 -e PEBBLE_WFE_NONCEREJECT=0 \
    -v "$docker_config:/test/sbctl-pebble.json:ro" \
    "$pebble_image" -config /test/sbctl-pebble.json -strict=false >/dev/null
  docker run -d --name "$container" --network "$network" \
    --network-alias sub.example.test --network-alias fresh.example.test \
    --privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    -v "$docker_artifact:/opt/sbctl/sbctl:ro" -v "$docker_kernel:/opt/test-core/sing-box:ro" \
    "$tag" >/dev/null
  ready=false
  for _ in $(seq 1 30); do
    state=$(docker exec "$container" systemctl is-system-running 2>/dev/null || true)
    if [ "$state" = running ] || [ "$state" = degraded ]; then ready=true; break; fi
    sleep 1
  done
  [ "$ready" = true ] || { docker logs "$container" >&2; exit 1; }
  # Trust only the ACME transport in this container; never alter a host trust store.
  mkdir -p "$repository_root/.scratch/acceptance-bin"
  docker cp "$ca:/test/certs/pebble.minica.pem" "$docker_repository_root/.scratch/acceptance-bin/$network-transport-ca.pem"
  docker cp "$docker_repository_root/.scratch/acceptance-bin/$network-transport-ca.pem" "$container:/tmp/pebble-transport-ca.pem"
  if ! docker exec "$container" env REQUESTS_CA_BUNDLE=/tmp/pebble-transport-ca.pem \
      /usr/local/lib/sbctl-acceptance/verify-acme.sh; then
    docker logs "$ca" >&2
    docker exec "$container" journalctl --no-pager -u sbctl.service -u sing-box.service -n 60 >&2 || true
    exit 1
  fi
  docker logs "$ca" 2>&1 | grep 'Attempting to validate w/ HTTP:' >/dev/null
  cleanup
  trap - EXIT INT TERM
done
