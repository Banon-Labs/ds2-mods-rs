#!/usr/bin/env bash
# Run `.github/workflows/release.yml`'s `build` job locally, in a container, with `act`.
#
# WHAT THIS CAN AND CANNOT TELL YOU, because a green act run and a green GitHub run are not the
# same claim. act executes the real steps in a real container, so the cross-compile, the staging
# shell and the zip are genuinely exercised. Three steps are cloud-only and fail here for reasons
# that are NOT defects in the workflow:
#
#   actions/attest@v4         needs a Sigstore OIDC token. act cannot mint one, and there is
#                             nothing to configure -- the identity is the point of the attestation.
#   actions/upload-artifact   needs GitHub's artifact service. --artifact-server-path below stands
#                             in for it, so the step passes but proves nothing about the real one.
#   gh release create         never runs: the step is gated on `refs/heads/main` and this is a
#                             pull_request event. That gate is exactly what is being relied on, so
#                             a local run reaching it at all would be the finding.
#
# So: read a failure at `Build what ships` or `Stage the release assets` as real, and a failure at
# `Attest build provenance` as the expected cost of running off GitHub.
#
# NO SECRETS ARE PASSED, AND NONE SHOULD BE. Both repos involved (this one and Banon-Labs/dearxan)
# are public, so checkout and the sibling clone need no token. Handing act a real GITHUB_TOKEN
# would put a live credential inside a container and into its log for no step that needs one.
#
# REQUIRES A DOCKER DAEMON, which is not installed by default on this machine -- /usr/local/bin/docker
# is only the client binary. `sudo pacman -S --needed docker && sudo systemctl enable --now docker`
# and a `docker` group membership; `sg docker -c scripts/run-act.sh` applies that group without a
# re-login.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Artifacts land outside the repo. They are container output, not source, and an act run drops a
# few hundred MB of them.
ARTIFACT_DIR="${ACT_ARTIFACT_DIR:-${TMPDIR:-/tmp}/ds2-act-artifacts}"
mkdir -p "$ARTIFACT_DIR"

if ! docker info >/dev/null 2>&1; then
  echo "no docker daemon reachable -- see the header of this script" >&2
  exit 1
fi

echo "act artifacts -> $ARTIFACT_DIR"
exec act pull_request \
  --job build \
  --platform ubuntu-latest=catthehacker/ubuntu:act-latest \
  --artifact-server-path "$ARTIFACT_DIR" \
  --rm \
  "$@"
