# Renders runs/<RUN_TS>/report.md from actions.tsv.
#
# Resource columns are the DECLARED Soroban resources from each signed
# envelope (instructions = CPU meter, disk-read / write bytes) plus the
# resource fee. Memory bytes are not exposed by RPC or the explorer API —
# the linked explorer page shows the full resource breakdown per tx, and
# local memory profiling lives in verification/test-harness budget tests.

write_report() {
    local report="$RUN_DIR/report.md"
    {
        echo "# Live Testnet Integration Run — \`$RUN_TS\`"
        echo
        echo "- Network: **$NETWORK** (\`$RPC_URL\`)"
        echo "- Date: $(date -u '+%Y-%m-%d %H:%M UTC')"
        echo "- Controller: \`${CONTROLLER:-n/a}\`"
        echo "- Central pool: \`${POOL:-n/a}\`"
        echo "- Aggregator: \`$AGGREGATOR\`"
        echo "- Mock Reflector: \`${MOCK:-n/a}\` · Mock RedStone: \`${MOCKRS:-n/a}\`"
        echo
        echo "## Result summary"
        echo
        echo '| status | count | meaning |'
        echo '|---|---|---|'
        awk -F'\t' 'NR>1 {c[$4]++} END {
            m["ok"]="transaction succeeded on-chain";
            m["read"]="read-only view (simulated, no tx)";
            m["xfail"]="reverted exactly as the test expected";
            m["sim-ok"]="budget probe: fits in the per-tx budget";
            m["sim-exceeded"]="budget probe: Budget,ExceededLimit";
            m["sim-error"]="budget probe: non-budget simulation error";
            m["FAIL"]="UNEXPECTED failure";
            m["UNEXPECTED-OK"]="expected a revert but succeeded";
            for (k in c) printf "| %s | %d | %s |\n", k, c[k], m[k];
        }' "$ACTIONS_TSV"
        echo
        local phases
        phases=$(awk -F'\t' 'NR>1 && !seen[$2]++ {print $2}' "$ACTIONS_TSV")
        for ph in $phases; do
            echo "## Phase: $ph"
            echo
            echo '| # | action | status | fn | instructions | read B | write B | fee (stroops) | tx | note |'
            echo '|---|---|---|---|---|---|---|---|---|---|'
            awk -F'\t' -v ph="$ph" -v xp="$EXPLORER_TX" 'NR>1 && $2==ph {
                link = ($6 != "") ? "[" substr($6,1,8) "…](" xp "/" $6 ")" : "";
                printf "| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |\n",
                    $1, $3, $4, $5, $7, $8, $9, $10, link, $11;
            }' "$ACTIONS_TSV"
            echo
        done
        echo "---"
        echo "_Every linked transaction page on stellar.expert shows the full"
        echo "resource report (CPU instructions, memory, read/write entries and"
        echo "bytes) under \"Resources\"._"
    } > "$report"
    log "report written: $report"
}
