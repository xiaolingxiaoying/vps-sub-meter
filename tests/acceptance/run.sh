#!/usr/bin/env sh
# Runs the release-artifact acceptance suites in the two supported distributions.
# SBCTL_ARTIFACT must name the Linux release binary being accepted.
#
# The terminal client and sbcli legs live in the singbox-sub-me workspace. This
# repository is the server only, so those two binaries are optional here:
# without them the two client assertions are skipped, the script says so out
# loud, and the server assertions (bootstrap installer, fixture transaction,
# real systemd install) still run.
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
artifact=${SBCTL_ARTIFACT:?set SBCTL_ARTIFACT to the Linux release binary to accept}
fixture_artifact=${SBCTL_TEST_ARTIFACT:?set SBCTL_TEST_ARTIFACT to a separate test-signing build; never publish it}
test -f "$fixture_artifact" || exit 2
fixture_artifact=$(CDPATH= cd -- "$(dirname "$fixture_artifact")" && pwd)/$(basename "$fixture_artifact")

test -f "$artifact" || { echo "SBCTL_ARTIFACT is not a file: $artifact" >&2; exit 2; }
artifact=$(CDPATH= cd -- "$(dirname "$artifact")" && pwd)/$(basename "$artifact")

client_legs=0
sbtui_artifact=
sbcli_artifact=
if [ -n "${SBCTUI_ARTIFACT:-}" ] && [ -n "${SBCLI_ARTIFACT:-}" ]; then
  sbtui_artifact=$SBCTUI_ARTIFACT
  sbcli_artifact=$SBCLI_ARTIFACT
  test -f "$sbcli_artifact" || { echo "SBCLI_ARTIFACT is not a file: $sbcli_artifact" >&2; exit 2; }
  sbcli_artifact=$(CDPATH= cd -- "$(dirname "$sbcli_artifact")" && pwd)/$(basename "$sbcli_artifact")
  test -f "$sbtui_artifact" || { echo "SBCTUI_ARTIFACT is not a file: $sbtui_artifact" >&2; exit 2; }
  sbtui_artifact=$(CDPATH= cd -- "$(dirname "$sbtui_artifact")" && pwd)/$(basename "$sbtui_artifact")
  client_legs=1
else
  echo 'branch: server-only workspace - skipping the client legs'
  echo '  set SBCTUI_ARTIFACT and SBCLI_ARTIFACT from a singbox-sub-me build to run them' >&2
fi

for image in debian:12-slim ubuntu:22.04 ubuntu:24.04; do
  tag="sbctl-acceptance-$(printf '%s' "$image" | tr ':/' '--')"
  docker build --build-arg "BASE_IMAGE=$image" -f "$repository_root/tests/acceptance/Dockerfile" -t "$tag" "$repository_root"
  container="${tag}-$(date +%s)-$$"
  cleanup() { MSYS_NO_PATHCONV=1 docker rm -f "$container" >/dev/null 2>&1 || true; }
  trap cleanup EXIT INT TERM

  docker_fixture_artifact=$fixture_artifact
  docker_artifact=$artifact
  docker_sbtui_artifact=$sbtui_artifact
  docker_sbcli_artifact=$sbcli_artifact
  if command -v cygpath >/dev/null 2>&1; then
    docker_artifact=$(cygpath -w "$artifact")
    docker_fixture_artifact=$(cygpath -w "$fixture_artifact")
    if [ "$client_legs" = 1 ]; then
      docker_sbtui_artifact=$(cygpath -w "$sbtui_artifact")
      docker_sbcli_artifact=$(cygpath -w "$sbcli_artifact")
    fi
  fi

  set -- "$tag"
  if [ "$client_legs" = 1 ]; then
    set -- "$@" -v "$docker_sbtui_artifact:/opt/sbtui-client/sbtui:ro" -v "$docker_sbcli_artifact:/opt/sbtui-client/sbcli:ro"
  fi
  MSYS_NO_PATHCONV=1 docker run -d --name "$container" --privileged --cgroupns=host \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    -v "$docker_artifact:/opt/sbctl/sbctl:ro" -v "$docker_fixture_artifact:/opt/sbctl-test/sbctl:ro" \
    "$@" >/dev/null

  ready=false
  for _ in $(seq 1 30); do
    state=$(docker exec "$container" systemctl is-system-running 2>/dev/null || true)
    if [ "$state" = running ] || [ "$state" = degraded ]; then
      ready=true
      break
    fi
    sleep 1
  done
  [ "$ready" = true ] || { docker logs "$container" >&2; exit 1; }

  MSYS_NO_PATHCONV=1 docker exec "$container" /usr/local/lib/sbctl-acceptance/verify-bootstrap.sh
  # Keep the release artifact outside sbctl's managed installation path: a
  # purge deliberately removes /usr/local/bin/sbctl, while this suite needs to
  # verify a subsequent fresh install in the same container.
  # The bootstrap verifier temporarily installs a stub at the managed path;
  # replace it so the generated systemd service starts the release artifact.
  MSYS_NO_PATHCONV=1 docker exec "$container" cp /opt/sbctl-test/sbctl /usr/local/bin/sbctl
  MSYS_NO_PATHCONV=1 docker exec "$container" env SBCTL_BIN=/opt/sbctl-test/sbctl \
    /usr/local/lib/sbctl-acceptance/verify.sh
  MSYS_NO_PATHCONV=1 docker exec "$container" cp /opt/sbctl/sbctl /usr/local/bin/sbctl
  MSYS_NO_PATHCONV=1 docker exec "$container" env SBCTL_BIN=/opt/sbctl/sbctl sbctl-acceptance-real
  if [ "$client_legs" = 1 ]; then
    MSYS_NO_PATHCONV=1 docker exec "$container" env SBTUI_BIN=/opt/sbtui-client/sbtui SBCLI_BIN=/opt/sbtui-client/sbcli \
      /usr/local/lib/sbctl-acceptance/verify-client.sh
    MSYS_NO_PATHCONV=1 docker exec "$container" env SBCLI_BIN=/opt/sbtui-client/sbcli \
      /usr/local/lib/sbctl-acceptance/verify-sbcli.sh
  fi
  cleanup
  trap - EXIT INT TERM
done
