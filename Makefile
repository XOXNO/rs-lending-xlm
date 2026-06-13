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
#   make install-stellar-cli  Install pinned stellar-cli (matches CI version)
#
# Deployment (requires stellar CLI + funded account):
#   make testnet deploy             Deploy all contracts to testnet
#   make mainnet deploy             Deploy all contracts to mainnet
#   make testnet upgradeController  Upgrade controller in-place on testnet
#   make testnet upgradeAll         Upgrade pool template, controller, pools, then unpause
#   make testnet setup              Deploy + configure markets/e-modes, then unpause
#   make mainnet setup              Deploy + configure markets/e-modes, then unpause
#
# Ledger signing:
#   SIGNER=ledger make testnet deploy
# ===========================================================================

SHELL := /bin/bash
.PHONY: \
        build build-one optimize deploy-artifacts certora-wasm wasm-artifacts \
        certora certora-list \
        test test-verbose test-one test-match test-pool \
        miri-common miri-pool miri-controller miri-all \
        coverage coverage-controller coverage-pool coverage-merged \
        coverage-report coverage-report-controller coverage-report-pool coverage-report-merged \
        fmt fmt-check clippy clippy-contracts clippy-fuzz \
        wasm-size-check mutants clean install-stellar-cli \
        fuzz fuzz-contract fuzz-one fuzz-build fuzz-seed-corpus \
        fuzz-coverage fuzz-coverage-all fuzz-coverage-one fuzz-coverage-clean \
        proptest proptest-one proptest-build \
        keygen deploy-testnet deploy-mainnet upgrade-pool-template upgrade-controller upgrade-pools upgrade-all _deploy \
        _preflight-tools _preflight-network-config _preflight-setup _preflight-controller _preflight-governance _preflight-pool-hash \
        _preflight-configure-controller _preflight-upgrade-pools _post-setup-status \
        build-flash-loan-receiver deploy-flash-loan-receiver fund-flash-loan-receiver test-flash-loan-receiver \
        configure-controller setup-testnet setup-mainnet _setup-markets create-market \
        update-indexes \
        info invoke invoke-id view view-id \
        testnet mainnet \
        help

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

WASM_TARGET  := wasm32v1-none
RELEASE_DIR  := target/$(WASM_TARGET)/release
# Wasm shadow-stack size. rustc's default is 1MB (16 pages of linear memory),
# and Soroban charges a callee's full initial linear memory against the tx
# MEMORY budget on EVERY cross-contract invocation — measured ~1.28MB/call,
# ~70% of the per-oracle-feed cost of HF-checked ops. 16KB collapses the
# declared memory from 17 pages to ONE (stack + static data fit in 64KB) and
# the full test-harness suite (637 tests incl. max-position liquidations)
# passes with no overflow. Layout is stack-first, so an overflow TRAPS — it
# cannot silently corrupt the data section.
WASM_STACK_SIZE ?= 16384
WASM_RUSTFLAGS := -C link-arg=-zstack-size=$(WASM_STACK_SIZE)
OPTIMIZED_DIR := target/optimized
# Canonical WASM output: deploy/ for mainnet, certora/ for hosted prover (prebuilt).
WASM_ARTIFACTS_DIR := artifacts/wasm
DEPLOY_DIR := $(WASM_ARTIFACTS_DIR)/deploy
CERTORA_WASM_DIR := $(WASM_ARTIFACTS_DIR)/certora
CERTORA_BUILD_DIR := target/certora-build
COV_DIR := target/coverage
TEST_HARNESS_DIR := verification/test-harness
FUZZ_DIR := verification/fuzz

# Contract crates (order matters for deployment)
CONTRACTS := pool controller governance

# Coverage exclusions (no executable code / stubs only).
# Exclude test scaffolding (verification/test-harness internals, the Certora
# spec layer, vendored cvlr/OZ crates) and trivial type-alias files that have
# no executable lines. Protocol code in `common/`, `contracts/`, and
# `interfaces/` stays in scope.
COV_IGNORE := --ignore-filename-regex='(^|/)(verification/test-harness|verification/certora|vendor|target)/|common/src/types/(shared|aggregator)\.rs$$'

# Network config (override via env or CLI, for example `make SIGNER=ledger mainnet setupAll`)
NETWORK     ?= testnet
SIGNER      ?= deployer
CONTRACT    ?= controller
CONFIG_DIR  ?= configs
FLASH_MARKET ?= XLM
FLASH_LOAN_AMOUNT ?= 10000000
FLASH_RECEIVER_FUND ?= 10000000
POOL_WASM_HASH_FILE ?= target/pool_wasm_hash.txt
POOL_UPGRADE_WASM_HASH_FILE ?= target/pool_upgrade_wasm_hash.txt
CONTROLLER_WASM_HASH_FILE ?= target/controller_wasm_hash.txt
SIGNER_ADDRESS = $$(stellar keys public-key $(SIGNER) 2>/dev/null || stellar keys address $(SIGNER) 2>/dev/null || echo $(SIGNER))

# Stellar CLI source account flag
ifeq ($(SIGNER),ledger)
  SOURCE_FLAG = --source-account $(SIGNER_ADDRESS) --sign-with-ledger
else
  SOURCE_FLAG = --source $(SIGNER)
endif

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

## Build all contracts (WASM release)
build:
	@echo "Building all contracts (stack-size $(WASM_STACK_SIZE))..."
	CARGO_BUILD_RUSTFLAGS="$(WASM_RUSTFLAGS)" stellar contract build
	@echo ""
	@echo "WASM artifacts:"
	@ls -lh $(RELEASE_DIR)/*.wasm 2>/dev/null || ls -lh target/wasm32-unknown-unknown/release/*.wasm 2>/dev/null || echo "  (none found)"

## Build a single contract: make build-one CRATE=controller
build-one:
	@echo "Building $(CRATE) (stack-size $(WASM_STACK_SIZE))..."
	CARGO_BUILD_RUSTFLAGS="$(WASM_RUSTFLAGS)" stellar contract build --package $(CRATE)

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

## Create stripped deploy artifacts from optimized WASM. Spec doc strings
## are removed (scripts/strip_spec_docs.py): they count against the network's
## contractMaxSizeBytes and the reference docs live in the interface crates.
deploy-artifacts: optimize
	@mkdir -p $(DEPLOY_DIR)
	@for contract in $(CONTRACTS); do \
		src="$(OPTIMIZED_DIR)/$$contract.wasm"; \
		dst="$(DEPLOY_DIR)/$$contract.wasm"; \
		python3 scripts/strip_spec_docs.py "$$src" "$$dst" || cp "$$src" "$$dst"; \
	done
	@$(MAKE) --no-print-directory _wasm-manifest DEPLOY=1
	@echo ""
	@echo "Deploy WASM ($(DEPLOY_DIR)):"
	@ls -lh $(DEPLOY_DIR)/*.wasm 2>/dev/null

## Build certora-feature WASM for hosted prover (no stellar optimize; spec hooks preserved).
## Stellar's post-build optimizer can produce WASM that passes wasm-validate but crashes
## Certora's GC stack checker on large controller binaries (FunctionIndex_* ref stack errors).
certora-wasm:
	@mkdir -p $(CERTORA_WASM_DIR)
	@for pkg in common pool controller; do \
		echo "Building certora $$pkg (optimize=false)..."; \
		CARGO_TARGET_DIR="$(CERTORA_BUILD_DIR)/$$pkg" \
			stellar contract build --package $$pkg --features certora --optimize=false; \
		src="$(CERTORA_BUILD_DIR)/$$pkg/$(WASM_TARGET)/release/$$pkg.wasm"; \
		dst="$(CERTORA_WASM_DIR)/$$pkg.wasm"; \
		/bin/cp -f "$$src" "$$dst"; \
	done
	@$(MAKE) --no-print-directory _wasm-manifest CERTORA=1
	@echo ""
	@echo "Certora WASM ($(CERTORA_WASM_DIR)):"
	@ls -lh $(CERTORA_WASM_DIR)/*.wasm 2>/dev/null

## Production deploy WASM + certora prover WASM (local build once, cloud proves).
wasm-artifacts: deploy-artifacts certora-wasm
	@echo ""
	@echo "All WASM artifacts under $(WASM_ARTIFACTS_DIR)/"

# Certora hosted prover (requires CERTORAKEY, certora-cli, and certora WASM).
CERTORA_PROFILE ?= sanity

## List Certora verification profiles.
certora-list:
	@./verification/certora/run_profile.py --list

## Submit profile to Certora cloud: make certora [CERTORA_PROFILE=fast]
certora: certora-wasm
	@test -n "$$CERTORAKEY" || { echo "CERTORAKEY is not set"; exit 1; }
	@command -v certoraSorobanProver >/dev/null 2>&1 || { \
		echo "certoraSorobanProver not found; install with: pip install certora-cli"; \
		exit 1; \
	}
	@./verification/certora/scripts/run-all.sh $(CERTORA_PROFILE) $(CERTORA_ARGS)

_wasm-manifest:
	@python3 verification/certora/scripts/write_wasm_manifest.py \
		$(if $(DEPLOY),--deploy,) \
		$(if $(CERTORA),--certora,)

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

## Run Miri on pool::interest pure-arithmetic paths.
miri-pool:
	@cd contracts/pool && MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-symbolic-alignment-check" \
		cargo +nightly miri test --lib -- \
		interest::

## Run Miri on controller::helpers pure-arithmetic paths.
miri-controller:
	@cd contracts/controller && MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-symbolic-alignment-check" \
		cargo +nightly miri test --lib -- \
		helpers::

## Run all Miri checks (common + pool + controller pure paths).
miri-all: miri-common miri-pool miri-controller

# ---------------------------------------------------------------------------
# Coverage
# ---------------------------------------------------------------------------

## Run coverage and print summary to CLI
coverage: coverage-merged

coverage-controller:
	@echo "Running controller coverage (common + controller unit tests + test-harness)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@cargo llvm-cov test -p common --lib --no-report $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov test -p controller --lib --no-report $(COV_IGNORE) 2>&1 | tail -5
	@backup="$(COV_DIR)/snapshots-backup"; \
	restore_snapshots() { \
		rm -rf $(TEST_HARNESS_DIR)/test_snapshots; \
		mkdir -p $(TEST_HARNESS_DIR)/test_snapshots; \
		cp -R "$$backup"/. $(TEST_HARNESS_DIR)/test_snapshots/; \
	}; \
	rm -rf "$$backup" && mkdir -p "$$backup"; \
	cp -R $(TEST_HARNESS_DIR)/test_snapshots/. "$$backup"/; \
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
	@echo "Running merged coverage (common + controller + pool + test-harness)..."
	@mkdir -p $(COV_DIR)
	@cargo llvm-cov clean --workspace
	@cargo llvm-cov test -p common --lib --no-report $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov test -p pool --no-report $(COV_IGNORE) 2>&1 | tail -5
	@cargo llvm-cov test -p controller --lib --no-report $(COV_IGNORE) 2>&1 | tail -5
	@backup="$(COV_DIR)/snapshots-backup"; \
	restore_snapshots() { \
		rm -rf $(TEST_HARNESS_DIR)/test_snapshots; \
		mkdir -p $(TEST_HARNESS_DIR)/test_snapshots; \
		cp -R "$$backup"/. $(TEST_HARNESS_DIR)/test_snapshots/; \
	}; \
	rm -rf "$$backup" && mkdir -p "$$backup"; \
	cp -R $(TEST_HARNESS_DIR)/test_snapshots/. "$$backup"/; \
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

## Lint the fuzz crate (excluded from the workspace).
clippy-fuzz:
	cargo clippy --manifest-path $(FUZZ_DIR)/Cargo.toml --all-targets -- -D warnings

# ---------------------------------------------------------------------------
# WASM size budget
# ---------------------------------------------------------------------------
# Thresholds live in `configs/wasm_size_budget.txt`.

WASM_BUDGET_FILE ?= configs/wasm_size_budget.txt

## Fail if any release WASM exceeds the committed budget.
wasm-size-check: build
	@if [ ! -f $(WASM_BUDGET_FILE) ]; then \
		echo "WASM budget file missing: $(WASM_BUDGET_FILE)"; \
		echo "Create one with 'path bytes' lines (one per contract)."; \
		exit 1; \
	fi
	@status=0; \
	while IFS=' ' read -r rel_path budget; do \
		case "$$rel_path" in ''|\#*) continue ;; esac; \
		path="$(RELEASE_DIR)/$$rel_path"; \
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

# ---------------------------------------------------------------------------
# Mutation testing
# ---------------------------------------------------------------------------

## Run cargo-mutants on common/ + controller/src/helpers/.
## We deliberately exclude verification/certora spec files because they
## contain large amounts of test-only / modeling code that produces noisy
## "missed" mutants and are not part of the production attack surface we
## care about protecting with mutation testing.
mutants:
	@command -v cargo-mutants >/dev/null 2>&1 || { \
		echo "cargo-mutants not installed. Install with:"; \
		echo "  cargo install cargo-mutants --locked"; \
		exit 1; \
	}
	cargo mutants --package common --package controller \
		--file 'common/src/**/*.rs' \
		--file 'contracts/controller/src/helpers/**/*.rs' \
		--exclude '**/verification/**' \
		--exclude '**/certora/**' \
		--jobs 1

# ---------------------------------------------------------------------------
# Clean
# ---------------------------------------------------------------------------

## Clean all build artifacts
clean:
	cargo clean
	rm -rf $(OPTIMIZED_DIR)
	rm -rf $(WASM_ARTIFACTS_DIR)
	rm -rf $(CERTORA_BUILD_DIR)
	rm -rf $(COV_DIR)

# ---------------------------------------------------------------------------
# Tools (CI parity for local development)
# ---------------------------------------------------------------------------

## Install the exact stellar-cli version used across CI, fuzz, Certora, and release
## workflows (26.0.0, matching soroban-sdk pin and rust-toolchain.toml).
## The helper script is platform-aware (Linux + macOS darwin) and idempotent.
install-stellar-cli:
	STELLAR_VERSION=26.0.0 bash .github/scripts/install-stellar-cli.sh

# ---------------------------------------------------------------------------
# Fuzzing (function-level math primitives)
# ---------------------------------------------------------------------------

FUZZ_TARGETS := fp_math rates_and_index fp_ops
FUZZ_CONTRACT_TARGETS := flow_e2e flow_strategy pool_native
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
		cargo +nightly fuzz run --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS) $$t -- -max_total_time=$(FUZZ_TIME) 2>&1 | tail -3; \
	done

## Run all contract-level libFuzzer targets for $(FUZZ_TIME) seconds each.
fuzz-contract:
	@for t in $(FUZZ_CONTRACT_TARGETS); do \
		echo "=== $$t ==="; \
		cargo +nightly fuzz run --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS) $$t -- -max_total_time=$(FUZZ_TIME) 2>&1 | tail -3; \
	done

## Run a single fuzz target: make fuzz-one TARGET=fp_math FUZZ_TIME=300
fuzz-one:
	@cargo +nightly fuzz run --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS) $(TARGET) -- -max_total_time=$(FUZZ_TIME)

## Build all fuzz targets (compile-only)
fuzz-build:
	@cargo +nightly fuzz build --fuzz-dir $(FUZZ_DIR) $(FUZZ_FLAGS)

## Seed verification/fuzz/corpus/<target>/ from */test_snapshots/**/*.json. Run once before
## a campaign to give libFuzzer realistic numeric entropy from the start.
fuzz-seed-corpus:
	@cd $(FUZZ_DIR) && cargo run --release --features seed-corpus --bin seed_corpus -- --output corpus

# ---------------------------------------------------------------------------
# Fuzz coverage (fast: corpus replay only, no active fuzzing)
# ---------------------------------------------------------------------------
# `cargo fuzz coverage` builds with profile instrumentation and replays the
# existing corpus once — inherently fast once the build is warm. HTML reports
# land in $(COV_DIR)/fuzz/<target>/. Set FUZZ_COV_TIME=<seconds> to do a short
# fuzz run first (grows the corpus before measuring).
#
# macOS: all targets need --sanitizer=thread -Zbuild-std because the default
# sancov+ASAN build fails to link the stellar-access cdylib (same workaround
# used by `make fuzz`). First build is slow (~2–5 min); subsequent runs reuse
# the cache so replay + report complete in seconds.

FUZZ_COV_TIME ?= 0
ifeq ($(UNAME_S),Darwin)
  FUZZ_COV_ENV := SANITIZER=thread BUILD_STD=1
else
  FUZZ_COV_ENV :=
endif

## Fast: coverage for function-level targets (fp_math, rates_and_index)
fuzz-coverage:
	@$(FUZZ_COV_ENV) FUZZ_COV_TIME=$(FUZZ_COV_TIME) \
		./$(FUZZ_DIR)/coverage.sh $(FUZZ_TARGETS)

## All: adds contract-level targets — same flags, same cache, just more targets
fuzz-coverage-all:
	@$(FUZZ_COV_ENV) FUZZ_COV_TIME=$(FUZZ_COV_TIME) \
		./$(FUZZ_DIR)/coverage.sh $(FUZZ_TARGETS) $(FUZZ_CONTRACT_TARGETS)

## Single target: make fuzz-coverage-one TARGET=flow_e2e [FUZZ_COV_TIME=30]
fuzz-coverage-one:
	@if [ -z "$(TARGET)" ]; then \
		echo "Usage: make fuzz-coverage-one TARGET=<name> [FUZZ_COV_TIME=30]"; \
		exit 1; \
	fi
	@$(FUZZ_COV_ENV) FUZZ_COV_TIME=$(FUZZ_COV_TIME) \
		./$(FUZZ_DIR)/coverage.sh $(TARGET)

## Remove fuzz coverage artifacts (keeps the corpus)
fuzz-coverage-clean:
	@rm -rf $(COV_DIR)/fuzz $(FUZZ_DIR)/coverage

# ---------------------------------------------------------------------------
# Contract-level property tests (proptest inside test-harness)
# ---------------------------------------------------------------------------

PROPTEST_CASES ?= 256

## Run all contract-level property tests (`verification/test-harness/tests/fuzz/`).
## Set PROPTEST_CASES=10000 (or higher) for longer runs on dedicated hardware.
proptest:
	@echo "=== fuzz (proptest) ==="
	@PROPTEST_CASES=$(PROPTEST_CASES) cargo test --release -p test-harness --test fuzz -- --test-threads=1

## Run a single property: make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=10000
proptest-one:
	@PROPTEST_CASES=$(PROPTEST_CASES) cargo test --release -p test-harness --test fuzz $(TEST) -- --test-threads=1

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

_preflight-tools:
	@command -v stellar >/dev/null 2>&1 || { echo "Missing required tool: stellar"; exit 1; }
	@command -v jq >/dev/null 2>&1 || { echo "Missing required tool: jq"; exit 1; }

_preflight-network-config: _preflight-tools
	@test -f $(CONFIG_DIR)/networks.json || { echo "Config file not found: $(CONFIG_DIR)/networks.json"; exit 1; }
	@jq -e '.["$(NETWORK)"] != null' $(CONFIG_DIR)/networks.json >/dev/null || { echo "Network $(NETWORK) not found in $(CONFIG_DIR)/networks.json"; exit 1; }
	@test -f $(CONFIG_DIR)/$(NETWORK)_markets.json || { echo "Config file not found: $(CONFIG_DIR)/$(NETWORK)_markets.json"; exit 1; }
	@jq -e '.markets | type == "array" and length > 0' $(CONFIG_DIR)/$(NETWORK)_markets.json >/dev/null || { echo "No configured markets in $(CONFIG_DIR)/$(NETWORK)_markets.json"; exit 1; }
	@jq -e 'all(.markets[]; (.name // "") != "" and (.asset_address // "") != "")' $(CONFIG_DIR)/$(NETWORK)_markets.json >/dev/null || { echo "Every configured market must have name and asset_address"; exit 1; }
	@test -f $(CONFIG_DIR)/emodes.json || { echo "Config file not found: $(CONFIG_DIR)/emodes.json"; exit 1; }
	@jq -e '.["$(NETWORK)"] | type == "object"' $(CONFIG_DIR)/emodes.json >/dev/null || { echo "E-mode config for $(NETWORK) not found in $(CONFIG_DIR)/emodes.json"; exit 1; }

_preflight-setup: _preflight-network-config
	@AGG=$$(jq -r '.["$(NETWORK)"].aggregator // empty' $(CONFIG_DIR)/networks.json); \
	if [ -z "$$AGG" ] || [ "$$AGG" = "null" ]; then \
		echo "Aggregator not configured for $(NETWORK) in $(CONFIG_DIR)/networks.json"; \
		exit 1; \
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
		echo "Pool WASM hash not found. Run deploy/upgrade-pool-template first or set configs/networks.json."; \
		exit 1; \
	fi

_preflight-configure-controller: _preflight-setup _preflight-controller _preflight-governance

_preflight-upgrade-pools: _preflight-controller _preflight-governance _preflight-pool-hash

_post-setup-status:
	@echo ""
	@echo "=== Setup status ($(NETWORK)) ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh info
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh listMarkets
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh listEModeCategories

## Deploy all contracts to a network
deploy-testnet: NETWORK=testnet
deploy-testnet: _deploy

deploy-mainnet: NETWORK=mainnet
deploy-mainnet: _deploy

## Upgrade the deployed controller contract in-place via governance.
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
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].controller_wasm_hash = "'$$HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json; \
	echo "Governance: $$GOV"; \
	echo "New controller WASM hash: $$HASH"; \
	stellar contract invoke --id $$GOV $(SOURCE_FLAG) --network $(NETWORK) \
		-- upgrade_controller --new_wasm_hash $$HASH

## Upload the latest pool WASM and set it as the pool template via governance.
upgrade-pool-template: _preflight-controller _preflight-governance deploy-artifacts
	@echo "=== Upgrading pool template on $(NETWORK) ==="
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
	echo "New pool template WASM hash: $$HASH"; \
	stellar contract invoke --id $$GOV $(SOURCE_FLAG) --network $(NETWORK) \
		-- set_liquidity_pool_template --hash $$HASH; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].pool_wasm_hash = "'$$HASH'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json

## Upgrade the central liquidity pool to the latest pool template hash via governance.
upgrade-pools: _preflight-upgrade-pools
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
	HASH=$$(if [ -s $(POOL_UPGRADE_WASM_HASH_FILE) ]; then cat $(POOL_UPGRADE_WASM_HASH_FILE); else jq -r ".\"$(NETWORK)\".pool_wasm_hash // empty" $(CONFIG_DIR)/networks.json; fi); \
	if [ -z "$$HASH" ] || [ "$$HASH" = "null" ]; then \
		echo "Pool WASM hash not found. Run upgrade-pool-template first."; \
		exit 1; \
	fi; \
	echo "Governance: $$GOV"; \
	echo "Pool WASM hash: $$HASH"; \
	stellar contract invoke --id $$GOV $(SOURCE_FLAG) --network $(NETWORK) \
		-- upgrade_pool --new_wasm_hash $$HASH

## Upload pool template, upgrade controller, upgrade the central pool, then unpause.
upgrade-all: upgrade-pool-template upgrade-controller upgrade-pools _unpause-after-setup _post-setup-status

## Build the flash-loan receiver test contract for network smoke testing.
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

## Deploy the latest flash-loan receiver test contract and record its address.
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

## Fund the deployed flash-loan receiver with the selected market asset.
fund-flash-loan-receiver:
	@echo "=== Funding flash-loan receiver on $(NETWORK) ==="
	@ASSET=$$(jq -r '.markets[] | select(.name == "$(FLASH_MARKET)") | .asset_address' $(CONFIG_DIR)/$(NETWORK)_markets.json); \
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

## Run testnet flash-loan smoke cases against the deployed receiver.
test-flash-loan-receiver:
	@echo "=== Flash-loan receiver smoke test on $(NETWORK) ==="
	@CTRL=$$(stellar contract alias show controller --network $(NETWORK) 2>/dev/null | tail -n1); \
	if [ -z "$$CTRL" ]; then \
		CTRL=$$(jq -r ".\"$(NETWORK)\".controller // empty" $(CONFIG_DIR)/networks.json); \
	fi; \
	ASSET=$$(jq -r '.markets[] | select(.name == "$(FLASH_MARKET)") | .asset_address' $(CONFIG_DIR)/$(NETWORK)_markets.json); \
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
	if [ -z "$$RECEIVER" ] || [ "$$RECEIVER" = "null" ]; then \
		echo "Flash receiver not found. Run deploy-flash-loan-receiver first."; \
		exit 1; \
	fi; \
	echo "Controller: $$CTRL"; \
	echo "Receiver: $$RECEIVER"; \
	echo "Asset: $$ASSET ($(FLASH_MARKET))"; \
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
			--asset $$ASSET \
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
	@echo "1/6 Checking Aggregator..."
	@AGGREGATOR=$$(jq -r ".\"$(NETWORK)\".aggregator" $(CONFIG_DIR)/networks.json 2>/dev/null); \
	if [ ! -z "$$AGGREGATOR" ] && [ "$$AGGREGATOR" != "null" ] && [ "$$AGGREGATOR" != "" ]; then \
		echo "Using Aggregator: $$AGGREGATOR"; \
		stellar contract alias add aggregator --id $$AGGREGATOR --network $(NETWORK) --overwrite || echo "Warning: Failed to set aggregator alias"; \
	else \
		echo "Skipping Aggregator setup (not configured or invalid)"; \
	fi
	@echo ""
	@# 2. Upload Pool WASM (template, not deployed directly)
	@echo "2/6 Uploading Pool WASM template..."
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
	@# 3. Upload controller WASM so governance deploys a network-installed hash.
	@echo "3/6 Uploading Controller WASM..."
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
	@# 4. Deploy Governance with the deployer EOA as admin/owner.
	@echo "4/6 Deploying Governance..."
	@stellar contract deploy \
		--wasm $(DEPLOY_DIR)/governance.wasm \
		$(SOURCE_FLAG) \
		--network $(NETWORK) \
		--alias governance \
		-- --admin $(SIGNER_ADDRESS)
	@GOV_ID=$$(stellar contract alias show governance --network $(NETWORK) | tail -n1); \
	if [ -z "$$GOV_ID" ]; then echo "Governance alias not resolvable after deploy"; exit 1; fi; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].governance = "'$$GOV_ID'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@# 5. Deploy Controller through governance — governance becomes its owner.
	@# The CLI prints the returned address as a quoted strkey on the last line.
	@echo "5/6 Deploying Controller via governance..."
	@GOV_ID=$$(stellar contract alias show governance --network $(NETWORK) | tail -n1); \
	CTRL_ID=$$(stellar contract invoke --id $$GOV_ID $(SOURCE_FLAG) --network $(NETWORK) \
		-- deploy_controller --wasm_hash $$(cat $(CONTROLLER_WASM_HASH_FILE)) | tail -n1 | tr -d '"'); \
	if [ -z "$$CTRL_ID" ]; then echo "deploy_controller returned no address"; exit 1; fi; \
	echo "Controller: $$CTRL_ID"; \
	stellar contract alias add controller --id $$CTRL_ID --network $(NETWORK) --overwrite; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].controller = "'$$CTRL_ID'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@# 6. Set the pool template and deploy the central pool through governance.
	@echo "6/6 Setting pool template and deploying central pool via governance..."
	@GOV_ID=$$(stellar contract alias show governance --network $(NETWORK) | tail -n1); \
	stellar contract invoke --id $$GOV_ID $(SOURCE_FLAG) --network $(NETWORK) \
		-- set_liquidity_pool_template --hash $$(cat $(POOL_WASM_HASH_FILE)); \
	POOL=$$(stellar contract invoke --id $$GOV_ID $(SOURCE_FLAG) --network $(NETWORK) \
		-- deploy_pool | tail -n1 | tr -d '"'); \
	if [ -z "$$POOL" ]; then echo "deploy_pool returned no address"; exit 1; fi; \
	echo "Central pool: $$POOL"; \
	TMP_JSON=$$(mktemp); \
	jq '.["$(NETWORK)"].pool = "'$$POOL'"' \
		$(CONFIG_DIR)/networks.json > $$TMP_JSON && mv $$TMP_JSON $(CONFIG_DIR)/networks.json
	@echo ""
	@echo "=== Deployment complete ==="
	@echo "Aggregator:     $$(stellar contract alias show aggregator --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Governance:     $$(stellar contract alias show governance --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Controller:     $$(stellar contract alias show controller --network $(NETWORK) 2>/dev/null || echo 'check aliases')"
	@echo "Pool:           $$(jq -r '.["$(NETWORK)"].pool // empty' $(CONFIG_DIR)/networks.json)"
	@echo "Pool WASM Hash: $$(cat $(POOL_WASM_HASH_FILE))"
	@echo "Controller WASM Hash: $$(cat $(CONTROLLER_WASM_HASH_FILE))"

## Configure controller after deployment (all admin calls route via governance)
configure-controller: _preflight-configure-controller
	@echo "=== Configuring Controller via governance on $(NETWORK) ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh setAggregator
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh setAccumulator
	@# The controller constructor auto-grants KEEPER to governance (its admin).
	@# The deployer EOA needs explicit controller roles: KEEPER for
	@# update_indexes, ORACLE for disable_token_oracle, REVENUE for
	@# claim_revenue. Governance's own ORACLE role (configure_market_oracle)
	@# is granted to the deployer by the governance constructor.
	@echo "Granting controller KEEPER role to deployer..."
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh grantRole $(SIGNER_ADDRESS) KEEPER
	@echo "Granting controller ORACLE role to deployer..."
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh grantRole $(SIGNER_ADDRESS) ORACLE
	@echo "Granting controller REVENUE role to deployer..."
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh grantRole $(SIGNER_ADDRESS) REVENUE
	@echo "Controller configured."

## Full setup: deploy + configure + create/configure markets and e-modes, then unpause.
## Constructor pauses the controller; the final unpause turns the protocol live.
setup-testnet: NETWORK=testnet
setup-testnet: _preflight-setup deploy-testnet configure-controller _setup-markets _unpause-after-setup _post-setup-status

setup-mainnet: NETWORK=mainnet
setup-mainnet: _preflight-setup deploy-mainnet configure-controller _setup-markets _unpause-after-setup _post-setup-status

_unpause-after-setup:
	@echo "=== Unpausing $(NETWORK) protocol via governance ==="
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh unpause

_setup-markets:
	@echo "=== Setting up markets from $(CONFIG_DIR)/$(NETWORK)_markets.json ==="
	@if [ ! -f $(CONFIG_DIR)/$(NETWORK)_markets.json ]; then \
		echo "Config file not found: $(CONFIG_DIR)/$(NETWORK)_markets.json"; \
		echo "Create it based on configs/devnet_market_configs.json pattern."; \
		exit 1; \
	fi
	@NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh setupAll

## Create a single market via governance (interactive)
create-market:
	@echo "Creating market for $(ASSET) on $(NETWORK)..."
	@GOV=$$(stellar contract alias show governance --network $(NETWORK)); \
	stellar contract invoke --id $$GOV $(SOURCE_FLAG) --network $(NETWORK) \
		-- create_liquidity_pool \
		--asset $(ASSET_ADDRESS) \
		--params '$(MARKET_PARAMS)' \
		--config '$(ASSET_CONFIG)'

# ---------------------------------------------------------------------------
# Config-driven operations (via configs/script.sh)
#
# Single unified dispatcher: `make <network> <action> [positional args]`
# All values are read from JSON configs by name; positional args reference
# markets / categories / accounts by their config name or id.
#
# Examples:
#   make testnet addEModeCategory 1
#   make testnet addAssetToEMode 1 USDC
#   make testnet createMarket USDC
#   make testnet updateIndexes USDC XLM
#   make testnet setupAll
#   make testnet pause
#   make testnet unpause
#   make testnet grantRole GAB...XYZ KEEPER
#   make testnet getPrice USDC
#   make testnet getHealth 1
#   make testnet getCollateral 1 XLM
#   SIGNER=ledger make mainnet setupAll
# ---------------------------------------------------------------------------

# Action classification — the dispatcher routes each action to script.sh
# passing positional args verbatim. Adding a new verb = add here + script.sh.
SIMPLE_ACTIONS := listMarkets listEModeCategories \
                  setupAll setupAllMarkets setupAllEModes \
                  setAggregator setAccumulator pause unpause info \
                  getAllMarkets getAllIndexes \
                  claimRevenueAll
POSITIONAL_MARKET_ACTIONS := createMarket editAssetConfig updateMarketParams \
                             configureMarketOracle \
                             getPrice getMarket getIndex getIsolatedDebt \
                             getReflector
POSITIONAL_ID_ACTIONS := addEModeCategory getEMode
POSITIONAL_ID_ASSET_ACTIONS := addAssetToEMode
POSITIONAL_ACCOUNT_ACTIONS := getHealth getAccount getCollateralUsd getBorrowUsd \
                              getLtvUsd getLiqAvailable canLiquidate
POSITIONAL_ACCOUNT_MARKET_ACTIONS := getCollateral getBorrow
POSITIONAL_ACCOUNT_ROLE_ACTIONS := grantRole revokeRole hasRole grantGovRole revokeGovRole
REFLECTOR_PROBE_ACTIONS := queryReflector queryReflectorPrice queryReflectorTwap
VARARG_ACTIONS := updateIndexes claimRevenue supply borrow

# Makefile-internal actions — handled directly by make targets, not forwarded
# to configs/script.sh (they manipulate WASM artifacts and deploy pipelines).
MAKEFILE_ACTIONS := deploy upgradeController upgradePoolTemplate upgradePools upgradeAll \
                    deployFlashReceiver fundFlashReceiver testFlashReceiver setup

ALL_ACTIONS := $(SIMPLE_ACTIONS) $(POSITIONAL_MARKET_ACTIONS) $(POSITIONAL_ID_ACTIONS) \
               $(POSITIONAL_ID_ASSET_ACTIONS) $(POSITIONAL_ACCOUNT_ACTIONS) \
               $(POSITIONAL_ACCOUNT_MARKET_ACTIONS) $(POSITIONAL_ACCOUNT_ROLE_ACTIONS) \
               $(REFLECTOR_PROBE_ACTIONS) $(VARARG_ACTIONS) $(MAKEFILE_ACTIONS)

.PHONY: $(ALL_ACTIONS)

# Network dispatcher — routes each action either to an internal Makefile
# target (MAKEFILE_ACTIONS) or forwards it verbatim to configs/script.sh.
# All action targets below are no-ops so Make accepts the remaining words
# on the command line.
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
				upgradePoolTemplate) $(MAKE) --no-print-directory upgrade-pool-template NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradePools)       $(MAKE) --no-print-directory upgrade-pools NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				upgradeAll)         $(MAKE) --no-print-directory upgrade-all NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				deployFlashReceiver) $(MAKE) --no-print-directory deploy-flash-loan-receiver NETWORK=$(1) SIGNER=$(SIGNER) ;; \
				fundFlashReceiver)  $(MAKE) --no-print-directory fund-flash-loan-receiver NETWORK=$(1) SIGNER=$(SIGNER) FLASH_MARKET=$(FLASH_MARKET) FLASH_RECEIVER_FUND=$(FLASH_RECEIVER_FUND) ;; \
				testFlashReceiver)  $(MAKE) --no-print-directory test-flash-loan-receiver NETWORK=$(1) SIGNER=$(SIGNER) FLASH_MARKET=$(FLASH_MARKET) FLASH_LOAN_AMOUNT=$(FLASH_LOAN_AMOUNT) ;; \
				setup)              $(MAKE) --no-print-directory _preflight-setup _deploy configure-controller _setup-markets _unpause-after-setup _post-setup-status NETWORK=$(1) SIGNER=$(SIGNER) ;; \
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

# All action verbs are no-op targets so Make accepts them as positional words
# after `testnet` / `mainnet`. Invoking them directly (e.g. `make getPrice`)
# is intentionally unsupported — always go through a network target.
$(ALL_ACTIONS):
	@:

# Catch-all for any remaining positional args (market names, ids, addresses).
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
# Contract inspection (named-parameter escape hatches for ad-hoc calls)
# ---------------------------------------------------------------------------

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
	@echo "  make deploy-artifacts   Optimized WASM for mainnet ($(DEPLOY_DIR))"
	@echo "  make certora-wasm       Certora-feature WASM for hosted prover"
	@echo "  make wasm-artifacts     Build deploy + certora WASM ($(WASM_ARTIFACTS_DIR))"
	@echo "  make certora            Submit Certora cloud jobs (CERTORA_PROFILE=sanity)"
	@echo "  make certora-list       List Certora profiles"
	@echo "  make test               Run all test-harness tests"
	@echo "  make test-one FILE=x    Run specific test file"
	@echo "  make coverage           Run merged coverage with CLI summary"
	@echo "  make fuzz-coverage      Fast fuzz coverage (fp_math, rates_and_index) — corpus replay only"
	@echo "  make fuzz-coverage-all  Include contract-level targets (slower on macOS: TSAN build)"
	@echo "  make fuzz-coverage-one TARGET=flow_e2e [FUZZ_COV_TIME=30]"
	@echo "  make coverage-controller  Coverage for controller/common via unit+harness"
	@echo "  make coverage-pool        Coverage for pool via direct unit tests"
	@echo "  make coverage-merged      Coverage merged across pool + controller + harness"
	@echo "  make coverage-report      Generate merged LCOV + Markdown reports"
	@echo "  make fmt                Format code"
	@echo "  make clippy             Lint all targets with warnings denied"
	@echo "  make clean              Clean artifacts"
	@echo ""
	@echo "Deployment (pattern: make <network> <action>, network = testnet | mainnet):"
	@echo "  make keygen                         Generate deployer key"
	@echo "  make setup-testnet                  Same as 'make testnet setup'"
	@echo "  make testnet deploy                 Deploy all contracts"
	@echo "  make testnet upgradeController      Upgrade controller WASM in-place"
	@echo "  make testnet upgradeAll             Upgrade pool template, controller, all pools, then unpause"
	@echo "  make testnet deployFlashReceiver    Deploy flash-loan test receiver"
	@echo "  make testnet fundFlashReceiver      Fund flash receiver with FLASH_MARKET"
	@echo "  make testnet testFlashReceiver      Run flash receiver smoke cases"
	@echo "  make testnet setup                  Full setup (deploy + config + markets/e-modes + unpause)"
	@echo "  make testnet info                   Show deployed contract IDs"
	@echo ""
	@echo "Config-driven operations (pattern: make <network> <action> [args]):"
	@echo ""
	@echo "  Markets (writes):"
	@echo "    make testnet createMarket USDC"
	@echo "    make testnet editAssetConfig USDC"
	@echo "    make testnet updateMarketParams USDC                       Push max_utilization/rate model from JSON"
	@echo "    make testnet configureMarketOracle USDC"
	@echo "    make testnet updateIndexes USDC XLM"
	@echo "    make testnet setupAllMarkets       Configure markets only; does not deploy or unpause"
	@echo "    make testnet listMarkets"
	@echo ""
	@echo "  E-Mode (writes):"
	@echo "    make testnet addEModeCategory 1"
	@echo "    make testnet addAssetToEMode 1 USDC"
	@echo "    make testnet setupAllEModes        Configure e-modes only; does not deploy or unpause"
	@echo "    make testnet setupAll              Configure markets/e-modes only; does not deploy or unpause"
	@echo "    make testnet listEModeCategories"
	@echo ""
	@echo "  Positions (writes):"
	@echo "    make testnet supply USDC 1000000000                  100 USDC at 7 dec, into account 0"
	@echo "    make testnet borrow USDC 100000000 <account_id>      Direct borrow (no swap)"
	@echo ""
	@echo "  Strategies (multiply / swap_debt / swap_collateral / repay_debt_with_collateral)"
	@echo "  require an AggregatorSwap JSON from the off-chain quote server. Invoke directly:"
	@echo "    make invoke FN=multiply ARGS='--caller G... --account_id 0 ... --swap @swap.json' NETWORK=testnet"
	@echo ""
	@echo "  Protocol control (writes):"
	@echo "    make testnet pause"
	@echo "    make testnet unpause"
	@echo "    make testnet setAggregator"
	@echo "    make testnet grantRole GAB...XYZ KEEPER     Controller roles via governance (KEEPER|REVENUE|ORACLE)"
	@echo "    make testnet revokeRole GAB...XYZ KEEPER"
	@echo "    make testnet grantGovRole GAB...XYZ ORACLE  Governance's own roles (ORACLE = configure_market_oracle)"
	@echo "    make testnet revokeGovRole GAB...XYZ ORACLE"
	@echo "    make testnet claimRevenue USDC XLM          Claim revenue for one or more markets (REVENUE role)"
	@echo "    make testnet claimRevenueAll                Claim revenue for every configured market"
	@echo ""
	@echo "  Quick views (reads, no signing cost):"
	@echo "    make testnet info                      Deployment addresses"
	@echo "    make testnet hasRole GAB... KEEPER"
	@echo "    make testnet getPrice USDC             Spot / safe / aggregator prices"
	@echo "    make testnet getMarket USDC            Full MarketConfig"
	@echo "    make testnet getIndex USDC             Supply / borrow RAY index"
	@echo "    make testnet getIsolatedDebt USDC"
	@echo "    make testnet getAllMarkets"
	@echo "    make testnet getAllIndexes"
	@echo "    make testnet getEMode 1"
	@echo "    make testnet getHealth 1"
	@echo "    make testnet getAccount 1"
	@echo "    make testnet getCollateralUsd 1"
	@echo "    make testnet getBorrowUsd 1"
	@echo "    make testnet getLtvUsd 1"
	@echo "    make testnet getLiqAvailable 1"
	@echo "    make testnet canLiquidate 1"
	@echo "    make testnet getCollateral 1 XLM"
	@echo "    make testnet getBorrow 1 USDC"
	@echo ""
	@echo "  Reflector probes (debug DualOracle wiring):"
	@echo "    make testnet getReflector USDC                                 Live CEX + DEX for a market"
	@echo "    make testnet queryReflector CCYOZJ...MJRN63                    decimals + resolution"
	@echo "    make testnet queryReflectorPrice CCYOZJ... other USDC          lastprice"
	@echo "    make testnet queryReflectorTwap  CCYOZJ... other USDC 3        prices history"
	@echo "    make testnet queryReflectorPrice C...DEX... stellar CBIELTK... lastprice on Stellar DEX"
	@echo ""
	@echo "Escape hatches for ad-hoc calls:"
	@echo "    make view FN=get_market_config ARGS='--asset C...' NETWORK=testnet"
	@echo "    make invoke CONTRACT=governance FN=set_position_limits ARGS='--limits {...}' NETWORK=testnet"
	@echo "    make update-indexes NETWORK=testnet ASSETS='[\"C...\",\"C...\"]'"
	@echo ""
	@echo "Ledger signing (any command):"
	@echo "    SIGNER=ledger make mainnet setupAll"

# --- Mutation testing -----------------------------------------------------
# Requires: cargo install --locked cargo-mutants
# Config:   .cargo/mutants.toml (workspace-wide excludes)
# Output:   mutants.out/ (gitignored)
#
# Per-target scope keeps each invocation under ~10 min. Parallelise with
# MUTANTS_JOBS (defaults to 4 workers).

MUTANTS_JOBS ?= 4
# Test-harness integration tests can run long under accrual-loop mutations.
# 120s is generous enough to disambiguate "infinite loop" from "merely slow".
MUTANTS_TIMEOUT ?= 120

mutants-math:
	cargo mutants --package common --file 'common/src/math/**' -j $(MUTANTS_JOBS)

mutants-rates:
	cargo mutants --package common --file 'common/src/rates.rs' -j $(MUTANTS_JOBS)

mutants-pool-interest:
	cargo mutants --package pool --file 'contracts/pool/src/interest.rs' -j $(MUTANTS_JOBS)

mutants-pool:
	cargo mutants --package pool \
		--test-package pool --test-package test-harness \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) \
		-j $(MUTANTS_JOBS)

mutants-oracle-policy:
	cargo mutants --package controller \
		--file 'contracts/controller/src/oracle/policy.rs' \
		--file 'contracts/controller/src/oracle/compose.rs' \
		--test-package controller --test-package test-harness \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) \
		-j $(MUTANTS_JOBS)

mutants-controller-positions:
	cargo mutants --package controller --file 'contracts/controller/src/positions/**' \
		--test-package controller --test-package test-harness \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) \
		-j $(MUTANTS_JOBS)

mutants-controller-strategies:
	cargo mutants --package controller --file 'contracts/controller/src/strategies/**' \
		--test-package controller --test-package test-harness \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) \
		-j $(MUTANTS_JOBS)

mutants-common:
	cargo mutants --package common -j $(MUTANTS_JOBS)

.DEFAULT_GOAL := help
