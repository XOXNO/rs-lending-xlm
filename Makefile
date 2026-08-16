





































SHELL := /bin/bash
.PHONY: \
        build build-one optimize deploy-artifacts integration-wasm integration-preflight integration-validate integration-shellcheck integration-appendix certora-wasm wasm-artifacts \
        certora certora-list \
        test test-verbose test-one test-match test-pool \
        miri-common miri-pool miri-controller miri-all \
        coverage coverage-controller coverage-pool coverage-price-aggregator coverage-merged \
        fmt fmt-check clippy clippy-contracts clippy-fuzz scout scout-host scout-strict \
        access-control-check \
        wasm-size-check wasm-testing-abi-check clean install-stellar-cli \
        cbm-reindex cbm-index \
        _mutants-check _mutants-harness-prepare \
        mutants mutants-math mutants-rates mutants-pool-interest mutants-common mutants-pool \
        mutants-governance mutants-governance-oracle-probe mutants-diff \
        mutants-controller-core mutants-controller-oracle mutants-controller-positions \
        mutants-controller-strategies mutants-controller-views \
        fuzz fuzz-contract fuzz-one fuzz-build fuzz-seed-corpus \
        fuzz-coverage fuzz-coverage-all fuzz-coverage-one fuzz-coverage-clean \
        proptest proptest-one proptest-build \
        keygen deploy-testnet deploy-mainnet upgrade-controller upgrade-governance upgrade-pool upgrade-all _deploy \
        _preflight-tools _preflight-network-config _preflight-validate-configs _preflight-setup _preflight-controller _preflight-governance _preflight-pool-hash \
        _preflight-configure-controller _preflight-upgrade-pool _post-setup-status \
        build-flash-loan-receiver deploy-flash-loan-receiver fund-flash-loan-receiver test-flash-loan-receiver \
        build-aggregator deploy-aggregator prepay-rent \
        build-oracle-adapter deploy-oracle-adapter upgrade-oracle-adapter upgrade-oracle-adapter-full \
        configure-controller setup-testnet setup-mainnet _setup-markets _unpause-after-setup \
        info invoke invoke-id view view-id \
        testnet mainnet \
        usage help help-build help-verify help-deploy help-ops \
        help-views help-oracle help-aggregator help-all





WASM_TARGET  := wasm32v1-none


CARGO_TARGET_DIR ?= target
RELEASE_DIR  := $(CARGO_TARGET_DIR)/$(WASM_TARGET)/release


WASM_STACK_SIZE ?= 16384
WASM_RUSTFLAGS := -C link-arg=-zstack-size=$(WASM_STACK_SIZE)
OPTIMIZED_DIR := target/optimized

WASM_ARTIFACTS_DIR := artifacts/wasm
DEPLOY_DIR := $(WASM_ARTIFACTS_DIR)/deploy
CERTORA_WASM_DIR := $(WASM_ARTIFACTS_DIR)/certora
CERTORA_BUILD_DIR := target/certora-build


CERTORA_BUILD_JOBS ?= 1
COV_DIR := target/coverage
TEST_HARNESS_DIR := tests/test-harness
FUZZ_DIR := tests/fuzz


CONTRACTS := pool controller governance


WASM_SIZE_CONTRACTS := pool controller governance common flash_loan_receiver defindex_strategy price_aggregator position_nft






COV_IGNORE := --ignore-filename-regex='(^|/)(tests/test-harness|tests/fuzz|certora|vendor|target)/|common/src/types/(shared|aggregator)\.rs$$'


NETWORK     ?= testnet
SIGNER      ?= deployer
CONTRACT    ?= controller
CONFIG_DIR  ?= configs
FLASH_MARKET ?= XLM
FLASH_LOAN_AMOUNT ?= 10000000
FLASH_RECEIVER_FUND ?= 10000000

AGGREGATOR_ADMIN ?=



ORACLE_ADAPTER_ADMIN ?=
ORACLE_ADAPTER_SIGNERS ?=
ORACLE_ADAPTER_THRESHOLD ?= 1
ORACLE_ADAPTER_RESOLUTION ?= 60
POOL_WASM_HASH_FILE ?= target/pool_wasm_hash.txt
POOL_UPGRADE_WASM_HASH_FILE ?= target/pool_upgrade_wasm_hash.txt
CONTROLLER_WASM_HASH_FILE ?= target/controller_wasm_hash.txt
PRICE_AGGREGATOR_WASM_HASH_FILE ?= target/price_aggregator_wasm_hash.txt
POSITION_NFT_WASM_HASH_FILE ?= target/position_nft_wasm_hash.txt
POSITION_NFT_URI ?= https://xoxno.com/nft/lending/
POSITION_NFT_NAME ?= XOXNO Lending Position
POSITION_NFT_SYMBOL ?= XLP
GOVERNANCE_WASM_HASH_FILE ?= target/governance_wasm_hash.txt
SIGNER_ADDRESS = $$(stellar keys public-key $(SIGNER) 2>/dev/null || stellar keys address $(SIGNER) 2>/dev/null || echo $(SIGNER))






STELLAR_RPC_URL = $(shell jq -r '.["$(NETWORK)"].rpc_url // empty' $(CONFIG_DIR)/networks.json 2>/dev/null)
STELLAR_NETWORK_PASSPHRASE = $(shell jq -r '.["$(NETWORK)"].network_passphrase // empty' $(CONFIG_DIR)/networks.json 2>/dev/null)
export STELLAR_RPC_URL
export STELLAR_NETWORK_PASSPHRASE


ifeq ($(SIGNER),ledger)
  SOURCE_FLAG = --source-account $(SIGNER_ADDRESS) --sign-with-ledger
else
  SOURCE_FLAG = --source $(SIGNER)
endif






build:
	@echo "Building all contracts (stack-size $(WASM_STACK_SIZE))..."
	CARGO_BUILD_RUSTFLAGS="$(WASM_RUSTFLAGS)" stellar contract build
	@echo ""
	@echo "WASM artifacts:"
	@ls -lh $(RELEASE_DIR)/*.wasm 2>/dev/null || ls -lh target/wasm32-unknown-unknown/release/*.wasm 2>/dev/null || echo "  (none found)"


build-one:
	@echo "Building $(CRATE) (stack-size $(WASM_STACK_SIZE))..."
	CARGO_BUILD_RUSTFLAGS="$(WASM_RUSTFLAGS)" stellar contract build --package $(CRATE)


optimize: build
	@mkdir -p $(OPTIMIZED_DIR)
	@for contract in $(WASM_SIZE_CONTRACTS); do \
		echo "Optimizing $$contract..."; \
		if command -v stellar &>/dev/null; then \
			stellar contract optimize \
				--wasm $(RELEASE_DIR)/$${contract//-/_}.wasm \
				--wasm-out $(OPTIMIZED_DIR)/$$contract.wasm 2>/dev/null || \
			cp $(RELEASE_DIR)/$${contract//-/_}.wasm $(OPTIMIZED_DIR)/$$contract.wasm; \
		elif command -v wasm-opt &>/dev/null; then \
			wasm-opt -Oz $(RELEASE_DIR)/$${contract//-/_}.wasm \
				-o $(OPTIMIZED_DIR)/$$contract.wasm; \
		else \
			cp $(RELEASE_DIR)/$${contract//-/_}.wasm $(OPTIMIZED_DIR)/$$contract.wasm; \
		fi; \
	done
	@echo ""
	@echo "Optimized WASM:"
	@ls -lh $(OPTIMIZED_DIR)/*.wasm 2>/dev/null




deploy-artifacts: optimize
	@mkdir -p $(DEPLOY_DIR)
	@for contract in $(WASM_SIZE_CONTRACTS); do \
		src="$(OPTIMIZED_DIR)/$$contract.wasm"; \
		dst="$(DEPLOY_DIR)/$$contract.wasm"; \
		python3 scripts/strip_spec_docs.py "$$src" "$$dst" || cp "$$src" "$$dst"; \
	done
	@$(MAKE) --no-print-directory _wasm-manifest DEPLOY=1
	@echo ""
	@echo "Deploy WASM ($(DEPLOY_DIR)):"
	@ls -lh $(DEPLOY_DIR)/*.wasm 2>/dev/null




certora-wasm:
	@set -euo pipefail; \
	mkdir -p $(CERTORA_WASM_DIR) $(CERTORA_BUILD_DIR); \
	source_snapshot=$$(mktemp "$(CERTORA_BUILD_DIR)/focused-inputs.XXXXXX"); \
	trap '/bin/rm -f -- "$$source_snapshot"' EXIT; \
	python3 certora/scripts/write_wasm_manifest.py \
		--write-input-snapshot "$$source_snapshot"; \
	python3 certora/scripts/focused_wasm.py | while IFS='|' read -r layer pkg feature artifact build_key; do \
		echo "Building focused certora $$layer/$$feature (optimize=false)..."; \
		src="$(CERTORA_BUILD_DIR)/focused/$(WASM_TARGET)/release/$${pkg//-/_}.wasm"; \
		/bin/rm -f "$$src"; \
		CARGO_BUILD_JOBS="$(CERTORA_BUILD_JOBS)" \
		CARGO_TARGET_DIR="$(CERTORA_BUILD_DIR)/focused" \
			stellar contract build --package $$pkg \
				--features "certora,certora-focused,$$feature" --optimize=false; \
		test -s "$$src"; \
		dst="$(CERTORA_WASM_DIR)/$$artifact"; \
		/bin/cp -f "$$src" "$$dst"; \
	done; \
	python3 certora/scripts/write_wasm_manifest.py \
		--certora --input-snapshot "$$source_snapshot"; \
	python3 certora/scripts/write_wasm_manifest.py \
		--check-input-snapshot "$$source_snapshot"; \
	echo ""; \
	echo "Certora WASM ($(CERTORA_WASM_DIR)):"; \
	ls -lh $(CERTORA_WASM_DIR)/*.wasm 2>/dev/null


integration-wasm: deploy-artifacts
	@mkdir -p $(OPTIMIZED_DIR)
	@for wasm in controller pool governance flash_loan_receiver defindex_strategy price_aggregator; do \
		cp "$(DEPLOY_DIR)/$$wasm.wasm" "$(OPTIMIZED_DIR)/$$wasm.wasm"; \
	done
	@for pkg in mock_oracle mock_redstone swap_aggregator; do \
		echo "Optimizing $$pkg for integration..."; \
		if command -v stellar &>/dev/null; then \
			stellar contract optimize \
				--wasm $(RELEASE_DIR)/$$pkg.wasm \
				--wasm-out $(OPTIMIZED_DIR)/$$pkg.wasm 2>/dev/null || \
			cp $(RELEASE_DIR)/$$pkg.wasm $(OPTIMIZED_DIR)/$$pkg.wasm; \
		else \
			cp $(RELEASE_DIR)/$$pkg.wasm $(OPTIMIZED_DIR)/$$pkg.wasm; \
		fi; \
	done
	@echo ""
	@echo "Integration WASM ($(OPTIMIZED_DIR)):"
	@ls -lh $(OPTIMIZED_DIR)/{controller,pool,flash_loan_receiver,defindex_strategy,price_aggregator,mock_oracle,mock_redstone}.wasm 2>/dev/null



integration-appendix:
	@echo "Generating tests/integration/appendix.md from test-harness budget data..."
	@mkdir -p tests/integration
	@( \
	  echo "# Memory & resource budgets (auto-generated from test-harness)"; \
	  echo; \
	  echo "_Regenerate with: make integration-appendix (or run specific meta tests)._"; \
	  echo; \
	  echo "See tests/test-harness/tests/meta/budget_breakdown.rs and footprint_test.rs."; \
	  echo "Run e.g.:"; \
	  echo '  cargo test -p test-harness --test meta budget_breakdown -- --nocapture 2>&1 | tail -100'; \
	) > tests/integration/appendix.md
	@echo "Wrote tests/integration/appendix.md (update with real numbers from harness when budgets change)."


.PHONY: integration-preflight integration-validate integration-shellcheck

integration-preflight: integration-wasm
	@echo "Running integration harness preflight..."
	@bash -c 'source tests/integration/env.sh; source tests/integration/lib/core.sh; \
	  check_tools || echo "(some tools missing — install jq xxd stellar etc.)"; \
	  check_stellar_version || echo "(stellar version may be old)"; \
	  echo "WASM_DIR=$$WASM_DIR"; ls -l $$WASM_DIR/*.wasm 2>/dev/null | head -3 || true; \
	  echo "Preflight complete."'

integration-validate:
	@echo "Validating harness sources (sourcing + basic guards)..."
	@bash -c 'set -u; \
	  for f in tests/integration/env.sh tests/integration/lib/core.sh tests/integration/lib/invoke.sh; do \
	    echo "  sourcing $$f"; bash -n "$$f" || exit 1; \
	  done; \
	  echo "Basic syntax + source validation passed."'

integration-shellcheck:
	@command -v shellcheck >/dev/null 2>&1 || { echo "shellcheck not installed (brew/apt install shellcheck)"; exit 0; }
	@echo "Running shellcheck on harness sources (non-blocking)..."
	@shellcheck -x -s bash tests/integration/env.sh tests/integration/lib/*.sh tests/integration/scenarios/*.sh tests/integration/flows/*.sh 2>&1 | head -30 || true


wasm-artifacts: deploy-artifacts certora-wasm
	@echo ""
	@echo "All WASM artifacts under $(WASM_ARTIFACTS_DIR)/"


CERTORA_PROFILE ?= sanity


certora-list:
	@./certora/scripts/run_profile.py --list


certora: certora-wasm
	@test -n "$$CERTORAKEY" || { echo "CERTORAKEY is not set"; exit 1; }
	@command -v certoraSorobanProver >/dev/null 2>&1 || { \
		echo "certoraSorobanProver not found; install with: pip install certora-cli"; \
		exit 1; \
	}
	@./certora/scripts/run_profile.py $(CERTORA_PROFILE) $(CERTORA_ARGS)

_wasm-manifest:
	@python3 certora/scripts/write_wasm_manifest.py \
		$(if $(DEPLOY),--deploy,) \
		$(if $(CERTORA),--certora,)






# Each test owns its own Soroban Env and its own `test_snapshots` file, so the
# suite parallelises; empty means libtest's default of one thread per core.
# Set TEST_THREADS=1 to serialise while bisecting a cross-test interaction.
TEST_THREADS ?=
TEST_THREAD_FLAG = $(if $(strip $(TEST_THREADS)),--test-threads=$(TEST_THREADS),)


test:
	cargo test -p test-harness -- $(TEST_THREAD_FLAG)


# Serial by default: interleaved output from parallel threads defeats the point.
test-verbose: TEST_THREADS := 1
test-verbose:
	cargo test -p test-harness -- $(TEST_THREAD_FLAG) --nocapture


test-one:
	@[ -n "$(strip $(FILE))" ] || { \
	  echo "test-one requires FILE=<integration test file, without .rs>"; \
	  exit 2; }
	cargo test -p test-harness --test $(FILE) -- $(TEST_THREAD_FLAG)


test-match:
	@[ -n "$(strip $(PATTERN))" ] || { \
	  echo "test-match requires PATTERN=<substring>."; \
	  echo "MATCH= is NOT recognised and silently runs the entire suite."; \
	  exit 2; }
	cargo test -p test-harness $(PATTERN) -- $(TEST_THREAD_FLAG)


test-pool:
	cargo test -p pool



miri-common:
	@cd common && MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-symbolic-alignment-check -Zmiri-disable-isolation" \
		cargo +nightly miri test --lib -- \
		fp_core::tests::test_rescale \
		fp_core::tests::test_div_by_int






miri-all: miri-common








coverage: coverage-merged




define COV_RUN_HARNESS
	backup="$(COV_DIR)/snapshots-backup"; \
	restore_snapshots() { \
		rm -rf $(TEST_HARNESS_DIR)/test_snapshots; \
		mkdir -p $(TEST_HARNESS_DIR)/test_snapshots; \
		cp -R "$$backup"/. $(TEST_HARNESS_DIR)/test_snapshots/ 2>/dev/null || true; \
	}; \
	rm -rf "$$backup" && mkdir -p "$$backup" $(TEST_HARNESS_DIR)/test_snapshots; \
	cp -R $(TEST_HARNESS_DIR)/test_snapshots/. "$$backup"/ 2>/dev/null || true; \
	trap 'restore_snapshots' EXIT; \
	set -o pipefail; \
	cargo llvm-cov test -p test-harness --no-report --no-fail-fast $(COV_IGNORE) -- $(TEST_THREAD_FLAG) 2>&1 | tee $(COV_DIR)/harness.log | tail -20
endef

coverage-controller:
	@echo "Running controller coverage (common + controller unit tests + test-harness)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@set -o pipefail; cargo llvm-cov test -p common --lib --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@set -o pipefail; cargo llvm-cov test -p controller --lib --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@$(COV_RUN_HARNESS)
	@cargo llvm-cov report --lcov --output-path $(COV_DIR)/controller.lcov.info $(COV_IGNORE) >/dev/null
	@python3 scripts/coverage_report.py \
		$(COV_DIR)/controller.lcov.info \
		$(COV_DIR)/controller-report.md \
		controller
	@cp -f $(COV_DIR)/controller.lcov.info lcov.info
	@echo "Reports saved to:"
	@echo "  $(COV_DIR)/controller.lcov.info"
	@echo "  $(COV_DIR)/controller-report.md"
	@echo "  lcov.info  (IDE default; copy of $(COV_DIR)/controller.lcov.info)"

coverage-pool:
	@echo "Running pool coverage (direct pool unit tests)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@set -o pipefail; cargo llvm-cov test -p pool --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov report --lcov --output-path $(COV_DIR)/pool.lcov.info $(COV_IGNORE) >/dev/null
	@python3 scripts/coverage_report.py \
		$(COV_DIR)/pool.lcov.info \
		$(COV_DIR)/pool-report.md \
		pool
	@cp -f $(COV_DIR)/pool.lcov.info lcov.info
	@echo "Reports saved to:"
	@echo "  $(COV_DIR)/pool.lcov.info"
	@echo "  $(COV_DIR)/pool-report.md"
	@echo "  lcov.info  (IDE default; copy of $(COV_DIR)/pool.lcov.info)"

coverage-price-aggregator:
	@echo "Running price-aggregator coverage (common + aggregator unit tests)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@set -o pipefail; cargo llvm-cov test -p common --lib --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@set -o pipefail; cargo llvm-cov test -p price-aggregator --features testing --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov report --lcov --output-path $(COV_DIR)/price-aggregator.lcov.info $(COV_IGNORE) >/dev/null
	@python3 scripts/coverage_report.py \
		$(COV_DIR)/price-aggregator.lcov.info \
		$(COV_DIR)/price-aggregator-report.md \
		price-aggregator
	@cp -f $(COV_DIR)/price-aggregator.lcov.info lcov.info
	@echo "Reports saved to:"
	@echo "  $(COV_DIR)/price-aggregator.lcov.info"
	@echo "  $(COV_DIR)/price-aggregator-report.md"
	@echo "  lcov.info  (IDE default; copy of $(COV_DIR)/price-aggregator.lcov.info)"

coverage-merged:
	@echo "Running merged coverage (common + controller + pool + price-aggregator + test-harness)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@set -o pipefail; cargo llvm-cov test -p common --lib --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@set -o pipefail; cargo llvm-cov test -p pool --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@set -o pipefail; cargo llvm-cov test -p price-aggregator --features testing --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@set -o pipefail; cargo llvm-cov test -p controller --lib --no-report --no-fail-fast $(COV_IGNORE) 2>&1 | tail -5
	@-$(COV_RUN_HARNESS); harness_status=$$?; \
	cargo llvm-cov report --lcov --output-path $(COV_DIR)/merged.lcov.info $(COV_IGNORE) >/dev/null; \
	python3 scripts/coverage_report.py \
		$(COV_DIR)/merged.lcov.info \
		$(COV_DIR)/merged-report.md \
		merged; \
	cp -f $(COV_DIR)/merged.lcov.info lcov.info; \
	echo "Reports saved to:"; \
	echo "  $(COV_DIR)/merged.lcov.info"; \
	echo "  $(COV_DIR)/merged-report.md"; \
	echo "  lcov.info  (IDE default; copy of $(COV_DIR)/merged.lcov.info)"; \
	exit $$harness_status






fmt:
	cargo fmt --all


fmt-check:
	cargo fmt --all -- --check


clippy:
	cargo clippy --all-targets -- -D warnings


clippy-contracts:
	cargo clippy -p controller -p pool -p common -- -D warnings


clippy-fuzz:
	cargo clippy --manifest-path $(FUZZ_DIR)/Cargo.toml --all-targets -- -D warnings


scout:
	.github/scripts/run_scout.sh


scout-host: scout


scout-strict:
	SCOUT_STRICT=1 .github/scripts/run_scout.sh


# Fail the build if any `#[contractimpl]` entrypoint can change state without
# being owner-gated, role-gated, or timelocked, unless it is declared -- with a
# justification -- in scripts/permissionless_entrypoints.txt. Source-only and
# deterministic: no build, no network, runs in about a second.
access-control-check:
	@python3 scripts/check_access_control.py






WASM_BUDGET_FILE ?= configs/wasm_size_budget.txt






# Fail the build if a `testing`-feature-only entrypoint leaked into a deployable
# WASM. We grep only for symbols that are unambiguously test-only: a leak
# co-exports every symbol in the cfg-gated impl, so any one firing catches it.
# `set_price_aggregator` is deliberately NOT grepped — production governance
# references it as a cross-contract invoke target (op.rs `Symbol::new`), so it is
# not a reliable leak-only marker; a governance-testing leak is still caught by
# `set_controller` / `execute_immediate` on the same artifact.
wasm-testing-abi-check: deploy-artifacts
	@gov="$(DEPLOY_DIR)/governance.wasm"; \
	if [ ! -f "$$gov" ]; then echo "governance deploy WASM missing: $$gov"; exit 1; fi; \
	if strings "$$gov" | grep -Eqw 'set_controller|execute_immediate'; then \
		echo "FAIL: governance.wasm exports test-only ABI (set_controller / execute_immediate)"; \
		echo "  The governance/testing feature leaked into the deployable build."; \
		exit 1; \
	fi; \
	echo "OK   governance.wasm exports no test-only ABI"
	@pa="$(DEPLOY_DIR)/price_aggregator.wasm"; \
	if [ ! -f "$$pa" ]; then echo "price-aggregator deploy WASM missing: $$pa"; exit 1; fi; \
	if strings "$$pa" | grep -Eqw 'seed_oracle|seed_oracle_config|remove_oracle'; then \
		echo "FAIL: price_aggregator.wasm exports test-only ABI (seed_oracle / seed_oracle_config / remove_oracle)"; \
		echo "  The price-aggregator/testing feature leaked into the deployable build."; \
		exit 1; \
	fi; \
	echo "OK   price_aggregator.wasm exports no test-only ABI"


wasm-size-check: deploy-artifacts wasm-testing-abi-check
	@if [ ! -f $(WASM_BUDGET_FILE) ]; then \
		echo "WASM budget file missing: $(WASM_BUDGET_FILE)"; \
		echo "Create one with 'path bytes' lines (one per contract)."; \
		exit 1; \
	fi
	@status=0; \
	while IFS=' ' read -r rel_path budget; do \
		case "$$rel_path" in ''|\#*) continue ;; esac; \
		path="$(DEPLOY_DIR)/$$rel_path"; \
		if [ ! -f "$$path" ]; then \
			echo "WASM not built: $$path"; status=1; continue; \
		fi; \
		size=$$(wc -c <"$$path" | tr -d ' '); \
		if [ "$$size" -gt "$$budget" ]; then \
			echo "FAIL $$rel_path  size=$$size bytes  budget=$$budget bytes"; \
			status=1; \
		else \
			echo "OK   $$rel_path  size=$$size bytes  budget=$$budget bytes"; \
		fi; \
	done <$(WASM_BUDGET_FILE); \
	exit $$status












MUTANTS_JOBS ?= 4
CARGO_MUTANTS_VERSION ?= 27.1.0














MUTANTS_TIMEOUT ?= 600


MUTANTS_RUN_MODE ?=

MUTANTS_FILTER ?=

MUTANTS_EXTRA_ARGS ?=


MUTANTS_SHARD ?=

MUTANTS_DIFF_FILE ?= pr.diff






MUTANTS_JOB_ARGS = $(if $(filter --in-place,$(MUTANTS_RUN_MODE)),,-j $(MUTANTS_JOBS))
MUTANTS_SHARD_ARGS = $(if $(MUTANTS_SHARD),--shard $(MUTANTS_SHARD))



MUTANTS_JOBSERVER_TASKS ?=
MUTANTS_JOBSERVER_ARGS = $(if $(MUTANTS_JOBSERVER_TASKS),--jobserver-tasks $(MUTANTS_JOBSERVER_TASKS))

MUTANTS_TEST_TOOL ?=
MUTANTS_TEST_TOOL_ARGS = $(if $(MUTANTS_TEST_TOOL),--test-tool=$(MUTANTS_TEST_TOOL),)
























# Named for the pass they belong to, not for their shape: the shard flag must
# be applied exactly once per target (see run_mutants_two_pass), so a call site
# passing the wrong one is a coverage bug that nothing else catches.
MUTANTS_RUN_ARGS_ITERATE = $(MUTANTS_JOB_ARGS) \
	$(MUTANTS_JOBSERVER_ARGS) $(MUTANTS_TEST_TOOL_ARGS) \
	$(MUTANTS_FILTER) $(MUTANTS_EXTRA_ARGS)
MUTANTS_RUN_ARGS = $(MUTANTS_SHARD_ARGS) $(MUTANTS_RUN_ARGS_ITERATE)
MUTANTS_POOL_WASM := $(abspath $(RELEASE_DIR)/pool.wasm)
MUTANTS_CONTROLLER_WASM := $(abspath $(RELEASE_DIR)/controller.wasm)
MUTANTS_PRICE_AGGREGATOR_WASM := $(abspath $(RELEASE_DIR)/price_aggregator.wasm)




MUTANTS_ENV = PROPTEST_CASES=1 PROPTEST_RNG_SEED=0 \
	POOL_WASM_PATH="$(MUTANTS_POOL_WASM)" \
	CONTROLLER_WASM_PATH="$(MUTANTS_CONTROLLER_WASM)" \
	PRICE_AGGREGATOR_WASM_PATH="$(MUTANTS_PRICE_AGGREGATOR_WASM)"

define run_mutants
	@count=$$(cargo mutants $(1) $(MUTANTS_FILTER) --list | wc -l); \
		[ "$$count" -gt 0 ] || { echo "No mutants matched scope: $(1)"; exit 1; }; \
		echo "Mutation scope: $$count mutants"
	$(MUTANTS_ENV) cargo mutants $(MUTANTS_RUN_MODE) $(1) \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS)
endef














# Sharding rule for every multi-pass macro below: `--shard k/N` partitions the
# mutant list cargo-mutants has just generated, so it must be applied EXACTLY
# ONCE, on the widest pass. The later passes run under `--iterate`, whose list
# is already the previous pass's missed/timeout set -- sharding again there
# would partition an already-partitioned set and silently drop mutants.
#
# Applying it to pass 1 (rather than to the `--iterate` passes, as this used to)
# is also what makes sharding pay: pass 1 compiles and tests every mutant in
# scope and dominates the runtime, so an unsharded pass 1 was duplicated in full
# by every shard. Each shard now owns a slice end to end, and the union over
# shards is the same coverage as an unsharded run.
define run_mutants_two_pass
	@count=$$(cargo mutants $(1) $(MUTANTS_FILTER) --list | wc -l); \
		[ "$$count" -gt 0 ] || { echo "No mutants matched scope: $(1)"; exit 1; }; \
		echo "Mutation scope: $$count mutants (two-pass)"
	@status=0; \
		$(MUTANTS_ENV) GITHUB_ACTIONS=false cargo mutants $(MUTANTS_RUN_MODE) $(1) $(2) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS) \
			|| status=$$?; \
		case $$status in 0|2|3) ;; *) exit $$status;; esac
	@if [ -s mutants.out/missed.txt ] || [ -s mutants.out/timeout.txt ]; then \
		$(MUTANTS_ENV) cargo mutants $(MUTANTS_RUN_MODE) --iterate $(1) $(3) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS_ITERATE); \
	else \
		echo "Full-suite pass skipped: package tests resolved every mutant"; \
	fi
endef







define run_mutants_three_pass
	@count=$$(cargo mutants $(1) $(MUTANTS_FILTER) --list | wc -l); \
		[ "$$count" -gt 0 ] || { echo "No mutants matched scope: $(1)"; exit 1; }; \
		echo "Mutation scope: $$count mutants (three-pass)"
	@status=0; \
		$(MUTANTS_ENV) GITHUB_ACTIONS=false cargo mutants $(MUTANTS_RUN_MODE) $(1) $(2) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS) \
			|| status=$$?; \
		case $$status in 0|2|3) ;; *) exit $$status;; esac
	@if [ -s mutants.out/missed.txt ] || [ -s mutants.out/timeout.txt ]; then \
		status=0; \
		$(MUTANTS_ENV) GITHUB_ACTIONS=false cargo mutants $(MUTANTS_RUN_MODE) --iterate $(1) $(3) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS_ITERATE) \
			|| status=$$?; \
		case $$status in 0|2|3) ;; *) exit $$status;; esac; \
	else \
		echo "Native-consumer pass skipped: package tests resolved every mutant"; \
	fi
	@if [ -s mutants.out/missed.txt ] || [ -s mutants.out/timeout.txt ]; then \
		$(MUTANTS_ENV) cargo mutants $(MUTANTS_RUN_MODE) --iterate $(1) $(4) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS_ITERATE); \
	else \
		echo "Integration pass skipped: native tests resolved every mutant"; \
	fi
endef

_mutants-check:
	@command -v cargo-mutants >/dev/null 2>&1 || { \
		echo "cargo-mutants not installed. Install with:"; \
		echo "  cargo install cargo-mutants --version $(CARGO_MUTANTS_VERSION) --locked"; \
		exit 1; \
	}
	@INSTALLED=$$(cargo mutants --version | awk '{print $$2}'); \
	if [ "$$INSTALLED" != "$(CARGO_MUTANTS_VERSION)" ]; then \
		echo "cargo-mutants $$INSTALLED installed but $(CARGO_MUTANTS_VERSION) pinned (mutant generation can differ across versions). Install with:"; \
		echo "  cargo install cargo-mutants --version $(CARGO_MUTANTS_VERSION) --locked"; \
		exit 1; \
	fi













MUTANTS_FIXTURES_PREBUILT ?=

_mutants-harness-prepare: _mutants-check
ifeq ($(MUTANTS_FIXTURES_PREBUILT),)
	rm -rf $(CARGO_TARGET_DIR)/$(WASM_TARGET)
	$(MAKE) build
else
	@for w in "$(MUTANTS_POOL_WASM)" "$(MUTANTS_CONTROLLER_WASM)" "$(MUTANTS_PRICE_AGGREGATOR_WASM)"; do \
		[ -s "$$w" ] || { echo "prebuilt fixture missing or empty: $$w"; exit 1; }; \
	done
	@[ -s "$(RELEASE_DIR)/SHA256SUMS" ] \
		|| { echo "prebuilt fixtures have no SHA256SUMS manifest in $(RELEASE_DIR)"; exit 1; }
	@cd $(RELEASE_DIR) && { \
		if command -v sha256sum >/dev/null 2>&1; then sha256sum -c SHA256SUMS; \
		else shasum -a 256 -c SHA256SUMS; fi; } >/dev/null \
		|| { echo "prebuilt fixture checksums do not match SHA256SUMS"; exit 1; }
	@echo "Using prebuilt wasm fixtures from $(RELEASE_DIR) (checksums verified)"
endif
	@grep -aq set_swap_aggregator "$(MUTANTS_CONTROLLER_WASM)" \
		|| { echo "controller.wasm fixture is stale (missing set_swap_aggregator export)"; exit 1; }


mutants: mutants-common mutants-pool mutants-governance \
		 mutants-controller-core \
         mutants-controller-oracle mutants-controller-positions \
         mutants-controller-strategies mutants-controller-views \
         mutants-aggregator mutants-oracle-adapter mutants-defindex-strategy \
         mutants-swap-aggregator


mutants-math: _mutants-check
	$(call run_mutants,--package common --file 'common/src/math/**')

mutants-rates: _mutants-check
	$(call run_mutants,--package common --file 'common/src/rates/**')

mutants-pool-interest: _mutants-check
	$(call run_mutants,--package pool --file 'contracts/pool/src/interest.rs')





mutants-common: _mutants-harness-prepare
	$(call run_mutants_three_pass,--package common,\
		--test-package common,\
		--test-package common --test-package controller --test-package pool \
		--test-package governance,\
		--test-package common --test-package controller --test-package pool \
		--test-package governance --test-package test-harness)



mutants-pool: _mutants-check
	$(call run_mutants,--package pool --test-package pool)

mutants-governance: _mutants-harness-prepare


	$(call run_mutants,--package governance \
		--test-package governance)



CONTROLLER_FAST_TESTS = --test-package controller
CONTROLLER_FULL_TESTS = --test-package controller --test-package governance \
	--test-package test-harness


mutants-controller-core: _mutants-harness-prepare
	$(call run_mutants_two_pass,--package controller --file 'contracts/controller/src/**' \
		--exclude 'contracts/controller/src/context/oracle.rs' \
		--exclude 'contracts/controller/src/positions/**' \
		--exclude 'contracts/controller/src/strategies/**' \
		--exclude 'contracts/controller/src/views.rs',\
		$(CONTROLLER_FAST_TESTS),$(CONTROLLER_FULL_TESTS))

mutants-controller-oracle: _mutants-harness-prepare
	$(call run_mutants_two_pass,--package controller --file 'contracts/controller/src/context/oracle.rs',\
		$(CONTROLLER_FAST_TESTS),$(CONTROLLER_FULL_TESTS))

mutants-controller-positions: _mutants-harness-prepare
	$(call run_mutants_two_pass,--package controller --file 'contracts/controller/src/positions/**',\
		$(CONTROLLER_FAST_TESTS),$(CONTROLLER_FULL_TESTS))

mutants-controller-strategies: _mutants-harness-prepare
	$(call run_mutants_two_pass,--package controller --file 'contracts/controller/src/strategies/**',\
		$(CONTROLLER_FAST_TESTS),$(CONTROLLER_FULL_TESTS))

mutants-controller-views: _mutants-harness-prepare
	$(call run_mutants_two_pass,--package controller --file 'contracts/controller/src/views.rs',\
		$(CONTROLLER_FAST_TESTS),$(CONTROLLER_FULL_TESTS))






mutants-diff: _mutants-harness-prepare
	@[ -s "$(MUTANTS_DIFF_FILE)" ] || { echo "Empty diff; nothing to mutate."; exit 0; }


	$(MUTANTS_ENV) cargo mutants $(MUTANTS_RUN_MODE) --in-diff "$(MUTANTS_DIFF_FILE)" \
		--test-workspace true \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) \
		$(MUTANTS_JOB_ARGS) $(MUTANTS_SHARD_ARGS) $(MUTANTS_JOBSERVER_ARGS) \
		$(MUTANTS_TEST_TOOL_ARGS) $(MUTANTS_EXTRA_ARGS)





mutants-aggregator: _mutants-check
	$(call run_mutants,--package price-aggregator --test-package price-aggregator --features testing)

mutants-oracle-adapter: _mutants-check
	$(call run_mutants,--package xoxno-oracle --test-package xoxno-oracle)

mutants-swap-aggregator: _mutants-check
	$(call run_mutants,--package swap-aggregator --test-package swap-aggregator)





mutants-defindex-strategy: _mutants-harness-prepare
	$(call run_mutants,--package defindex-strategy --test-package defindex-strategy)






clean:
	cargo clean
	rm -rf $(OPTIMIZED_DIR)
	rm -rf $(WASM_ARTIFACTS_DIR)
	rm -rf $(CERTORA_BUILD_DIR)
	rm -rf $(COV_DIR)








install-stellar-cli:
	STELLAR_VERSION=27.0.0 bash .github/scripts/install-stellar-cli.sh





FUZZ_TARGETS := fp_math rates_and_index fp_ops
FUZZ_CONTRACT_TARGETS := flow_e2e flow_strategy pool_native aggregator
FUZZ_TIME ?= 60
FUZZ_MAX_LEN ?= 256
FUZZ_LEN_CONTROL ?= 0




UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
  FUZZ_FLAGS := --sanitizer=thread -Zbuild-std
else




  FUZZ_HOST = $(shell rustc -vV | sed -n 's/^host: //p')
  FUZZ_FLAGS = --target $(FUZZ_HOST)
endif


fuzz:
	@set -o pipefail; for t in $(FUZZ_TARGETS); do \
		echo "=== $$t ==="; \
		mkdir -p $(FUZZ_DIR)/corpus/$$t; \
		cargo +nightly fuzz run --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS) $$t $(FUZZ_DIR)/corpus/$$t $(FUZZ_DIR)/seeds/$$t -- -max_total_time=$(FUZZ_TIME) -max_len=$(FUZZ_MAX_LEN) -len_control=$(FUZZ_LEN_CONTROL) 2>&1 | tee /tmp/fuzz-$$t.log | tail -3 || { echo "::error::fuzz $$t crashed:"; tail -80 /tmp/fuzz-$$t.log; exit 1; }; \
	done


fuzz-contract:
	@set -o pipefail; for t in $(FUZZ_CONTRACT_TARGETS); do \
		echo "=== $$t ==="; \
		mkdir -p $(FUZZ_DIR)/corpus/$$t; \
		cargo +nightly fuzz run --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS) $$t $(FUZZ_DIR)/corpus/$$t $(FUZZ_DIR)/seeds/$$t -- -max_total_time=$(FUZZ_TIME) -max_len=$(FUZZ_MAX_LEN) -len_control=$(FUZZ_LEN_CONTROL) 2>&1 | tee /tmp/fuzz-$$t.log | tail -3 || { echo "::error::fuzz $$t crashed:"; tail -80 /tmp/fuzz-$$t.log; exit 1; }; \
	done


fuzz-one:
	@mkdir -p $(FUZZ_DIR)/corpus/$(TARGET)
	@cargo +nightly fuzz run --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS) $(TARGET) $(FUZZ_DIR)/corpus/$(TARGET) $(FUZZ_DIR)/seeds/$(TARGET) -- -max_total_time=$(FUZZ_TIME) -max_len=$(FUZZ_MAX_LEN) -len_control=$(FUZZ_LEN_CONTROL)


fuzz-build:
	@cargo +nightly fuzz build --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS)



fuzz-seed-corpus:
	@cd $(FUZZ_DIR) && cargo run --release --features seed-corpus --bin seed_corpus -- --output corpus












FUZZ_COV_TIME ?= 0
ifeq ($(UNAME_S),Darwin)
  FUZZ_COV_ENV := SANITIZER=thread BUILD_STD=1
else
  FUZZ_COV_ENV :=
endif


fuzz-coverage:
	@$(FUZZ_COV_ENV) FUZZ_COV_TIME=$(FUZZ_COV_TIME) FUZZ_MAX_LEN=$(FUZZ_MAX_LEN) FUZZ_LEN_CONTROL=$(FUZZ_LEN_CONTROL) \
		./$(FUZZ_DIR)/coverage.sh $(FUZZ_TARGETS)


fuzz-coverage-all:
	@$(FUZZ_COV_ENV) FUZZ_COV_TIME=$(FUZZ_COV_TIME) FUZZ_MAX_LEN=$(FUZZ_MAX_LEN) FUZZ_LEN_CONTROL=$(FUZZ_LEN_CONTROL) \
		./$(FUZZ_DIR)/coverage.sh $(FUZZ_TARGETS) $(FUZZ_CONTRACT_TARGETS)


fuzz-coverage-one:
	@if [ -z "$(TARGET)" ]; then \
		echo "Usage: make fuzz-coverage-one TARGET=<name> [FUZZ_COV_TIME=30]"; \
		exit 1; \
	fi
	@$(FUZZ_COV_ENV) FUZZ_COV_TIME=$(FUZZ_COV_TIME) FUZZ_MAX_LEN=$(FUZZ_MAX_LEN) FUZZ_LEN_CONTROL=$(FUZZ_LEN_CONTROL) \
		./$(FUZZ_DIR)/coverage.sh $(TARGET)


fuzz-coverage-clean:
	@rm -rf $(COV_DIR)/fuzz $(FUZZ_DIR)/coverage





PROPTEST_CASES ?=
PROPTEST_ENV = $(if $(strip $(PROPTEST_CASES)),PROPTEST_CASES=$(PROPTEST_CASES),)



proptest:
	@echo "=== fuzz (proptest) ==="
	@$(PROPTEST_ENV) cargo test --release -p test-harness --test fuzz -- --test-threads=1


proptest-one:
	@[ -n "$(strip $(TEST))" ] || { \
	  echo "proptest-one requires TEST=<substring>; omitting it runs the whole fuzz suite."; \
	  echo "Use 'make proptest' if that is what you want."; \
	  exit 2; }
	@$(PROPTEST_ENV) cargo test --release -p test-harness --test fuzz $(TEST) -- --test-threads=1


proptest-build:
	@cargo build --release --tests -p test-harness






keygen:
	@echo "Generating deployer key for $(NETWORK)..."
	stellar keys generate deployer --network $(NETWORK) --fund
	@echo "Deployer address:"
	@stellar keys public-key deployer

_preflight-tools:
	@command -v stellar >/dev/null 2>&1 || { echo "Missing required tool: stellar"; exit 1; }
	@command -v jq >/dev/null 2>&1 || { echo "Missing required tool: jq"; exit 1; }

_preflight-network-config: _preflight-tools
	@test -f $(CONFIG_DIR)/networks.json || { echo "Config file not found: $(CONFIG_DIR)/networks.json"; exit 1; }
	@jq -e '.["$(NETWORK)"] != null' $(CONFIG_DIR)/networks.json >/dev/null || { echo "Network $(NETWORK) not found in $(CONFIG_DIR)/networks.json"; exit 1; }
	@test -f $(CONFIG_DIR)/$(NETWORK)/markets.json || { echo "Config file not found: $(CONFIG_DIR)/$(NETWORK)/markets.json"; exit 1; }
	@jq -e '.markets | type == "array" and length > 0' $(CONFIG_DIR)/$(NETWORK)/markets.json >/dev/null || { echo "No configured markets in $(CONFIG_DIR)/$(NETWORK)/markets.json"; exit 1; }
	@jq -e 'all(.markets[]; (.name // "") != "" and (.asset_address // "") != "")' $(CONFIG_DIR)/$(NETWORK)/markets.json >/dev/null || { echo "Every configured market must have name and asset_address"; exit 1; }
	@jq -e 'any(.markets[]; .enabled != false)' $(CONFIG_DIR)/$(NETWORK)/markets.json >/dev/null || { echo "All markets have enabled=false in $(CONFIG_DIR)/$(NETWORK)/markets.json; nothing to deploy"; exit 1; }
	@test -f $(CONFIG_DIR)/$(NETWORK)/spokes.json || { echo "Config file not found: $(CONFIG_DIR)/$(NETWORK)/spokes.json"; exit 1; }
	@jq -e 'type == "object"' $(CONFIG_DIR)/$(NETWORK)/spokes.json >/dev/null || { echo "Spoke config in $(CONFIG_DIR)/$(NETWORK)/spokes.json is not a JSON object"; exit 1; }




_preflight-setup: _preflight-network-config _preflight-validate-configs
	@AGG=$$(jq -r '.["$(NETWORK)"].aggregator // empty' $(CONFIG_DIR)/networks.json); \
	if [ -n "$${AGGREGATOR_CONTRACT:-}" ]; then AGG="$$AGGREGATOR_CONTRACT"; fi; \
	if [ -z "$$AGG" ] || [ "$$AGG" = "null" ]; then \
		if [ "$${ALLOW_MISSING_AGGREGATOR:-0}" = "1" ]; then \
			echo "WARNING: aggregator not configured for $(NETWORK); continuing (ALLOW_MISSING_AGGREGATOR=1). Strategies stay broken until setAggregator runs."; \
		else \
			echo "Aggregator not configured for $(NETWORK). Set $(CONFIG_DIR)/networks.json aggregator or AGGREGATOR_CONTRACT=<addr>."; \
			echo "To deliberately proceed without one, set ALLOW_MISSING_AGGREGATOR=1."; \
			exit 1; \
		fi; \
	fi; \
	ACC=$$(jq -r '.["$(NETWORK)"].accumulator // empty' $(CONFIG_DIR)/networks.json); \
	if [ -n "$${ACCUMULATOR_CONTRACT:-}" ]; then ACC="$$ACCUMULATOR_CONTRACT"; fi; \
	if [ -z "$$ACC" ] || [ "$$ACC" = "null" ]; then \
		if [ "$${ALLOW_MISSING_ACCUMULATOR:-0}" = "1" ]; then \
			echo "WARNING: accumulator not configured for $(NETWORK); continuing (ALLOW_MISSING_ACCUMULATOR=1). claimRevenue fails with NoAccumulator (#211) until setAccumulator runs."; \
		else \
			echo "Accumulator not configured for $(NETWORK). Set $(CONFIG_DIR)/networks.json accumulator or ACCUMULATOR_CONTRACT=<treasury-wallet>."; \
			echo "claimRevenue fails with NoAccumulator (#211) until setAccumulator runs."; \
			echo "To deliberately proceed without one, set ALLOW_MISSING_ACCUMULATOR=1."; \
			exit 1; \
		fi; \
	fi

_preflight-controller: _preflight-network-config
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$CTRL" ]; then \
		CTRL=$$(jq -r '.["$(NETWORK)"].controller // empty' $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$CTRL" ] || [ "$$CTRL" = "null" ]; then \
		echo "Controller not configured for $(NETWORK). Deploy first or set configs/networks.json."; \
		exit 1; \
	fi

_preflight-governance: _preflight-network-config
	@GOV=$$(stellar contract alias show governance --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$GOV" ]; then \
		GOV=$$(jq -r '.["$(NETWORK)"].governance // empty' $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$GOV" ] || [ "$$GOV" = "null" ]; then \
		echo "Governance not configured for $(NETWORK). Deploy first or set configs/networks.json."; \
		exit 1; \
	fi

_preflight-pool-hash: _preflight-network-config
	@HASH=$$(if [ -s $(POOL_WASM_HASH_FILE) ]; then cat $(POOL_WASM_HASH_FILE); else jq -r '.["$(NETWORK)"].pool_wasm_hash // empty' $(CONFIG_DIR)/networks.json; fi); \
	if [ -z "$$HASH" ] || [ "$$HASH" = "null" ]; then \
		echo "Pool WASM hash not found. Run deploy first or set configs/networks.json."; \
		exit 1; \
	fi



_preflight-validate-configs: _preflight-network-config
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh validateConfigs

_preflight-configure-controller: _preflight-setup _preflight-controller _preflight-governance

_preflight-upgrade-pool: _preflight-controller _preflight-governance _preflight-pool-hash

_post-setup-status:
	@echo ""
	@echo "=== Setup status ($(NETWORK)) ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh info
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh listMarkets
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh listSpokes
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh checkDelay


deploy-testnet: NETWORK=testnet
deploy-testnet: _deploy

deploy-mainnet: NETWORK=mainnet
deploy-mainnet: _deploy


upgrade-controller: _preflight-controller _preflight-governance deploy-artifacts
	@echo "=== Upgrading controller on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@GOV=$$(stellar contract alias show governance --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$GOV" ]; then \
		GOV=$$(jq -r '.["$(NETWORK)"].governance // empty' $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$GOV" ] || [ "$$GOV" = "null" ]; then \
		echo "Governance alias not found on $(NETWORK)"; \
		exit 1; \
	fi; \
	stellar contract upload \
		--wasm $(DEPLOY_DIR)/controller.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(CONTROLLER_WASM_HASH_FILE); \
	HASH=$$(cat $(CONTROLLER_WASM_HASH_FILE)); \
	echo "Governance: $$GOV"; \
	echo "New controller WASM hash: $$HASH"
	@HASH=$$(cat $(CONTROLLER_WASM_HASH_FILE)); \
	NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh upgradeControllerHash $$HASH
	@
	@
	@HASH=$$(cat $(CONTROLLER_WASM_HASH_FILE)); \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].controller_wasm_hash = "'$$HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json


upgrade-governance: _preflight-governance deploy-artifacts
	@echo "=== Upgrading governance on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@GOV=$$(stellar contract alias show governance --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$GOV" ]; then \
		GOV=$$(jq -r '.["$(NETWORK)"].governance // empty' $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$GOV" ] || [ "$$GOV" = "null" ]; then \
		echo "Governance alias not found on $(NETWORK)"; \
		exit 1; \
	fi; \
	stellar contract upload \
		--wasm $(DEPLOY_DIR)/governance.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(GOVERNANCE_WASM_HASH_FILE); \
	HASH=$$(cat $(GOVERNANCE_WASM_HASH_FILE)); \
	echo "Governance: $$GOV"; \
	echo "New governance WASM hash: $$HASH"
	@HASH=$$(cat $(GOVERNANCE_WASM_HASH_FILE)); \
	NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh upgradeGovernanceHash $$HASH
	@
	@
	@HASH=$$(cat $(GOVERNANCE_WASM_HASH_FILE)); \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].governance_wasm_hash = "'$$HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json



upgrade-pool: _preflight-controller _preflight-governance deploy-artifacts
	@echo "=== Upgrading central pool on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@GOV=$$(stellar contract alias show governance --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$GOV" ]; then \
		GOV=$$(jq -r ".\"$(NETWORK)\".governance // empty" $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$GOV" ] || [ "$$GOV" = "null" ]; then \
		echo "Governance not found for $(NETWORK)"; \
		exit 1; \
	fi; \
	stellar contract upload \
		--wasm $(DEPLOY_DIR)/pool.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(POOL_UPGRADE_WASM_HASH_FILE); \
	HASH=$$(cat $(POOL_UPGRADE_WASM_HASH_FILE)); \
	echo "Governance: $$GOV"; \
	echo "New pool WASM hash: $$HASH"
	@HASH=$$(cat $(POOL_UPGRADE_WASM_HASH_FILE)); \
	NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh upgradePoolHash $$HASH
	@
	@HASH=$$(cat $(POOL_UPGRADE_WASM_HASH_FILE)); \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].pool_wasm_hash = "'$$HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json


upgrade-all: upgrade-pool upgrade-controller _unpause-after-setup _post-setup-status






prepay-rent:
	@echo "=== Prepaying protocol rent on $(NETWORK) ==="
	@mkdir -p target
	@CFG=target/keeper-prepay-$(NETWORK).yaml; \
	RPC=$$(jq -r '.["$(NETWORK)"].rpc_url' $(CONFIG_DIR)/networks.json); \
	PASS=$$(jq -r '.["$(NETWORK)"].network_passphrase' $(CONFIG_DIR)/networks.json); \
	CTRL=$$(jq -r '.["$(NETWORK)"].controller' $(CONFIG_DIR)/networks.json); \
	GOV=$$(jq -r '.["$(NETWORK)"].governance' $(CONFIG_DIR)/networks.json); \
	HASH=$$(jq -r '.["$(NETWORK)"].pool_wasm_hash' $(CONFIG_DIR)/networks.json); \
	FLR=$$(jq -r '.["$(NETWORK)"].flash_loan_receiver // empty' $(CONFIG_DIR)/networks.json); \
	PAGG=$$(jq -r '.["$(NETWORK)"].price_aggregator // empty' $(CONFIG_DIR)/networks.json); \
	OADP=$$(jq -r '.["$(NETWORK)"].xoxno_oracle_adapter // empty' $(CONFIG_DIR)/networks.json); \
	{ echo "network: $(NETWORK)"; \
	  echo "rpc:"; \
	  echo "  url: $$RPC"; \
	  echo "  passphrase: \"$$PASS\""; \
	  echo "  timeout_seconds: 30"; \
	  echo "contracts:"; \
	  echo "  controller: $$CTRL"; \
	  echo "  pool_wasm_hash: $$HASH"; \
	  echo "  markets:"; \
	  jq -r '.markets[] | "    - { hub_id: \(.hub_id), asset: \(.asset_address) }"' $(CONFIG_DIR)/$(NETWORK)/markets.json; \
	  echo "  market_assets: []"; \
	  echo "  flash_loan_receiver: $$FLR"; \
	  echo "  governance: $$GOV"; \
	  echo "  price_aggregator: \"$$PAGG\""; \
	  echo "  xoxno_oracle_adapter: \"$$OADP\""; \
	  echo "keyvault:"; \
	  echo "  url: https://unused.vault.azure.net"; \
	  echo "  secret_name: unused"; \
	  echo "signer:"; \
	  echo "  derivation_path: \"m/44'/148'/0'\""; \
	  echo "fees:"; \
	  echo "  base_fee_stroops: 100"; \
	  echo "  resource_fee_multiplier: 1.20"; \
	  echo "schedule:"; \
	  echo "  ttl_tick_seconds: 21600"; \
	  echo "  index_tick_seconds: 3600"; \
	  echo "  ttl_safety_margin_days: 14"; \
	  echo "  asset_chunk: 20"; \
	  echo "  max_txs_per_tick: 50"; \
	  echo "  enable_index_refresh: false"; \
	  echo "metrics:"; \
	  echo "  bind: 0.0.0.0:9090"; \
	  echo "log:"; \
	  echo "  level: info"; \
	  echo "  format: json"; \
	} > $$CFG; \
	PREPAY_SECRET=$$(stellar keys show $(SIGNER)); \
	export PREPAY_SECRET; \
	cargo run --manifest-path services/keeper/Cargo.toml --bin prepay_rent -- --config $$CFG


build-aggregator:
	@echo "Building aggregator..."
	@stellar contract build --package swap-aggregator
	@mkdir -p $(DEPLOY_DIR)
	@if command -v stellar &>/dev/null; then \
		stellar contract optimize \
			--wasm $(RELEASE_DIR)/swap_aggregator.wasm \
			--wasm-out $(DEPLOY_DIR)/aggregator.wasm 2>/dev/null || \
		cp $(RELEASE_DIR)/swap_aggregator.wasm $(DEPLOY_DIR)/aggregator.wasm; \
	else \
		cp $(RELEASE_DIR)/swap_aggregator.wasm $(DEPLOY_DIR)/aggregator.wasm; \
	fi
	@ls -lh $(DEPLOY_DIR)/aggregator.wasm




deploy-aggregator: build-aggregator
	@echo "=== Deploying aggregator on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@ADMIN=$${AGGREGATOR_ADMIN:-$(SIGNER_ADDRESS)}; \
	echo "Admin: $$ADMIN"; \
	stellar contract deploy \
		--wasm $(DEPLOY_DIR)/aggregator.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) \
		--alias aggregator \
		-- --admin $$ADMIN > target/aggregator_id.txt
	@AGG=$$(tail -n1 target/aggregator_id.txt); \
	echo "Aggregator: $$AGG"; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].aggregator = "'$$AGG'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json


build-oracle-adapter:
	@echo "Building xoxno-oracle-adapter..."
	@stellar contract build --package xoxno-oracle
	@mkdir -p $(DEPLOY_DIR)
	@if command -v stellar &>/dev/null; then \
		stellar contract optimize \
			--wasm $(RELEASE_DIR)/xoxno_oracle.wasm \
			--wasm-out $(DEPLOY_DIR)/xoxno-oracle-adapter.wasm 2>/dev/null || \
		cp $(RELEASE_DIR)/xoxno_oracle.wasm $(DEPLOY_DIR)/xoxno-oracle-adapter.wasm; \
	else \
		cp $(RELEASE_DIR)/xoxno_oracle.wasm $(DEPLOY_DIR)/xoxno-oracle-adapter.wasm; \
	fi
	@ls -lh $(DEPLOY_DIR)/xoxno-oracle-adapter.wasm








deploy-oracle-adapter: build-oracle-adapter
	@echo "=== Deploying xoxno-oracle-adapter on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@ADMIN=$${ORACLE_ADAPTER_ADMIN:-$(SIGNER_ADDRESS)}; \
	SIGNERS=$${ORACLE_ADAPTER_SIGNERS:-'["'$(SIGNER_ADDRESS)'"]'}; \
	echo "Admin: $$ADMIN"; \
	echo "Signers: $$SIGNERS"; \
	echo "Threshold: $(ORACLE_ADAPTER_THRESHOLD)"; \
	echo "Resolution: $(ORACLE_ADAPTER_RESOLUTION)"; \
	stellar contract deploy \
		--wasm $(DEPLOY_DIR)/xoxno-oracle-adapter.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) \
		--alias xoxno-oracle-adapter \
		-- --admin $$ADMIN --signers "$$SIGNERS" --threshold $(ORACLE_ADAPTER_THRESHOLD) --resolution $(ORACLE_ADAPTER_RESOLUTION) > target/oracle_adapter_id.txt
	@ORA=$$(tail -n1 target/oracle_adapter_id.txt); \
	echo "Oracle adapter: $$ORA"; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].xoxno_oracle_adapter = "'$$ORA'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json




upgrade-aggregator: build-aggregator
	@echo "=== Upgrading aggregator on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/aggregator.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > target/aggregator_wasm_hash.txt
	@HASH=$$(cat target/aggregator_wasm_hash.txt); \
	echo "New aggregator WASM hash: $$HASH"; \
	NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh upgradeAggregatorHash $$HASH







upgrade-oracle-adapter: build-oracle-adapter
	@echo "=== Upgrading xoxno-oracle-adapter on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/xoxno-oracle-adapter.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > target/oracle_adapter_wasm_hash.txt
	@HASH=$$(cat target/oracle_adapter_wasm_hash.txt); \
	echo "New oracle adapter WASM hash: $$HASH"; \
	NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh upgradeOracleAdapterHash $$HASH












upgrade-oracle-adapter-full: upgrade-oracle-adapter
	@echo "=== Finalizing oracle adapter upgrade on $(NETWORK) (signer=$(SIGNER)) ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh finalizeOracleAdapterUpgrade


build-flash-loan-receiver:
	@echo "Building flash-loan receiver..."
	@stellar contract build --package flash-loan-receiver
	@mkdir -p $(DEPLOY_DIR)
	@if command -v stellar &>/dev/null; then \
		stellar contract optimize \
			--wasm $(RELEASE_DIR)/flash_loan_receiver.wasm \
			--wasm-out $(DEPLOY_DIR)/flash-loan-receiver.wasm 2>/dev/null || \
		cp $(RELEASE_DIR)/flash_loan_receiver.wasm $(DEPLOY_DIR)/flash-loan-receiver.wasm; \
	else \
		cp $(RELEASE_DIR)/flash_loan_receiver.wasm $(DEPLOY_DIR)/flash-loan-receiver.wasm; \
	fi
	@ls -lh $(DEPLOY_DIR)/flash-loan-receiver.wasm


deploy-flash-loan-receiver: build-flash-loan-receiver
	@echo "=== Deploying flash-loan receiver on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@stellar contract deploy \
		--wasm $(DEPLOY_DIR)/flash-loan-receiver.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) \
		--alias flash-loan-receiver > target/flash_loan_receiver_id.txt
	@RECEIVER=$$(tail -n1 target/flash_loan_receiver_id.txt); \
	echo "Flash receiver: $$RECEIVER"; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].flash_loan_receiver = "'$$RECEIVER'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json


fund-flash-loan-receiver:
	@echo "=== Funding flash-loan receiver on $(NETWORK) ==="
	@ASSET=$$(jq -r '.markets[] | select(.name == "$(FLASH_MARKET)") | .asset_address' $(CONFIG_DIR)/$(NETWORK)/markets.json); \
	RECEIVER=$$(stellar contract alias show flash-loan-receiver --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$RECEIVER" ]; then \
		RECEIVER=$$(jq -r ".\"$(NETWORK)\".flash_loan_receiver // empty" $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$ASSET" ] || [ "$$ASSET" = "null" ]; then \
		echo "Unknown FLASH_MARKET=$(FLASH_MARKET) for $(NETWORK)"; \
		exit 1; \
	fi; \
	if [ -z "$$RECEIVER" ] || [ "$$RECEIVER" = "null" ]; then \
		echo "Flash receiver not found. Run deploy-flash-loan-receiver first."; \
		exit 1; \
	fi; \
	echo "Asset: $$ASSET ($(FLASH_MARKET))"; \
	echo "Receiver: $$RECEIVER"; \
	echo "Amount: $(FLASH_RECEIVER_FUND)"; \
	stellar contract invoke --id $$ASSET $(SOURCE_FLAG) --network $(NETWORK) \
		-- transfer --from $(SIGNER_ADDRESS) --to $$RECEIVER --amount $(FLASH_RECEIVER_FUND)


test-flash-loan-receiver:
	@echo "=== Flash-loan receiver smoke test on $(NETWORK) ==="
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$CTRL" ]; then \
		CTRL=$$(jq -r ".\"$(NETWORK)\".controller // empty" $(CONFIG_DIR)/networks.json); \
	fi; \
	ASSET=$$(jq -r '.markets[] | select(.name == "$(FLASH_MARKET)") | .asset_address' $(CONFIG_DIR)/$(NETWORK)/markets.json); \
	HUB_ID=$$(jq -r '.markets[] | select(.name == "$(FLASH_MARKET)") | .hub_id' $(CONFIG_DIR)/$(NETWORK)/markets.json); \
	RECEIVER=$$(stellar contract alias show flash-loan-receiver --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$RECEIVER" ]; then \
		RECEIVER=$$(jq -r ".\"$(NETWORK)\".flash_loan_receiver // empty" $(CONFIG_DIR)/networks.json); \
	fi; \
	if [ -z "$$CTRL" ] || [ "$$CTRL" = "null" ]; then \
		echo "Controller not found for $(NETWORK)"; \
		exit 1; \
	fi; \
	if [ -z "$$ASSET" ] || [ "$$ASSET" = "null" ]; then \
		echo "Unknown FLASH_MARKET=$(FLASH_MARKET) for $(NETWORK)"; \
		exit 1; \
	fi; \
	if [ -z "$$HUB_ID" ] || [ "$$HUB_ID" = "null" ]; then \
		echo "FLASH_MARKET=$(FLASH_MARKET) missing hub_id for $(NETWORK)"; \
		exit 1; \
	fi; \
	if [ -z "$$RECEIVER" ] || [ "$$RECEIVER" = "null" ]; then \
		echo "Flash receiver not found. Run deploy-flash-loan-receiver first."; \
		exit 1; \
	fi; \
	echo "Controller: $$CTRL"; \
	echo "Receiver: $$RECEIVER"; \
	HUB_ASSET=$$(jq -nc --argjson hub_id "$$HUB_ID" --arg asset "$$ASSET" '{hub_id:$$hub_id, asset:$$asset}'); \
	echo "Asset: $$HUB_ASSET ($(FLASH_MARKET))"; \
	echo "Loan amount: $(FLASH_LOAN_AMOUNT)"; \
	run_data_case() { \
		local name="$$1"; \
		local expected="$$2"; \
		local data="$$3"; \
		local log; \
		log="target/flash_loan_$${name}.log"; \
		echo "Running $$name (expected $$expected)..."; \
		if stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
			-- flash_loan \
			--caller $(SIGNER_ADDRESS) \
		--asset "$$HUB_ASSET" \
			--amount $(FLASH_LOAN_AMOUNT) \
			--receiver $$RECEIVER \
			--data $$data > "$$log" 2>&1; then \
			if [ "$$expected" = "success" ]; then \
				echo "PASS $$name"; \
				tail -n 6 "$$log"; \
			else \
				echo "FAIL $$name unexpectedly succeeded"; \
				cat "$$log"; \
				exit 1; \
			fi; \
		else \
			if [ "$$expected" = "failure" ]; then \
				echo "PASS $$name rejected"; \
				tail -n 8 "$$log"; \
			else \
				echo "FAIL $$name unexpectedly failed"; \
				cat "$$log"; \
				exit 1; \
			fi; \
		fi; \
	}; \
	run_case() { \
		local mode="$$1"; \
		local expected="$$2"; \
		local data; \
		data=$$(cargo run -q -p flash-loan-receiver --example encode_request -- "$$mode"); \
		run_data_case "$$mode" "$$expected" "$$data"; \
	}; \
	run_case Success success; \
	run_case NoRepay failure; \
	run_case UnderRepay failure; \
	run_case ReenterPoolFlashLoan failure; \
	run_case ReenterControllerSupply failure; \
	run_case Panic failure; \
	run_data_case InvalidData failure 00; \
	run_case Success success

_deploy: deploy-artifacts
	@echo "=== Deploying to $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@echo ""
	@echo "1/8 Checking Swap Aggregator..."
	@AGGREGATOR=$$(jq -r ".\"$(NETWORK)\".aggregator // empty" $(CONFIG_DIR)/networks.json 2>/dev/null); \
	if [ -n "$${AGGREGATOR_CONTRACT:-}" ]; then AGGREGATOR="$$AGGREGATOR_CONTRACT"; fi; \
	if [ -n "$$AGGREGATOR" ] && [ "$$AGGREGATOR" != "null" ]; then \
		echo "Using Aggregator: $$AGGREGATOR"; \
		stellar contract alias add aggregator --id $$AGGREGATOR --network $(NETWORK) --overwrite || echo "Warning: Failed to set aggregator alias"; \
	else \
		echo "Skipping Aggregator alias (set networks.json aggregator or AGGREGATOR_CONTRACT before configure-controller)"; \
	fi
	@echo ""
	@
	@echo "2/8 Uploading pool WASM..."
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/pool.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(POOL_WASM_HASH_FILE)
	@echo "Pool WASM hash: $$(cat $(POOL_WASM_HASH_FILE))"
	@POOL_HASH=$$(cat $(POOL_WASM_HASH_FILE)); \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].pool_wasm_hash = "'$$POOL_HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@
	@echo "3/8 Uploading Controller WASM..."
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/controller.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(CONTROLLER_WASM_HASH_FILE)
	@echo "Controller WASM hash: $$(cat $(CONTROLLER_WASM_HASH_FILE))"
	@CTRL_HASH=$$(cat $(CONTROLLER_WASM_HASH_FILE)); \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].controller_wasm_hash = "'$$CTRL_HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@
	@echo "4/8 Deploying Governance..."
	@MIN_DELAY=$$(jq -r '.["$(NETWORK)"].timelock_min_delay_ledgers // empty' $(CONFIG_DIR)/networks.json); \
	if [ -n "$$DEPLOY_MIN_DELAY" ]; then \
		MIN_DELAY="$$DEPLOY_MIN_DELAY"; \
		echo "Bootstrap: DEPLOY_MIN_DELAY override = $$MIN_DELAY ledger(s). Deploy + setup run at this short delay WHILE PAUSED; raise to the production value with 'make $(NETWORK) updateDelay <ledgers>' (increase-only), then 'make $(NETWORK) unpause' to go live. On mainnet, unpause refuses until the delay reaches timelock_min_delay_ledgers."; \
	fi; \
	if [ -z "$$MIN_DELAY" ] || [ "$$MIN_DELAY" = "null" ]; then \
		echo "timelock_min_delay_ledgers not configured for $(NETWORK) in $(CONFIG_DIR)/networks.json"; \
		exit 1; \
	fi; \
	echo "Timelock min delay: $$MIN_DELAY ledgers"; \
	stellar contract deploy \
		--wasm $(DEPLOY_DIR)/governance.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) \
		--alias governance \
		-- --admin $(SIGNER_ADDRESS) --min_delay $$MIN_DELAY
	@GOV_ID=$$(stellar contract alias show governance --network $(NETWORK) | tail -n1); \
	if [ -z "$$GOV_ID" ]; then echo "Governance alias not resolvable after deploy"; exit 1; fi; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].governance = "'$$GOV_ID'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@
	@
	@echo "5/8 Deploying Controller via governance..."
	@GOV_ID=$$(stellar contract alias show governance --network $(NETWORK) | tail -n1); \
	CTRL_ID=$$(stellar contract invoke --id $$GOV_ID $(SOURCE_FLAG) --network $(NETWORK) \
		-- deploy_controller --wasm_hash $$(cat $(CONTROLLER_WASM_HASH_FILE)) | tail -n1 | tr -d '"'); \
	if [ -z "$$CTRL_ID" ]; then echo "deploy_controller returned no address"; exit 1; fi; \
	echo "Controller: $$CTRL_ID"; \
	stellar contract alias add controller --id $$CTRL_ID --network $(NETWORK) --overwrite; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].controller = "'$$CTRL_ID'" | .["$(NETWORK)"].hub_ids = {} | .["$(NETWORK)"].spoke_ids = {} | .["$(NETWORK)"].pool = ""' \
	$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@
	@
	@echo "6/8 Deploying Price Aggregator via governance..."
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/price_aggregator.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(PRICE_AGGREGATOR_WASM_HASH_FILE)
	@echo "Price Aggregator WASM hash: $$(cat $(PRICE_AGGREGATOR_WASM_HASH_FILE))"
	@GOV_ID=$$(stellar contract alias show governance --network $(NETWORK) | tail -n1); \
	PA_ID=$$(stellar contract invoke --id $$GOV_ID $(SOURCE_FLAG) --network $(NETWORK) \
		-- deploy_price_aggregator --wasm_hash $$(cat $(PRICE_AGGREGATOR_WASM_HASH_FILE)) | tail -n1 | tr -d '"'); \
	if [ -z "$$PA_ID" ]; then echo "deploy_price_aggregator returned no address"; exit 1; fi; \
	echo "Price Aggregator: $$PA_ID"; \
	stellar contract alias add price_aggregator --id $$PA_ID --network $(NETWORK) --overwrite; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].price_aggregator = "'$$PA_ID'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@
	@
	@echo "7/8 Deploying central pool via governance timelock..."
	@POOL=$$(NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh deployPool $$(cat $(POOL_WASM_HASH_FILE)) | tail -n1 | tr -d '"'); \
	if [ -z "$$POOL" ]; then echo "deployPool returned no address"; exit 1; fi; \
	echo "Central pool: $$POOL"; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].pool = "'$$POOL'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@echo "8/8 Deploying position NFT via governance timelock..."
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/position_nft.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > $(POSITION_NFT_WASM_HASH_FILE)
	@echo "Position NFT WASM hash: $$(cat $(POSITION_NFT_WASM_HASH_FILE))"
	@NFT=$$(NETWORK=$(NETWORK) SIGNER=$(SIGNER) \
		POSITION_NFT_URI="$(POSITION_NFT_URI)" \
		POSITION_NFT_NAME="$(POSITION_NFT_NAME)" \
		POSITION_NFT_SYMBOL="$(POSITION_NFT_SYMBOL)" \
		bash $(CONFIG_DIR)/script.sh deployPositionNft $$(cat $(POSITION_NFT_WASM_HASH_FILE)) | tail -n1 | tr -d '"'); \
	if [ -z "$$NFT" ]; then echo "deployPositionNft returned no address"; exit 1; fi; \
	echo "Position NFT: $$NFT"; \
	stellar contract alias add position_nft --id $$NFT --network $(NETWORK) --overwrite; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].position_nft = "'$$NFT'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@echo "=== Deployment complete ==="
	@echo "Aggregator:     $$(stellar contract alias show aggregator --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Governance:     $$(stellar contract alias show governance --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Controller:     $$(stellar contract alias show controller --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Pool:           $$(jq -r '.["$(NETWORK)"].pool // empty' $(CONFIG_DIR)/networks.json)"
	@echo "Position NFT:   $$(jq -r '.["$(NETWORK)"].position_nft // empty' $(CONFIG_DIR)/networks.json)"
	@echo "Pool WASM Hash: $$(cat $(POOL_WASM_HASH_FILE))"
	@echo "Controller WASM Hash: $$(cat $(CONTROLLER_WASM_HASH_FILE))"


configure-controller: _preflight-configure-controller
	@echo "=== Configuring Controller via governance on $(NETWORK) ==="
	@AGG=$$(jq -r '.["$(NETWORK)"].aggregator // empty' $(CONFIG_DIR)/networks.json); \
	if [ -n "$${AGGREGATOR_CONTRACT:-}" ]; then AGG="$$AGGREGATOR_CONTRACT"; fi; \
	if [ -z "$$AGG" ] || [ "$$AGG" = "null" ]; then \
		if [ "$${ALLOW_MISSING_AGGREGATOR:-0}" = "1" ]; then \
			echo "WARNING: skipping aggregator configuration (ALLOW_MISSING_AGGREGATOR=1)."; \
		else \
			echo "ERROR: aggregator not configured. Set networks.json aggregator or AGGREGATOR_CONTRACT before configure-controller."; \
			echo "To deliberately skip, set ALLOW_MISSING_AGGREGATOR=1."; \
			exit 1; \
		fi; \
	else \
		NETWORK=$(NETWORK) SIGNER=$(SIGNER) AGGREGATOR_CONTRACT=$$AGG bash $(CONFIG_DIR)/script.sh setAggregator; \
	fi
	@echo "Setting revenue accumulator (treasury wallet; required claimRevenue)..."
	@ACC=$$(jq -r '.["$(NETWORK)"].accumulator // empty' $(CONFIG_DIR)/networks.json); \
	if [ -n "$${ACCUMULATOR_CONTRACT:-}" ]; then ACC="$$ACCUMULATOR_CONTRACT"; fi; \
	if [ -z "$$ACC" ] || [ "$$ACC" = "null" ]; then \
		if [ "$${ALLOW_MISSING_ACCUMULATOR:-0}" = "1" ]; then \
			echo "WARNING: skipping accumulator configuration (ALLOW_MISSING_ACCUMULATOR=1). claimRevenue fails with NoAccumulator (#211) until set."; \
		else \
			echo "ERROR: accumulator not configured. Set networks.json accumulator or ACCUMULATOR_CONTRACT before configure-controller."; \
			echo "To deliberately skip, set ALLOW_MISSING_ACCUMULATOR=1."; \
			exit 1; \
		fi; \
	else \
		NETWORK=$(NETWORK) SIGNER=$(SIGNER) ACCUMULATOR_CONTRACT=$$ACC bash $(CONFIG_DIR)/script.sh setAccumulator; \
	fi
	@echo "Price aggregator wiring skipped here: governance's deploy_price_aggregator wires the"
	@echo "controller atomically at deploy. Re-point a live aggregator with 'make $(NETWORK) setPriceAggregator'"
	@echo "(timelocked SetPriceAggregator self-op, Sensitive tier)."
	@echo "Controller role grants skipped: controller uses owner-gated admin and caller-auth operational flows."
	@echo "Controller configured."



setup-testnet: NETWORK=testnet
setup-testnet: _preflight-setup deploy-testnet configure-controller _setup-markets _unpause-after-setup _post-setup-status




setup-mainnet: NETWORK=mainnet
setup-mainnet: _preflight-setup deploy-mainnet configure-controller _setup-markets _post-setup-status

_unpause-after-setup:
	@echo "=== Unpausing $(NETWORK) protocol via governance ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh unpause

_setup-markets:
	@echo "=== Setting up markets from $(CONFIG_DIR)/$(NETWORK)/markets.json ==="
	@if [ ! -f $(CONFIG_DIR)/$(NETWORK)/markets.json ]; then \
		echo "Config file not found: $(CONFIG_DIR)/$(NETWORK)/markets.json"; \
		echo "Create it based on the configs/testnet/markets.json pattern."; \
		exit 1; \
	fi
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh setupAll

























SIMPLE_ACTIONS := listMarkets listSpokes listHubs listOracles listReferences listOps executeReady \
	configureOracleFeeds reconfigureOracleFeeds listOracleFeeds configureOracleWindows \
	verifyOracleAdapterWindows finalizeOracleAdapterUpgrade \
	validateConfigs checkDelay \
	setupAll setupAllMarkets setupAllSpokes setupAllReferenceOracles \
	whitelistBlendPools approveBlendPools configureSpokeCurves \
	setAggregator setAccumulator pause unpause info \
	getAllMarkets getAllIndexes getMinBorrowCollateralUsd getBulkIndexes \
	claimRevenueAll deployPool deployPositionNft updateDelay \
	acceptAggregatorOwnership acceptOracleAdapterOwnership
POSITIONAL_MARKET_ACTIONS := createMarket updateMarketParams \
	configureMarketOracle \
	editOracleTolerance \
	getPrice getMarket getIndex \
	getOracle getReflector \
	getUtilisation getReserves getSupplied getBorrowed getDepositRate getBorrowRate \
	getRevenue getSyncData
POSITIONAL_ID_ACTIONS := addSpoke getSpoke createHub removeSpoke \
	executeOp cancelOp opState awaitOp transferGovOwnership \
	revokeBlendPool setPositionLimits setMinBorrowCollateralUsd setPositionManager \
	transferCtrlOwnership migrateController accountExists isBlendPoolApproved \
	addOracleSigner setOracleSubmissionAge setOracleMaxStale setOracleRelativeSkew \
	setSpokeLiquidationCurve \
	configureReferenceOracle \
	setAggregatorFee addAggregatorWhitelist removeAggregatorWhitelist \
	addAggregatorReferral setAggregatorReferralFee setAggregatorReferralActive \
	setAggregatorReferralOwner upgradeAggregatorHash upgradeOracleAdapterHash \
	transferAggregatorOwnership transferOracleAdapterOwnership
POSITIONAL_ID_ASSET_ACTIONS := addAssetToSpoke editAssetInSpoke removeAssetFromSpoke getSpokeAsset
POSITIONAL_ACCOUNT_ACTIONS := getHealth getAccount getCollateralUsd getBorrowUsd \
                              getLtvUsd getLiqAvailable canLiquidate
POSITIONAL_ACCOUNT_MARKET_ACTIONS := getCollateral getBorrow maxWithdraw maxSupply maxBorrow
POSITIONAL_ACCOUNT_ROLE_ACTIONS := hasRole grantGovRole revokeGovRole
REFLECTOR_PROBE_ACTIONS := queryReflector queryReflectorPrice queryReflectorTwap queryRedStone
VARARG_ACTIONS := updateIndexes claimRevenue supply borrow withdraw getLiquidationEstimate \
	claimAggregatorAdminFees sweepAggregatorBalance



MAKEFILE_ACTIONS := deploy upgradeController upgradeGovernance upgradePool upgradeAll \
                    deployFlashReceiver fundFlashReceiver testFlashReceiver deployAggregator deployOracleAdapter prepayRent setup resume \
                    upgradeAggregator upgradeOracleAdapter upgradeOracleAdapterFull

ALL_ACTIONS := $(SIMPLE_ACTIONS) $(POSITIONAL_MARKET_ACTIONS) $(POSITIONAL_ID_ACTIONS) \
               $(POSITIONAL_ID_ASSET_ACTIONS) $(POSITIONAL_ACCOUNT_ACTIONS) \
               $(POSITIONAL_ACCOUNT_MARKET_ACTIONS) $(POSITIONAL_ACCOUNT_ROLE_ACTIONS) \
               $(REFLECTOR_PROBE_ACTIONS) $(VARARG_ACTIONS) $(MAKEFILE_ACTIONS)

.PHONY: $(ALL_ACTIONS)





define NETWORK_DISPATCH
	@action="$(word 2,$(MAKECMDGOALS))"; \
	if [ -z "$$action" ]; then \
		echo "Error: please specify an action for $(1)"; \
		echo "Run 'make help' for available commands"; \
		exit 1; \
	fi; \
	case " $(ALL_ACTIONS) " in \
		*" $$action "*) ;; \
		*) echo "Error: unknown action '$$action'"; echo "Run 'make help'"; exit 1;; \
	esac; \
	case " $(MAKEFILE_ACTIONS) " in \
		*" $$action "*) \
			case "$$action" in \
				deploy)             $(MAKE) --no-print-directory _deploy NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradeController)  $(MAKE) --no-print-directory upgrade-controller NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradeGovernance)  $(MAKE) --no-print-directory upgrade-governance NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradePool)       $(MAKE) --no-print-directory upgrade-pool NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradeAll)         $(MAKE) --no-print-directory upgrade-all NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				deployFlashReceiver) $(MAKE) --no-print-directory deploy-flash-loan-receiver NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				fundFlashReceiver)  $(MAKE) --no-print-directory fund-flash-loan-receiver NETWORK=$(1) SIGNER=$(SIGNER) FLASH_MARKET=$(FLASH_MARKET) FLASH_RECEIVER_FUND=$(FLASH_RECEIVER_FUND) ;; \
				testFlashReceiver)  $(MAKE) --no-print-directory test-flash-loan-receiver NETWORK=$(1) SIGNER=$(SIGNER) FLASH_MARKET=$(FLASH_MARKET) FLASH_LOAN_AMOUNT=$(FLASH_LOAN_AMOUNT) ;; \
				deployAggregator)   $(MAKE) --no-print-directory deploy-aggregator NETWORK=$(1) SIGNER=$(SIGNER) AGGREGATOR_ADMIN=$(AGGREGATOR_ADMIN) ;; \
				deployOracleAdapter) $(MAKE) --no-print-directory deploy-oracle-adapter NETWORK=$(1) SIGNER=$(SIGNER) ORACLE_ADAPTER_ADMIN=$(ORACLE_ADAPTER_ADMIN) ORACLE_ADAPTER_SIGNERS=$(ORACLE_ADAPTER_SIGNERS) ORACLE_ADAPTER_THRESHOLD=$(ORACLE_ADAPTER_THRESHOLD) ORACLE_ADAPTER_RESOLUTION=$(ORACLE_ADAPTER_RESOLUTION) ;; \
				upgradeAggregator)  $(MAKE) --no-print-directory upgrade-aggregator NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradeOracleAdapter) $(MAKE) --no-print-directory upgrade-oracle-adapter NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradeOracleAdapterFull) $(MAKE) --no-print-directory upgrade-oracle-adapter-full NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				prepayRent)         $(MAKE) --no-print-directory prepay-rent NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				setup)              if [ "$(1)" = "mainnet" ]; then \
						$(MAKE) --no-print-directory _preflight-setup _deploy configure-controller _setup-markets prepay-rent _post-setup-status NETWORK=$(1) SIGNER=$(SIGNER); \
						echo ""; \
						echo "Mainnet setup complete — protocol left PAUSED (never unpaused at a bootstrap delay)."; \
						echo "Raise the timelock to the production floor, then go live:"; \
						echo "  make mainnet updateDelay <floor>   # e.g. 34560 (48h)"; \
						echo "  make mainnet unpause               # refuses until delay >= floor"; \
					else \
						$(MAKE) --no-print-directory _preflight-setup _deploy configure-controller _setup-markets _unpause-after-setup prepay-rent _post-setup-status NETWORK=$(1) SIGNER=$(SIGNER); \
					fi ;; \
				resume)             if [ "$(1)" = "mainnet" ]; then \
						$(MAKE) --no-print-directory _preflight-configure-controller configure-controller _setup-markets prepay-rent _post-setup-status NETWORK=$(1) SIGNER=$(SIGNER); \
						echo ""; \
						echo "Mainnet resume complete — protocol left PAUSED. Go live with:"; \
						echo "  make mainnet updateDelay <floor> && make mainnet unpause"; \
					else \
						$(MAKE) --no-print-directory _preflight-configure-controller configure-controller _setup-markets _unpause-after-setup prepay-rent _post-setup-status NETWORK=$(1) SIGNER=$(SIGNER); \
					fi ;; \
			esac; \
			exit 0 ;; \
	esac; \
	args="$(wordlist 3,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))"; \
	NETWORK=$(1) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh $$action $$args
endef

testnet:
	$(call NETWORK_DISPATCH,testnet)

mainnet:
	$(call NETWORK_DISPATCH,mainnet)




$(ALL_ACTIONS):
	@:





%:
	@if [ "$(word 1,$(MAKECMDGOALS))" != "testnet" ] && [ "$(word 1,$(MAKECMDGOALS))" != "mainnet" ]; then \
		echo "Error: unknown target '$@' (run 'make help')"; \
		exit 1; \
	fi






invoke:
	@CTRL=$$(stellar contract alias show $(CONTRACT) --network $(NETWORK) | tail -n1); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) -- $(FN) $(ARGS)


invoke-id:
	@stellar contract invoke --id $(CONTRACT_ID) $(SOURCE_FLAG) --network $(NETWORK) -- $(FN) $(ARGS)


view:
	@CTRL=$$(stellar contract alias show $(CONTRACT) --network $(NETWORK) | tail -n1); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) --send=no -- $(FN) $(ARGS)


view-id:
	@stellar contract invoke --id $(CONTRACT_ID) $(SOURCE_FLAG) --network $(NETWORK) --send=no -- $(FN) $(ARGS)






# -----------------------------------------------------------------------------
# Help (layered): usage (default) -> help (index) -> help-<topic> -> help-all
#
# Layout helpers (printf two-column). ASCII-only for stable column width.
#   $(call H1,title)     section banner
#   $(call H2,title)     subsection label
#   $(call ROW,cmd,desc) aligned command + description
#   $(call NOTE,text)    indented note / env line
# Commas inside call args split parameters -- avoid commas in text, or use
# a single-arg NOTE line. Do not put unescaped # or lone \ at EOL in recipes.
# -----------------------------------------------------------------------------
H_RULE := ----------------------------------------------------------------
H1 = @printf '%s\n%s\n%s\n\n' "$(H_RULE)" "  $(1)" "$(H_RULE)"
H2 = @printf '  %s\n' "$(1)"
ROW = @printf '    %-48s %s\n' "$(1)" "$(2)"
NOTE = @printf '    %s\n' "$(1)"
BLANK = @printf '\n'

usage:
	$(call H1,Stellar Lending Protocol)
	$(call H2,Quick start)
	$(call ROW,make help,command index + topics)
	$(call ROW,make build | test | clippy | fmt | coverage,daily develop loop)
	$(call BLANK)
	$(call H2,Deploy  (network = testnet | mainnet))
	$(call ROW,make <network> setup,deploy + configure + unpause)
	$(call ROW,make <network> resume,re-run config after partial failure)
	$(call ROW,make <network> validateConfigs,cross-check markets/spokes/networks JSON)
	$(call ROW,make <network> listOps,governance ops + live state)
	$(call ROW,make <network> info,deployed addresses + oracle wiring)
	$(call BLANK)
	$(call H2,Topics)
	$(call NOTE,make help-build | help-verify | help-deploy | help-ops)
	$(call NOTE,make help-views | help-oracle | help-aggregator | help-all)
	$(call NOTE,configs live under configs/)
	$(call BLANK)


help:
	$(call H1,Command index)
	$(call H2,Conventions)
	$(call ROW,make <network> <action> [args],network = testnet | mainnet)
	$(call ROW,SIGNER=ledger make mainnet ...,hardware-wallet signing)
	$(call ROW,configs/,JSON under configs/)
	$(call BLANK)
	$(call H2,Daily drivers)
	$(call ROW,develop,build | test | test-one FILE=x | clippy | fmt | coverage | clean)
	$(call ROW,keys,keygen)
	$(call ROW,deploy,setup | resume | deploy | upgradeAll | info)
	$(call ROW,governance,validateConfigs | listOps | executeReady)
	$(call ROW,markets,setupAllMarkets | setupAllSpokes | setupAll)
	$(call ROW,positions,supply | borrow | withdraw <asset> <amt> [account])
	$(call ROW,control,pause | unpause)
	$(call ROW,escape,view | invoke FN=... ARGS=... NETWORK=testnet)
	$(call NOTE,           invoke-id CONTRACT_ID=C... FN=... ARGS=... NETWORK=testnet)
	$(call BLANK)
	$(call H2,Topics)
	$(call ROW,make help-build,WASM | tests | coverage | lint)
	$(call ROW,make help-verify,miri | fuzz | proptest | mutants | scout | certora)
	$(call ROW,make help-deploy,setup | upgrades | mainnet env | flash receiver)
	$(call ROW,make help-ops,governance | markets | spokes | positions | control)
	$(call ROW,make help-views,read-only probes (controller + pool))
	$(call ROW,make help-oracle,adapter | feeds | Reflector | RedStone)
	$(call ROW,make help-aggregator,swap aggregator admin + ownership)
	$(call ROW,make help-all,print every topic (grep-friendly))
	$(call BLANK)


help-build:
	$(call H1,Build | test | coverage | lint)
	$(call H2,Build)
	$(call ROW,make build,all contracts (WASM))
	$(call ROW,make build-one CRATE=...,one crate)
	$(call ROW,make optimize,build + optimize WASM)
	$(call ROW,make deploy-artifacts,mainnet WASM -> $(DEPLOY_DIR))
	$(call ROW,make wasm-size-check,deploy artifacts + size budget)
	$(call ROW,make integration-wasm,deploy-sized WASM + harness mocks)
	$(call ROW,make certora-wasm,Certora-feature WASM)
	$(call ROW,make wasm-artifacts,deploy + certora -> $(WASM_ARTIFACTS_DIR))
	$(call BLANK)
	$(call H2,Test)
	$(call ROW,make test,all test-harness tests)
	$(call ROW,make test-one FILE=x,one harness file)
	$(call ROW,make test-match | test-pool,filtered / pool-focused)
	$(call BLANK)
	$(call H2,Coverage)
	$(call ROW,make coverage,merged coverage + CLI summary)
	$(call ROW,make coverage-controller | -pool | -merged,scoped reports)
	$(call ROW,make fuzz-coverage,fast math fuzz (corpus replay))
	$(call ROW,make fuzz-coverage-all,+ contract targets (slow on macOS))
	$(call ROW,make fuzz-coverage-one TARGET=flow_e2e,[FUZZ_COV_TIME=30])
	$(call BLANK)
	$(call H2,Lint / clean)
	$(call ROW,make fmt | fmt-check | clippy | clean,)
	$(call BLANK)


help-verify:
	$(call H1,Deep verification)
	$(call ROW,make miri-all,Miri UB on pure-i128 math)
	$(call ROW,make fuzz,libFuzzer math (FUZZ_TIME=60))
	$(call ROW,make fuzz-contract,libFuzzer contract flows)
	$(call ROW,make proptest,properties (PROPTEST_CASES=N))
	$(call ROW,make mutants,full mutation suite)
	$(call ROW,make mutants-math,focused math (+ -rates | -pool-interest))
	$(call ROW,make scout,Scout audit (scout-strict gates incomplete))
	$(call ROW,make certora,cloud jobs (CERTORA_PROFILE=sanity))
	$(call ROW,make certora-list,list Certora profiles)
	$(call ROW,make certora-wasm,build Certora-feature WASM first)
	$(call BLANK)


help-deploy:
	$(call H1,Deployment)
	$(call NOTE,Pattern:  make <network> <action>     network = testnet | mainnet)
	$(call BLANK)
	$(call H2,Bootstrap)
	$(call ROW,make keygen,deployer key (testnet: friendbot))
	$(call ROW,make setup-testnet,alias for make testnet setup)
	$(call ROW,make <n> setup,deploy + config + markets/spokes + unpause)
	$(call ROW,make <n> resume,re-run config (skips deploy))
	$(call ROW,make <n> deploy,contracts only (no market config))
	$(call ROW,make <n> info,deployed contract IDs)
	$(call BLANK)
	$(call H2,Upgrades (timelocked))
	$(call NOTE,make <n> upgradeController | upgradeGovernance | upgradePool | upgradeAll)
	$(call BLANK)
	$(call H2,Mainnet env (optional))
	$(call NOTE,AGGREGATOR_CONTRACT=C... ACCUMULATOR_CONTRACT=G... make mainnet setup)
	$(call NOTE,  Aggregator = swap router | Accumulator = revenue treasury)
	$(call NOTE,  ALLOW_MISSING_AGGREGATOR=1 / ALLOW_MISSING_ACCUMULATOR=1 to bootstrap without)
	$(call NOTE,AWAIT_MAX_WAIT_SECONDS=259200 make mainnet setup     (cap ~48h await))
	$(call NOTE,DEPLOY_MIN_DELAY=1 make mainnet setup               (bootstrap delay; then:))
	$(call NOTE,make mainnet updateDelay 34560                      (timelocked min-delay))
	$(call BLANK)
	$(call H2,Flash-loan test receiver)
	$(call NOTE,make <n> deployFlashReceiver | fundFlashReceiver | testFlashReceiver)
	$(call BLANK)
	$(call H2,Related)
	$(call NOTE,make help-oracle | help-aggregator)
	$(call BLANK)


help-ops:
	$(call H1,Config-driven ops)
	$(call NOTE,Pattern:  make <network> <action> [args])
	$(call BLANK)
	$(call H2,Governance / timelock)
	$(call ROW,make <n> validateConfigs,cross-check markets/spokes/networks JSON)
	$(call ROW,make <n> listOps,recorded ops + live state)
	$(call ROW,make <n> executeReady,execute every Ready op)
	$(call NOTE,make <n> opState | awaitOp | executeOp | cancelOp <id>)
	$(call NOTE,    per-op lifecycle: Unset | Waiting | Ready | Done)
	$(call ROW,make <n> checkDelay,live timelock delay vs config)
	$(call BLANK)
	$(call H2,Timelock knobs)
	$(call NOTE,AUTO_EXECUTE=0 make <n> <verb>     schedule only; execute later)
	$(call NOTE,REAPPLY_ON_DONE=0 / SALT_NONCE=N   re-apply / fresh salt for Done ops)
	$(call NOTE,Direct verbs auto re-apply Done (fresh salt); setupAll*/resume skip Done)
	$(call BLANK)
	$(call H2,Canceller-council recovery (owner-only | ~30d | non-vetoable))
	$(call NOTE,GOVERNANCE entrypoints via invoke-id (not the config dispatcher):)
	$(call NOTE,propose:  make invoke-id CONTRACT_ID=GOV FN=propose_canceller_reset)
	$(call NOTE,           ARGS=--new_cancellers [G...] --salt SALT64)
	$(call NOTE,execute:  make invoke-id CONTRACT_ID=GOV FN=execute_canceller_reset)
	$(call NOTE,           ARGS=--executor null --new_cancellers [G...] --salt SALT64)
	$(call BLANK)
	$(call H2,Markets)
	$(call NOTE,make <n> createMarket|updateMarketParams|configureMarketOracle SYM)
	$(call ROW,make <n> configureReferenceOracle SYM,set_oracle(PriceKey::Ref))
	$(call NOTE,make <n> setupAllReferenceOracles | setupAllMarkets     (batch from JSON))
	$(call NOTE,make <n> editOracleTolerance SYM BPS | updateIndexes SYM...)
	$(call NOTE,make <n> listMarkets | listReferences | listOracles)
	$(call BLANK)
	$(call H2,Hubs / spokes)
	$(call NOTE,make <n> listHubs | createHub ID | addSpoke ID | listSpokes)
	$(call NOTE,make <n> addAssetToSpoke|editAssetInSpoke|removeAssetFromSpoke ID SYM)
	$(call NOTE,make <n> removeSpoke ID | setupAllSpokes | setupAll)
	$(call BLANK)
	$(call H2,Positions)
	$(call ROW,make <n> supply USDC 1000000000,100 USDC @ 7 dec -> account 0)
	$(call ROW,make <n> borrow USDC 100000000 ACCOUNT,direct borrow (no swap))
	$(call ROW,make <n> withdraw USDC 100000000 ACCOUNT,0 amount = withdraw all)
	$(call BLANK)
	$(call H2,Strategies (need AggregatorSwap JSON from quote server))
	$(call NOTE,make invoke FN=multiply ARGS=--caller G... --swap @swap.json NETWORK=testnet)
	$(call BLANK)
	$(call H2,Protocol control)
	$(call ROW,make <n> pause | unpause,guardian immediate / timelocked unpause)
	$(call ROW,make <n> setAggregator | setAccumulator,from networks.json or env)
	$(call NOTE,make <n> grantGovRole|revokeGovRole G... ROLE)
	$(call NOTE,    ROLE = PROPOSER | EXECUTOR | CANCELLER | ORACLE | GUARDIAN)
	$(call ROW,make <n> setPositionLimits 10 10,max supply/borrow positions)
	$(call NOTE,make <n> setMinBorrowCollateralUsd RAY)
	$(call NOTE,make <n> setPositionManager G... true)
	$(call NOTE,make <n> setSpokeLiquidationCurve SPOKE THF HF_MAX BPS)
	$(call NOTE,make <n> transferCtrlOwnership|transferGovOwnership ADDR LEDGER)
	$(call NOTE,make <n> migrateController VER | revokeBlendPool C...)
	$(call NOTE,make <n> claimRevenue SYM... | claimRevenueAll)
	$(call NOTE,make <n> whitelistBlendPools | approveBlendPools | configureSpokeCurves)
	$(call BLANK)
	$(call H2,Escape hatches)
	$(call NOTE,make view FN=... ARGS=... NETWORK=testnet)
	$(call NOTE,make invoke FN=... ARGS=... NETWORK=testnet)
	$(call NOTE,make invoke-id CONTRACT_ID=C... FN=... ARGS=... NETWORK=testnet)
	$(call BLANK)


help-views:
	$(call H1,Read-only probes)
	$(call NOTE,No signing cost.  Prefix:  make testnet <verb> [args])
	$(call BLANK)
	$(call H2,Deployment / roles)
	$(call ROW,info,deployment addresses)
	$(call ROW,hasRole ADDR ROLE,)
	$(call BLANK)
	$(call H2,Markets)
	$(call ROW,getPrice|getMarket|getIndex SYM,spot / listing / RAY index)
	$(call NOTE,getAllMarkets | getAllIndexes)
	$(call ROW,getSpokeAsset SPOKE SYM,live config for any spoke)
	$(call ROW,getOracle SYM,price components  (-> help-oracle))
	$(call BLANK)
	$(call H2,Account / position)
	$(call NOTE,getSpoke|getHealth|getAccount|accountExists ID)
	$(call NOTE,getCollateralUsd|getBorrowUsd|getLtvUsd ID)
	$(call NOTE,getLiqAvailable|canLiquidate ID)
	$(call NOTE,getCollateral|getBorrow ID SYM)
	$(call ROW,maxWithdraw|maxSupply|maxBorrow ID SYM,headroom / max executable)
	$(call ROW,getLiquidationEstimate ID SYM AMT,seize / repay / refund / bonus)
	$(call NOTE,getMinBorrowCollateralUsd | isBlendPoolApproved C...)
	$(call BLANK)
	$(call H2,Pool (hub-level; spokes share hub liquidity))
	$(call NOTE,getUtilisation|getReserves|getSupplied|getBorrowed SYM)
	$(call NOTE,getDepositRate|getBorrowRate|getRevenue|getSyncData SYM)
	$(call NOTE,getBulkIndexes)
	$(call BLANK)
	$(call H2,Note)
	$(call NOTE,No on-chain getters for is_paused | get_hub | get_aggregator |)
	$(call NOTE,get_accumulator | get_position_limits.  info/listHubs = local config only.)
	$(call BLANK)


help-oracle:
	$(call H1,Oracle adapter + probes)
	$(call NOTE,Standalone Ownable -- not governance-timelocked)
	$(call BLANK)
	$(call H2,Deploy / configure)
	$(call NOTE,make <n> deployOracleAdapter)
	$(call NOTE,  ORACLE_ADAPTER_ADMIN=G...            constructor admin (default: deployer))
	$(call NOTE,  ORACLE_ADAPTER_SIGNERS=[G...]         bot-signer set (default: deployer alone))
	$(call NOTE,  ORACLE_ADAPTER_THRESHOLD=N           N-of-M threshold (default: 1))
	$(call ROW,make <n> configureOracleFeeds,add_feed for every oracle_feeds.json entry)
	$(call ROW,make <n> addOracleSigner ADDR,register bot signer (idempotent))
	$(call BLANK)
	$(call H2,Upgrade / windows)
	$(call ROW,make <n> upgradeOracleAdapter,Wasm only)
	$(call NOTE,SIGNER=ledger make mainnet upgradeOracleAdapterFull)
	$(call NOTE,    Wasm + windows + feeds + verify getters)
	$(call ROW,make <n> reconfigureOracleFeeds,remove+add feeds only)
	$(call ROW,make <n> configureOracleWindows,age + stale + relative skew from JSON)
	$(call NOTE,make <n> setOracleRelativeSkew SECS)
	$(call ROW,make <n> verifyOracleAdapterWindows,print live window getters)
	$(call ROW,make <n> finalizeOracleAdapterUpgrade,windows + reconfigure (no Wasm))
	$(call BLANK)
	$(call H2,Ownership (OZ Ownable two-step))
	$(call NOTE,make <n> transferOracleAdapterOwnership OWNER LEDGER)
	$(call NOTE,SIGNER=ledger make <n> acceptOracleAdapterOwnership    (run as NEW owner))
	$(call BLANK)
	$(call H2,Probes)
	$(call ROW,make <n> getOracle SYM,live price components)
	$(call ROW,make <n> queryReflector CONTRACT,decimals + resolution)
	$(call NOTE,make <n> queryReflectorPrice C other|stellar ASSET     (lastprice))
	$(call ROW,make <n> queryReflectorTwap C other ASSET N,prices history)
	$(call NOTE,make <n> queryRedStone FEED_ID [adapter])
	$(call BLANK)


help-aggregator:
	$(call H1,Swap aggregator)
	$(call NOTE,Standalone Ownable -- not governance-timelocked)
	$(call NOTE,Prefix for admin verbs:  make testnet <verb> ...)
	$(call BLANK)
	$(call H2,Deploy / wire)
	$(call NOTE,make <n> deployAggregator)
	$(call NOTE,  AGGREGATOR_ADMIN=G...                constructor admin (default: deployer))
	$(call ROW,make <n> setAggregator,point controller at it (timelocked))
	$(call ROW,make <n> upgradeAggregator,build + upload + upgrade in place)
	$(call BLANK)
	$(call H2,Admin (direct owner invoke))
	$(call NOTE,setAggregatorFee BPS)
	$(call NOTE,addAggregatorWhitelist|removeAggregatorWhitelist TOKEN)
	$(call NOTE,addAggregatorReferral OWNER BPS)
	$(call NOTE,setAggregatorReferralFee ID BPS | setAggregatorReferralActive ID BOOL)
	$(call NOTE,setAggregatorReferralOwner ID NEW_OWNER)
	$(call NOTE,claimAggregatorAdminFees RECIPIENT TOKEN...)
	$(call NOTE,sweepAggregatorBalance RECIPIENT TOKEN...)
	$(call BLANK)
	$(call H2,Ownership (OZ Ownable two-step))
	$(call NOTE,make <n> transferAggregatorOwnership OWNER LEDGER)
	$(call NOTE,SIGNER=ledger make <n> acceptAggregatorOwnership       (run as NEW owner))
	$(call BLANK)


help-all:
	@$(MAKE) --no-print-directory help
	@printf '\n'
	@$(MAKE) --no-print-directory help-build
	@printf '\n'
	@$(MAKE) --no-print-directory help-verify
	@printf '\n'
	@$(MAKE) --no-print-directory help-deploy
	@printf '\n'
	@$(MAKE) --no-print-directory help-ops
	@printf '\n'
	@$(MAKE) --no-print-directory help-views
	@printf '\n'
	@$(MAKE) --no-print-directory help-oracle
	@printf '\n'
	@$(MAKE) --no-print-directory help-aggregator

.DEFAULT_GOAL := usage






CBM_PROJECT := Users-mihaieremia-GitHub-rs-lending-xlm
CBM_ROOT := $(CURDIR)

.PHONY: cbm-reindex cbm-index


cbm-index:
	codebase-memory-mcp cli index_repository '{"repo_path":"$(CBM_ROOT)","mode":"fast","persistence":true}'


cbm-reindex:
	-codebase-memory-mcp cli delete_project '{"project":"$(CBM_PROJECT)"}'
	rm -f .codebase-memory/graph.db.zst .codebase-memory/artifact.json
	codebase-memory-mcp cli index_repository '{"repo_path":"$(CBM_ROOT)","mode":"fast","persistence":true}'
	@echo "Graph rebuilt."
