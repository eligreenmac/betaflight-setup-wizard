#!/usr/bin/env bash
# Creates workstream labels and MVP issues from docs/MVP_ISSUES.md using gh.
# Usage: scripts/create-mvp-issues.sh [owner/repo]
set -euo pipefail
REPO="${1:-eligreenmac/betaflight-setup-wizard}"
declare -A COLORS=([device]=1d76db [flasher]=d93f0b [config]=0e8a16 [ui]=5319e7 [packaging]=fbca04 [shared]=b60205)
for l in "${!COLORS[@]}"; do
  gh label create "$l" --repo "$REPO" --color "${COLORS[$l]}" --force >/dev/null
done
label=""
while IFS= read -r line; do
  if [[ "$line" =~ ^##\ ([a-z]+)$ ]]; then label="${BASH_REMATCH[1]}"; continue; fi
  if [[ "$line" =~ ^-\ \*\*([A-Z][0-9]+)\ –\ (.+)\*\*:\ (.+)$ ]]; then
    id="${BASH_REMATCH[1]}"; title="${BASH_REMATCH[2]}"; body="${BASH_REMATCH[3]}"
    gh issue create --repo "$REPO" --label "$label" \
      --title "$label: $title ($id)" \
      --body "$body"$'\n\n'"Workstream: \`$label\` (branch \`feat/$label\`). See docs/ARCHITECTURE.md and docs/MVP_ISSUES.md."
  fi
done < "$(dirname "$0")/../docs/MVP_ISSUES.md"
