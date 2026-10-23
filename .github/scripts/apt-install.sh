#!/usr/bin/env bash
# Install the packages listed in $1 (one per line, # comments) with apt, resiliently:
# - non-interactive, apt-level retries and a 30 s network timeout per connection;
# - .debs are downloaded into $2 (an actions/cache directory), so a warm cache
#   means no package download at all;
# - up to 3 attempts. Each attempt has a hard time limit on `update` and on the
#   download phase. CI runs have seen Azure's Ubuntu mirror trickle at ~70 kB/s for
#   17 min without ever timing out. From attempt 2 on, the runner's
#   azure.archive.ubuntu.com mirror is swapped for archive.ubuntu.com, and partial
#   downloads resume.
# - the dpkg install itself runs without `timeout` (killing dpkg mid-configure
#   would break the system); the workflow step's timeout-minutes bounds it.
set -uo pipefail

list=$1
archives=$2
update_timeout=${APT_UPDATE_TIMEOUT:-60}
download_timeout=${APT_DOWNLOAD_TIMEOUT:-100}

mapfile -t pkgs < <(sed -e 's/#.*//' -e 's/[[:space:]]//g' "$list" | grep -v '^$')
mkdir -p "$archives/partial"

# apt <time limit in s, 0 = none> <apt-get args...>
apt() {
  local limit=$1
  shift
  local pre=()
  [ "$limit" -gt 0 ] && pre=(timeout "$limit")
  "${pre[@]}" sudo env DEBIAN_FRONTEND=noninteractive apt-get -q \
    -o Acquire::Retries=3 \
    -o Acquire::http::Timeout=30 \
    -o Acquire::https::Timeout=30 \
    -o Dpkg::Use-Pty=0 \
    -o APT::Keep-Downloaded-Packages=true \
    -o "Dir::Cache::Archives=$archives" \
    "$@"
}

use_main_mirror() {
  sudo find /etc/apt -name '*.list' -o -name '*.sources' | while read -r f; do
    sudo sed -i 's#azure\.archive\.ubuntu\.com#archive.ubuntu.com#g' "$f"
  done
}

for attempt in 1 2 3; do
  if [ "$attempt" -gt 1 ]; then
    echo "::warning::apt attempt $attempt of 3 (previous attempt failed or timed out); switching to archive.ubuntu.com"
    use_main_mirror
    sleep 5
  fi
  echo "::group::apt-get update (attempt $attempt, limit ${update_timeout}s)"
  apt "$update_timeout" update && rc=0 || rc=$?
  echo "::endgroup::"
  [ $rc -eq 0 ] || continue
  echo "::group::apt-get download (attempt $attempt, limit ${download_timeout}s)"
  apt "$download_timeout" install -y --no-install-recommends --download-only "${pkgs[@]}" && rc=0 || rc=$?
  echo "::endgroup::"
  [ $rc -eq 0 ] || continue
  echo "::group::apt-get install"
  apt 0 install -y --no-install-recommends "${pkgs[@]}" && rc=0 || rc=$?
  echo "::endgroup::"
  if [ $rc -eq 0 ]; then
    # Drop .debs that are no longer downloadable so the cache does not grow.
    apt 0 autoclean >/dev/null || true
    sudo chmod -R a+rX "$archives"
    du -sh "$archives"
    exit 0
  fi
  sudo dpkg --configure -a || true
done
echo "::error::apt install failed after 3 attempts"
exit 1
