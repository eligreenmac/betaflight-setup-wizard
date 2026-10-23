#!/usr/bin/env bash
# Install the packages listed in $1 with apt, resiliently. $2 is a .deb cache dir
# (restored/saved by actions/cache), so a warm cache means no package download.
#
# Package list: one per line, `#` comments. `a | b` = first of a, b that this release's
# apt knows (for names that differ between Ubuntu releases).
#
# - non-interactive, apt-level retries, 30 s per-connection network timeout;
# - up to 3 attempts of update -> download-only -> install, all within one time budget
#   (APT_BUDGET, default 450 s, under the workflow step's 8 min timeout). Per attempt,
#   update is capped at APT_UPDATE_TIMEOUT (45 s) and the download at
#   APT_DOWNLOAD_TIMEOUT (240 s), and both are also capped by the remaining budget minus
#   APT_INSTALL_RESERVE (75 s). The runner's Azure mirror has trickled at ~70 kB/s
#   without ever hitting a network timeout. The dpkg install itself is never killed
#   (that would break the system); the step timeout bounds it.
# - from attempt 2, every Ubuntu archive source (GitHub runners use
#   `mirror+file:/etc/apt/apt-mirrors.txt`, Azure first) is pointed at
#   APT_FALLBACK_MIRROR (http://archive.ubuntu.com/ubuntu/). Partial downloads resume.
# - each phase logs the mirror host(s) apt actually contacted.
# - APT_TEST_BREAK_FIRST_MIRROR=1 (CI self-test): empty the .deb cache and point
#   attempt 1 at an unreachable mirror, so the fallback path must run.
set -uo pipefail

list=$1
archives=$2
update_limit=${APT_UPDATE_TIMEOUT:-45}
download_limit=${APT_DOWNLOAD_TIMEOUT:-240}
budget=${APT_BUDGET:-450}
reserve=${APT_INSTALL_RESERVE:-75}
fallback=${APT_FALLBACK_MIRROR:-http://archive.ubuntu.com/ubuntu/}
broken_mirror=http://127.0.0.1:9/ubuntu/
start=$SECONDS
log=$(mktemp)

mkdir -p "$archives/partial"

# apt <time limit in s, 0 = none> <apt-get args...>; output is also kept in $log.
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
    "$@" 2>&1 | tee "$log"
  return "${PIPESTATUS[0]}"
}

# Hosts apt contacted in the last phase (from its Get/Hit/Ign/Err lines).
hosts_used() {
  local h
  h=$(grep -oE '^(Get|Hit|Ign|Err):[0-9]+ [a-z+]+://[^/ ]+' "$log" | awk '{print $2}' |
    grep -v '^file:' | sort -u | tr '\n' ' ')
  echo "${h:-none (nothing fetched)}"
}

# Point every Ubuntu archive source at $1: the runner's mirror list, deb822 .sources
# and one-line .list entries (mirror+file:, azure/archive URLs, the test mirror).
set_mirror() {
  local url=$1 f
  if [ -f /etc/apt/apt-mirrors.txt ]; then
    printf '%s\tpriority:1\n' "$url" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
  fi
  for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    [ -f "$f" ] || continue
    sudo sed -i -E \
      -e "s#mirror\+file:/etc/apt/apt-mirrors\.txt#${url}#g" \
      -e "s#https?://(azure\.)?archive\.ubuntu\.com/ubuntu/?#${url}#g" \
      -e "s#${broken_mirror}#${url}#g" "$f"
  done
  echo "apt archive sources now:"
  grep -hsE '^(URIs:|deb )' /etc/apt/sources.list /etc/apt/sources.list.d/*.list \
    /etc/apt/sources.list.d/*.sources | sed 's/^/  /'
  [ -f /etc/apt/apt-mirrors.txt ] && sed 's/^/  apt-mirrors.txt: /' /etc/apt/apt-mirrors.txt
  return 0
}

left() { echo $((budget - (SECONDS - start) - reserve)); }
min() { if [ "$1" -lt "$2" ]; then echo "$1"; else echo "$2"; fi; }

# Resolve `a | b` alternatives against this release's package lists.
resolve_pkgs() {
  pkgs=()
  local line alt found
  while IFS= read -r line; do
    line=$(sed -e 's/#.*//' -e 's/[[:space:]]//g' <<<"$line")
    [ -n "$line" ] || continue
    found=
    IFS='|' read -ra alts <<<"$line"
    for alt in "${alts[@]}"; do
      if apt-cache show "$alt" >/dev/null 2>&1; then found=$alt; break; fi
    done
    if [ -z "$found" ]; then found=${alts[0]}; fi
    [ "$found" != "${alts[0]}" ] && echo "::notice::using $found instead of ${alts[0]} on $(. /etc/os-release; echo "$PRETTY_NAME")"
    pkgs+=("$found")
  done <"$list"
}

. /etc/os-release
echo "OS: $PRETTY_NAME (VERSION_ID=$VERSION_ID)"
if [ "${APT_TEST_BREAK_FIRST_MIRROR:-}" = 1 ]; then
  echo "::warning::APT_TEST_BREAK_FIRST_MIRROR=1: attempt 1 uses unreachable $broken_mirror and an empty .deb cache"
  sudo find "$archives" -name '*.deb' -delete
  set_mirror "$broken_mirror"
fi

for attempt in 1 2 3; do
  if [ "$attempt" -gt 1 ]; then
    if [ "$(left)" -lt 20 ]; then
      echo "::warning::apt time budget used up; not starting attempt $attempt"
      break
    fi
    echo "::warning::apt attempt $attempt of 3 (previous attempt failed or timed out); switching mirror to $fallback"
    set_mirror "$fallback"
    sudo dpkg --configure -a || true
  fi
  t=$(min "$update_limit" "$(left)")
  if [ "$t" -lt 10 ]; then t=10; fi
  echo "::group::apt-get update (attempt $attempt, limit ${t}s)"
  apt "$t" update && rc=0 || rc=$?
  echo "::endgroup::"
  echo "apt attempt $attempt update: rc=$rc, mirror(s) used: $(hosts_used)"
  [ $rc -eq 0 ] || continue
  resolve_pkgs
  t=$(min "$download_limit" "$(left)")
  if [ "$t" -lt 10 ]; then t=10; fi
  echo "::group::apt-get download (attempt $attempt, limit ${t}s): ${pkgs[*]}"
  apt "$t" install -y --no-install-recommends --download-only "${pkgs[@]}" && rc=0 || rc=$?
  echo "::endgroup::"
  echo "apt attempt $attempt download: rc=$rc, mirror(s) used: $(hosts_used), $(grep -hE '^(Fetched|Need to get)' "$log" | tr '\n' ' ')"
  [ $rc -eq 0 ] || continue
  echo "::group::apt-get install"
  apt 0 install -y --no-install-recommends "${pkgs[@]}" && rc=0 || rc=$?
  echo "::endgroup::"
  if [ $rc -eq 0 ]; then
    # Drop .debs that are no longer downloadable so the cache does not grow.
    apt 0 autoclean >/dev/null || true
    sudo chmod -R a+rX "$archives"
    echo "apt: installed on attempt $attempt in $((SECONDS - start)) s; cache $(du -sh "$archives" | cut -f1)"
    exit 0
  fi
done
echo "::error::apt install failed after $attempt attempt(s), $((SECONDS - start)) s"
exit 1
