# ===========================================================================
# Stellar Lending Protocol — Makefile
#
# Usage:
#   make build              Build all contracts (WASM)
#   make test               Run all tests
#   make coverage           Run coverage + generate report
#   make fmt                Format all code
#   make clippy             Lint all code
#   make clean              Clean build artifacts
#
# Deployment (requires stellar CLI + funded account):
#   make deploy-testnet     Deploy all contracts to testnet
#   make deploy-mainnet     Deploy all contracts to mainnet
#   make setup-testnet      Deploy + configure markets on testnet
#   make setup-mainnet      Deploy + configure markets on mainnet
#
# Ledger signing:
#   SIGNER=ledger make deploy-testnet
# ===========================================================================

SHELL := /bin/bash
.PHONY: \
        build build-one optimize deploy-artifacts \
        test test-verbose test-one test-match test-pool \
        miri-common \
        coverage coverage-controller coverage-pool coverage-merged \
        coverage-report coverage-report-controller coverage-report-pool coverage-report-merged \
        fmt fmt-check clippy clippy-contracts clean \
        fuzz fuzz-contract fuzz-one fuzz-build fuzz-seed-corpus \
        proptest proptest-one proptest-build \
        keygen deploy-testnet deploy-mainnet upgrade-controller _deploy \
        configure-controller setup-testnet setup-mainnet _setup-markets create-market \
        update-indexes \
        pause unpause grant-role revoke-role \
        info invoke invoke-id view view-id \
        help

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

WASM_TARGET  := wasm32v1-none
RELEASE_DIR  := target/$(WASM_TARGET)/release
OPTIMIZED_DIR := target/optimized
DEPLOY_DIR := target/deploy
COV_DIR := target/coverage

# Contract crates (order matters for deployment)
CONTRACTS := pool controller

# Coverage exclusions (no executable code / stubs only).
COV_IGNORE := --ignore-filename-regex="types\.rs|providers\.rs|router\.rs"

# Network config (override via env or CLI, for example `make SIGNER=ledger mainnet setupAll`)
NETWORK     ?= testnet
SIGNER      ?= deployer
CONTRACT    ?= controller
CONFIG_DIR  ?= configs
SIGNER_ADDRESS = $$(stellar keys public-key $(SIGNER) 2>/dev/null || stellar keys address $(SIGNER) 2>/dev/null || echo $(SIGNER))

# Stellar CLI source account flag
ifeq ($(SIGNER),ledger)
  SOURCE_FLAG = --source-account $(SIGNER_ADDRESS) --sign-with-ledger
else
  SOURCE_FLAG = --source-account $$(stellar keys secret $(SIGNER) 2>/dev/null || echo $(SIGNER))
endif

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

## Build all contracts (WASM release)
build:
	@echo "Building all contracts..."
	stellar contract build
	@echo ""
	@echo "WASM artifacts:"
	@ls -lh $(RELEASE_DIR)/*.wasm 2>/dev/null || ls -lh target/wasm32-unknown-unknown/release/*.wasm 2>/dev/null || echo "  (none found)"

## Build a single contract: make build-one CRATE=controller
build-one:
	@echo "Building $(CRATE)..."
	stellar contract build --package $(CRATE)

## Optimize WASM binaries for local tooling and inspection.
optimize: build
	@mkdir -p $(OPTIMIZED_DIR)
	@for contract in $(CONTRACTS); do \
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

## Create stripped deploy artifacts from optimized WASM.
deploy-artifacts: optimize
	@mkdir -p $(DEPLOY_DIR)
	@for contract in $(CONTRACTS); do \
		src="$(OPTIMIZED_DIR)/$$contract.wasm"; \
		dst="$(DEPLOY_DIR)/$$contract.wasm"; \
		cp "$$src" "$$dst"; \
	done
	@echo ""
	@echo "Deploy WASM:"
	@ls -lh $(DEPLOY_DIR)/*.wasm 2>/dev/null

# ---------------------------------------------------------------------------
# Test
# ---------------------------------------------------------------------------

## Run all tests
test:
	cargo test -p test-harness -- --test-threads=1

## Run all tests with output
test-verbose:
	cargo test -p test-harness -- --test-threads=1 --nocapture

## Run a specific test file: make test-one FILE=liquidation_tests
test-one:
	cargo test -p test-harness --test $(FILE) -- --test-threads=1

## Run tests matching a pattern: make test-match PATTERN=interest
test-match:
	cargo test -p test-harness $(PATTERN) -- --test-threads=1

## Run pool unit tests
test-pool:
	cargo test -p pool

## Run Miri on pure-i128 subset of common crate (rescale_half_up, div_by_int_half_up).
## Requires: rustup +nightly + miri + rust-src components.
miri-common:
	@cd common && MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-symbolic-alignment-check" \
		cargo +nightly miri test --lib -- \
		fp_core::tests::test_rescale \
		fp_core::tests::test_div_by_int

# ---------------------------------------------------------------------------
# Coverage
# ---------------------------------------------------------------------------

## Run coverage and print summary to CLI
coverage: coverage-merged

coverage-controller:
	@echo "Running controller coverage (controller unit tests + test-harness)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@cargo llvm-cov test -p controller --lib --no-report $(COV_IGNORE) 2>&1 | tail -5
	@backup="$(COV_DIR)/snapshots-backup"; \
	restore_snapshots() { \
		rm -rf test-harness/test_snapshots; \
		mkdir -p test-harness/test_snapshots; \
		cp -R "$$backup"/. test-harness/test_snapshots/; \
	}; \
	rm -rf "$$backup" && mkdir -p "$$backup"; \
	cp -R test-harness/test_snapshots/. "$$backup"/; \
	trap 'restore_snapshots' EXIT; \
	cargo llvm-cov test -p test-harness --no-report $(COV_IGNORE) -- --test-threads=1 2>&1 | tail -5
	@cargo llvm-cov report --lcov --output-path $(COV_DIR)/controller.lcov.info $(COV_IGNORE) >/dev/null
	@python3 $(CONFIG_DIR)/coverage_report.py \
		$(COV_DIR)/controller.lcov.info \
		$(COV_DIR)/controller-report.md \
		controller
	@echo "Reports saved to:"
	@echo "  $(COV_DIR)/controller.lcov.info"
	@echo "  $(COV_DIR)/controller-report.md"

coverage-pool:
	@echo "Running pool coverage (direct pool unit tests)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@cargo llvm-cov test -p pool --no-report $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov report --lcov --output-path $(COV_DIR)/pool.lcov.info $(COV_IGNORE) >/dev/null
	@python3 $(CONFIG_DIR)/coverage_report.py \
		$(COV_DIR)/pool.lcov.info \
		$(COV_DIR)/pool-report.md \
		pool
	@echo "Reports saved to:"
	@echo "  $(COV_DIR)/pool.lcov.info"
	@echo "  $(COV_DIR)/pool-report.md"

coverage-merged:
	@echo "Running merged coverage (controller + pool + test-harness)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@cargo llvm-cov test -p pool --no-report $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov test -p controller --lib --no-report $(COV_IGNORE) 2>&1 | tail -5
	@backup="$(COV_DIR)/snapshots-backup"; \
	restore_snapshots() { \
		rm -rf test-harness/test_snapshots; \
		mkdir -p test-harness/test_snapshots; \
		cp -R "$$backup"/. test-harness/test_snapshots/; \
	}; \
	rm -rf "$$backup" && mkdir -p "$$backup"; \
	cp -R test-harness/test_snapshots/. "$$backup"/; \
	trap 'restore_snapshots' EXIT; \
	cargo llvm-cov test -p test-harness --no-report $(COV_IGNORE) -- --test-threads=1 2>&1 | tail -5
	@cargo llvm-cov report --lcov --output-path $(COV_DIR)/merged.lcov.info $(COV_IGNORE) >/dev/null
	@python3 $(CONFIG_DIR)/coverage_report.py \
		$(COV_DIR)/merged.lcov.info \
		$(COV_DIR)/merged-report.md \
		merged
	@echo "Reports saved to:"
	@echo "  $(COV_DIR)/merged.lcov.info"
	@echo "  $(COV_DIR)/merged-report.md"

coverage-report: coverage-report-merged
coverage-report-controller: coverage-controller
coverage-report-pool: coverage-pool
coverage-report-merged: coverage-merged

# ---------------------------------------------------------------------------
# Code quality
# ---------------------------------------------------------------------------

## Format all code
fmt:
	cargo fmt --all

## Check formatting (CI mode)
fmt-check:
	cargo fmt --all -- --check

## Lint all code
clippy:
	cargo clippy --all-targets -- -D warnings

## Lint contracts only (no test-harness)
clippy-contracts:
	cargo clippy -p controller -p pool -p common -- -D warnings

# ---------------------------------------------------------------------------
# Clean
# ---------------------------------------------------------------------------

## Clean all build artifacts
clean:
	cargo clean
	rm -rf $(OPTIMIZED_DIR)
	rm -rf $(DEPLOY_DIR)
	rm -rf $(COV_DIR)

# ---------------------------------------------------------------------------
# Fuzzing (function-level math primitives)
# ---------------------------------------------------------------------------

FUZZ_TARGETS := fp_math rates_and_index
FUZZ_CONTRACT_TARGETS := flow_e2e flow_strategy
FUZZ_TIME ?= 60

# macOS requires `--sanitizer=thread -Zbuild-std` to link the contract-level
# targets (stellar-access cdylib + libFuzzer sancov conflict). Linux builds
# fine with the default sanitizer; detect and only opt-in on Darwin.
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
  FUZZ_FLAGS := --sanitizer=thread -Zbuild-std
else
  FUZZ_FLAGS :=
endif

## Run all fuzz targets for $(FUZZ_TIME) seconds each (default: 60s)
fuzz:
	@for t in $(FUZZ_TARGETS); do \
		echo "=== $$t ==="; \
		cd fuzz && cargo +nightly fuzz run $$t $(FUZZ_FLAGS) -- -max_total_time=$(FUZZ_TIME) 2>&1 | tail -3; cd ..; \
	done

## Run all contract-level libFuzzer targets for $(FUZZ_TIME) seconds each.
fuzz-contract:
	@for t in $(FUZZ_CONTRACT_TARGETS); do \
		echo "=== $$t ==="; \
		cd fuzz && cargo +nightly fuzz run $$t $(FUZZ_FLAGS) -- -max_total_time=$(FUZZ_TIME) 2>&1 | tail -3; cd ..; \
	done

## Run a single fuzz target: make fuzz-one TARGET=fp_math FUZZ_TIME=300
fuzz-one:
	@cd fuzz && cargo +nightly fuzz run $(TARGET) $(FUZZ_FLAGS) -- -max_total_time=$(FUZZ_TIME)

## Build all fuzz targets (compile-only)
fuzz-build:
	@cd fuzz && cargo +nightly fuzz build $(FUZZ_FLAGS)

## Seed fuzz/corpus/<target>/ from */test_snapshots/**/*.json. Run once before
## a campaign to give libFuzzer realistic numeric entropy from the start.
fuzz-seed-corpus:
	@cd fuzz && cargo run --release --features seed-corpus --bin seed_corpus -- --output corpus

# ---------------------------------------------------------------------------
# Contract-level property tests (proptest inside test-harness)
# ---------------------------------------------------------------------------

PROPTEST_TESTS := fuzz_multi_asset_solvency fuzz_conservation fuzz_auth_matrix \
                  fuzz_ttl_keepalive fuzz_budget_metering \
                  fuzz_strategy_flashloan fuzz_liquidation_differential
PROPTEST_CASES ?= 256

## Run all contract-level property tests.
## Set PROPTEST_CASES=10000 (or higher) for longer runs on dedicated hardware.
proptest:
	@for t in $(PROPTEST_TESTS); do \
		echo "=== $$t ==="; \
		PROPTEST_CASES=$(PROPTEST_CASES) cargo test --release -p test-harness --test $$t -- --test-threads=1; \
	done

## Run a single property test: make proptest-one TEST=fuzz_supply_borrow_liquidate PROPTEST_CASES=10000
proptest-one:
	@PROPTEST_CASES=$(PROPTEST_CASES) cargo test --release -p test-harness --test $(TEST) -- --test-threads=1

## Build property tests without running
proptest-build:
	@cargo build --release --tests -p test-harness

# ---------------------------------------------------------------------------
# Deployment
# ---------------------------------------------------------------------------

## Generate deployer key (one-time setup)
keygen:
	@echo "Generating deployer key for $(NETWORK)..."
	stellar keys generate deployer --network $(NETWORK) --fund
	@echo "Deployer address:"
	@stellar keys public-key deployer

## Deploy all contracts to a network
deploy-testnet: NETWORK=testnet
deploy-testnet: _deploy

deploy-mainnet: NETWORK=mainnet
deploy-mainnet: _deploy

## Upgrade the deployed controller contract in-place on the selected network.
upgrade-controller: deploy-artifacts
	@echo "=== Upgrading controller on $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK) | tail -n1); \
	if [ -z "$$CTRL" ]; then \
		echo "Controller alias not found on $(NETWORK)"; \
		exit 1; \
	fi; \
	stellar contract upload \
		--wasm $(DEPLOY_DIR)/controller.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > /tmp/controller_upgrade_wasm_hash.txt; \
	HASH=$$(cat /tmp/controller_upgrade_wasm_hash.txt); \
	echo "Controller: $$CTRL"; \
	echo "New WASM hash: $$HASH"; \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
		-- upgrade --new_wasm_hash $$HASH

_deploy: deploy-artifacts
	@echo "=== Deploying to $(NETWORK) ==="
	@echo "Signer: $(SIGNER)"
	@echo ""
	@echo "1/4 Checking Aggregator..."
	@AGGREGATOR=$$(jq -r ".\"$(NETWORK)\".aggregator" $(CONFIG_DIR)/networks.json 2>/dev/null); \
	if [ ! -z "$$AGGREGATOR" ] && [ "$$AGGREGATOR" != "null" ] && [ "$$AGGREGATOR" != "" ]; then \
		echo "Using Aggregator: $$AGGREGATOR"; \
		stellar contract alias add aggregator --id $$AGGREGATOR --network $(NETWORK) --overwrite || echo "Warning: Failed to set aggregator alias"; \
	else \
		echo "Skipping Aggregator setup (not configured or invalid)"; \
	fi
	@echo ""
	@# 2. Upload Pool WASM (template, not deployed directly)
	@echo "2/4 Uploading Pool WASM template..."
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/pool.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > /tmp/pool_wasm_hash.txt
	@echo "Pool WASM hash: $$(cat /tmp/pool_wasm_hash.txt)"
	@echo ""
	@# 3. Upload controller WASM explicitly so deploy references a network-installed hash.
	@echo "3/4 Uploading Controller WASM..."
	@stellar contract upload \
		--wasm $(DEPLOY_DIR)/controller.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) > /tmp/controller_wasm_hash.txt
	@echo "Controller WASM hash: $$(cat /tmp/controller_wasm_hash.txt)"
	@echo ""
	@# 4. Deploy Controller
	@echo "4/4 Deploying Controller..."
	@stellar contract deploy \
		--wasm-hash $$(cat /tmp/controller_wasm_hash.txt) \
		$(SOURCE_FLAG) \
		--network $(NETWORK) \
		--alias controller \
		-- --admin $(SIGNER_ADDRESS)
	@CTRL_ID=$$(stellar contract alias show controller --network $(NETWORK)); \
	POOL_HASH=$$(cat /tmp/pool_wasm_hash.txt); \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].controller = "'$$CTRL_ID'" | .["$(NETWORK)"].pool_wasm_hash = "'$$POOL_HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@echo "=== Deployment complete ==="
	@echo "Aggregator:     $$(stellar contract alias show aggregator --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Controller:     $$(stellar contract alias show controller --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Pool WASM Hash: $$(cat /tmp/pool_wasm_hash.txt)"

## Configure controller after deployment
configure-controller:
	@echo "=== Configuring Controller on $(NETWORK) ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh setAggregator || echo "Warning: setAggregator failed, continuing..."
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	POOL_HASH=$$(cat /tmp/pool_wasm_hash.txt); \
	echo "Setting pool template..."; \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
		-- set_liquidity_pool_template --hash $$POOL_HASH; \
	echo "Controller configured."

## Full setup: deploy + configure + create/configure markets and e-modes from config
setup-testnet: NETWORK=testnet
setup-testnet: deploy-testnet configure-controller _setup-markets

setup-mainnet: NETWORK=mainnet
setup-mainnet: deploy-mainnet configure-controller _setup-markets

_setup-markets:
	@echo "=== Setting up markets from $(CONFIG_DIR)/$(NETWORK)_markets.json ==="
	@if [ ! -f $(CONFIG_DIR)/$(NETWORK)_markets.json ]; then \
		echo "Config file not found: $(CONFIG_DIR)/$(NETWORK)_markets.json"; \
		echo "Create it based on configs/devnet_market_configs.json pattern."; \
		exit 1; \
	fi
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) ./configs/script.sh setupAll

## Create a single market (interactive)
create-market:
	@echo "Creating market for $(ASSET) on $(NETWORK)..."
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
		-- create_liquidity_pool \
		--asset $(ASSET_ADDRESS) \
		--params '$(MARKET_PARAMS)' \
		--config '$(ASSET_CONFIG)'

# ---------------------------------------------------------------------------
# Config-driven operations (via configs/script.sh)
#
# All values are read from JSON configs, not CLI args.
# Pattern: make <network> <action> [id] [asset]
#
# Examples:
#   make testnet addEModeCategory 1
#   make testnet addAssetToEMode 1 USDC
#   make testnet createMarket USDC
#   make testnet updateIndexes USDC XLM
#   make testnet setupAll
#   SIGNER=ledger make mainnet setupAll
# ---------------------------------------------------------------------------

# Network targets (set NETWORK and delegate to action)
testnet:
	@if [ -z "$(word 2,$(MAKECMDGOALS))" ]; then \
		echo "Please specify an action for network $@"; \
		echo "Run 'make help' for available commands"; \
		exit 1; \
	fi
	@NETWORK=testnet SIGNER=$(SIGNER) ./configs/script.sh $(filter-out $@,$(MAKECMDGOALS))

mainnet:
	@if [ -z "$(word 2,$(MAKECMDGOALS))" ]; then \
		echo "Please specify an action for network $@"; \
		echo "Run 'make help' for available commands"; \
		exit 1; \
	fi
	@NETWORK=mainnet SIGNER=$(SIGNER) ./configs/script.sh $(filter-out $@,$(MAKECMDGOALS))

# Config-driven actions.
# When invoked through `make testnet <action> ...`, the network target above
# already ran the script and these action targets must stay silent. When
# invoked directly (for example `make NETWORK=testnet listMarkets`), they
# still work as standalone entrypoints.
listMarkets listEModeCategories setupAll setupAllMarkets setupAllEModes:
	@if [ "$(firstword $(MAKECMDGOALS))" = "testnet" ] || [ "$(firstword $(MAKECMDGOALS))" = "mainnet" ]; then \
		:; \
	elif [ -z "$(NETWORK)" ]; then \
		echo "Error: NETWORK is required. Usage: make NETWORK=testnet $@ or make testnet $@"; \
		exit 1; \
	else \
		NETWORK=$(NETWORK) SIGNER=$(SIGNER) ./configs/script.sh $@; \
	fi

addEModeCategory addAssetToEMode createMarket editAssetConfig configureMarketOracle:
	@if [ "$(firstword $(MAKECMDGOALS))" = "testnet" ] || [ "$(firstword $(MAKECMDGOALS))" = "mainnet" ]; then \
		:; \
	elif [ -z "$(NETWORK)" ]; then \
		echo "Error: NETWORK is required. Usage: make NETWORK=testnet $@ ... or make testnet $@ ..."; \
		exit 1; \
	else \
		NETWORK=$(NETWORK) SIGNER=$(SIGNER) ./configs/script.sh $@ $(ID) $(ASSET); \
	fi

updateIndexes:
	@if [ "$(firstword $(MAKECMDGOALS))" = "testnet" ] || [ "$(firstword $(MAKECMDGOALS))" = "mainnet" ]; then \
		:; \
	elif [ -z "$(NETWORK)" ]; then \
		echo "Error: NETWORK is required. Usage: make NETWORK=testnet $@ MARKETS=\"USDC XLM\" or make testnet $@ USDC XLM"; \
		exit 1; \
	elif [ -z "$(MARKETS)" ]; then \
		echo "Error: MARKETS is required. Usage: make NETWORK=testnet $@ MARKETS=\"USDC XLM\""; \
		exit 1; \
	else \
		NETWORK=$(NETWORK) SIGNER=$(SIGNER) ./configs/script.sh $@ $(MARKETS); \
	fi

# Catch-all for positional args after testnet/mainnet
%:
	@:

## Sync indexes for one or more markets from config names.
update-indexes:
	@if [ -z "$(ASSETS)" ]; then \
		echo "Usage: make update-indexes NETWORK=testnet ASSETS='[\"C...\",\"C...\"]'"; \
		exit 1; \
	fi
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	CALLER=$(SIGNER_ADDRESS); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
		-- update_indexes --caller $$CALLER --assets '$(ASSETS)'

# ---------------------------------------------------------------------------
# Direct CLI operations (for one-off changes not in config)
# ---------------------------------------------------------------------------

## Pause the protocol
pause:
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) -- pause

## Unpause the protocol
unpause:
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) -- unpause

## Grant a role: make grant-role ACCOUNT=G... ROLE=KEEPER|REVENUE|ORACLE NETWORK=testnet
grant-role:
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
		-- grant_role --account $(ACCOUNT) --role $(ROLE)

## Revoke a role: make revoke-role ACCOUNT=G... ROLE=KEEPER|REVENUE|ORACLE NETWORK=testnet
revoke-role:
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK)); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) \
		-- revoke_role --account $(ACCOUNT) --role $(ROLE)

# ---------------------------------------------------------------------------
# Contract inspection
# ---------------------------------------------------------------------------

## Show contract info
info:
	@echo "=== Contract Aliases on $(NETWORK) ==="
	@stellar contract alias show controller --network $(NETWORK) 2>/dev/null && echo "  controller: found" || echo "  controller: not deployed"
	@stellar contract alias show aggregator --network $(NETWORK) 2>/dev/null && echo "  aggregator: found" || echo "  aggregator: not deployed"

## Invoke a controller function: make invoke FN=health_factor ARGS="--account_id 1"
invoke:
	@CTRL=$$(stellar contract alias show $(CONTRACT) --network $(NETWORK) | tail -n1); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) -- $(FN) $(ARGS)

## Invoke a function on an explicit contract id/alias: make invoke-id CONTRACT_ID=C... FN=reserves
invoke-id:
	@stellar contract invoke --id $(CONTRACT_ID) $(SOURCE_FLAG) --network $(NETWORK) -- $(FN) $(ARGS)

## Invoke a view function: make view FN=health_factor ARGS="--account_id 1"
view:
	@CTRL=$$(stellar contract alias show $(CONTRACT) --network $(NETWORK) | tail -n1); \
	stellar contract invoke --id $$CTRL $(SOURCE_FLAG) --network $(NETWORK) --send=no -- $(FN) $(ARGS)

## Invoke a view function on an explicit contract id/alias: make view-id CONTRACT_ID=C... FN=reserves
view-id:
	@stellar contract invoke --id $(CONTRACT_ID) $(SOURCE_FLAG) --network $(NETWORK) --send=no -- $(FN) $(ARGS)

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------

## Show this help
help:
	@echo "Stellar Lending Protocol Makefile"
	@echo ""
	@echo "Build & Test:"
	@echo "  make build              Build all contracts (WASM)"
	@echo "  make optimize           Build + optimize WASM binaries"
	@echo "  make test               Run all test-harness tests"
	@echo "  make test-one FILE=x    Run specific test file"
	@echo "  make coverage           Run merged coverage with CLI summary"
	@echo "  make coverage-controller  Coverage for controller/common via unit+harness"
	@echo "  make coverage-pool        Coverage for pool via direct unit tests"
	@echo "  make coverage-merged      Coverage merged across pool + controller + harness"
	@echo "  make coverage-report      Generate merged LCOV + Markdown reports"
	@echo "  make fmt                Format code"
	@echo "  make clippy             Lint all targets with warnings denied"
	@echo "  make clean              Clean artifacts"
	@echo ""
	@echo "Deployment:"
	@echo "  make keygen             Generate deployer key"
	@echo "  make deploy-testnet     Deploy all contracts to testnet"
	@echo "  make deploy-mainnet     Deploy all contracts to mainnet"
	@echo "  make upgrade-controller NETWORK=testnet"
	@echo "  make setup-testnet      Full setup (deploy + config + markets)"
	@echo "  make info               Show deployed contract IDs"
	@echo "  make view FN=x          Call a view function"
	@echo ""
	@echo "Config-driven operations (reads from JSON):"
	@echo "  make testnet addEModeCategory 1"
	@echo "  make testnet addAssetToEMode 1 USDC"
	@echo "  make testnet createMarket USDC"
	@echo "  make testnet editAssetConfig USDC"
	@echo "  make testnet configureMarketOracle USDC"
	@echo "  make testnet updateIndexes USDC XLM"
	@echo "  make testnet setupAllMarkets   (create -> configureOracle -> enable)"
	@echo "  make testnet setupAllEModes"
	@echo "  make testnet setupAll             (markets + emodes)"
	@echo "  make testnet listMarkets"
	@echo "  make testnet listEModeCategories"
	@echo ""
	@echo "Direct operations:"
	@echo "  make grant-role ACCOUNT=G... ROLE=KEEPER|REVENUE|ORACLE NETWORK=testnet"
	@echo "  make update-indexes NETWORK=testnet ASSETS='[\"C...\",\"C...\"]'"
	@echo "  make pause NETWORK=testnet"
	@echo "  make view FN=get_all_markets_detailed NETWORK=testnet"
	@echo ""
	@echo "  SIGNER=ledger make mainnet setupAll    (Ledger signing)"

.DEFAULT_GOAL := help
