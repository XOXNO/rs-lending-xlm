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

upgrade_all=$(awk -F: '$1 == "upgrade-all" {print; exit}' "$ROOT/Makefile")
case "$upgrade_all" in
    *upgrade-price-aggregator*upgrade-governance*) ;;
    *) echo "FAIL: upgrade-all must upgrade price-aggregator before governance" >&2; fail=1 ;;
esac
grep -q '^\.NOTPARALLEL: upgrade-all$' "$ROOT/Makefile" \
    || { echo "FAIL: upgrade-all must stay sequential under make -j" >&2; fail=1; }

# Exercise referral-claim dispatch without a signer or network connection.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/stellar" <<'SH'
#!/usr/bin/env bash
if [ "$1 $2" = "keys public-key" ]; then echo GSIGNER; exit 0; fi
printf '%s\n' "$@" > "$CLAIM_ARGS_FILE"
SH
chmod +x "$tmp/stellar"
export CLAIM_ARGS_FILE="$tmp/args"
if ! PATH="$tmp:$PATH" NETWORK=mainnet SIGNER=deployer CONFIG_ROOT="$ROOT/configs" \
    AGGREGATOR_CONTRACT= INSTRUCTION_LEEWAY=20000000 \
    bash "$SCRIPT" claimAggregatorReferralFees 7 CTOKEN1 CTOKEN2 >/dev/null 2>&1; then
    echo "FAIL: referral claim dispatch failed" >&2; fail=1
elif ! diff -u <(printf '%s\n' contract invoke --instruction-leeway 20000000 \
    --id "$(jq -r '.mainnet.aggregator' "$ROOT/configs/networks.json")" \
    --source deployer --network mainnet -- claim_referral_fees --id 7 \
    --tokens '["CTOKEN1","CTOKEN2"]') "$CLAIM_ARGS_FILE"; then
    echo "FAIL: referral claim arguments changed" >&2; fail=1
fi
rm -f "$CLAIM_ARGS_FILE"
for args in '' '7'; do
    if PATH="$tmp:$PATH" NETWORK=mainnet SIGNER=deployer CONFIG_ROOT="$ROOT/configs" \
        AGGREGATOR_CONTRACT= INSTRUCTION_LEEWAY=20000000 \
        bash "$SCRIPT" claimAggregatorReferralFees $args >/dev/null 2>&1 \
        || [ -e "$CLAIM_ARGS_FILE" ]; then
        echo "FAIL: referral claim must reject missing id/tokens before invoking" >&2; fail=1
    fi
done
exit $fail
