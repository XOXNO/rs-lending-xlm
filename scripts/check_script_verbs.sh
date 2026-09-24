#!/usr/bin/env bash
# Offline checks that configs/script.sh verbs match what `make <network> <verb>`
# dispatches, and that the grantGovRole usage names every accepted role.
# Usage: check_script_verbs.sh "<ALL_ACTIONS>" "<MAKEFILE_ACTIONS>"
set -euo pipefail
export LC_ALL=C

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/configs/script.sh"
# Called only by the upgrade-* recipes, after they upload the wasm.
INTERNAL=" upgradeControllerHash upgradeGovernanceHash upgradePoolHash upgradePositionNftHash upgradePriceAggregatorHash "

[ $# -eq 2 ] || { echo "usage: $0 \"<ALL_ACTIONS>\" \"<MAKEFILE_ACTIONS>\"" >&2; exit 2; }
words() { tr -s ' \t' '\n' <<<"$1" | sed '/^$/d' | sort -u; }

labels=$(awk '/^case "\$1" in/ {on=1; next} on && /^esac/ {exit} on' "$SCRIPT" |
    sed -nE 's/^[[:space:]]*"([A-Za-z]+)"\).*/\1/p' | sort -u)
all=$(words "$1")
forwarded=$(comm -23 <(echo "$all") <(words "$2"))
[ -n "$labels" ] && [ -n "$all" ] || { echo "FAIL: no verbs read" >&2; exit 1; }

fail=0
for verb in $(comm -23 <(echo "$labels") <(echo "$all")); do
    case "$INTERNAL" in
        *" $verb "*) ;;
        *) echo "FAIL: script.sh verb '$verb' is in no Makefile action list" >&2; fail=1 ;;
    esac
done
for verb in $(comm -23 <(echo "$forwarded") <(echo "$labels")); do
    echo "FAIL: make forwards '$verb' to script.sh, which has no case arm for it" >&2
    fail=1
done

roles=$(awk '/^validate_governance_role\(\)/ {on=1} on && /return 0/ {print; exit}' "$SCRIPT" |
    sed -nE 's/^[[:space:]]*([A-Z|]+)\).*/\1/p' | tr '|' ' ')
usage=$(grep -F 'Governance roles:' "$SCRIPT" || true)
[ -n "$roles" ] && [ -n "$usage" ] || { echo "FAIL: cannot read the governance roles" >&2; exit 1; }
for role in $roles; do
    case "$usage" in
        *"$role"*) ;;
        *) echo "FAIL: grantGovRole usage omits role $role" >&2; fail=1 ;;
    esac
done
exit $fail
