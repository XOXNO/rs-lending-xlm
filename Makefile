





































SHELL := /bin/bash
.PHONY: \
        build build-one optimize deploy-artifacts integration-wasm integration-preflight integration-validate integration-shellcheck integration-appendix certora-wasm wasm-artifacts \
        certora certora-list \
        test test-verbose test-one test-match test-pool \
        miri-common miri-pool miri-controller miri-all \
        coverage coverage-controller coverage-pool coverage-price-aggregator coverage-merged \
        fmt fmt-check clippy clippy-contracts clippy-fuzz scout scout-host scout-strict \
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
        usage help





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


WASM_SIZE_CONTRACTS := pool controller governance common flash_loan_receiver defindex_strategy price_aggregator






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
	@for pkg in mock_oracle mock_redstone; do \
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
	cargo test -p test-harness --test $(FILE) -- $(TEST_THREAD_FLAG)


test-match:
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






WASM_BUDGET_FILE ?= configs/wasm_size_budget.txt






wasm-testing-abi-check: deploy-artifacts
	@gov="$(DEPLOY_DIR)/governance.wasm"; \
	if [ ! -f "$$gov" ]; then echo "governance deploy WASM missing: $$gov"; exit 1; fi; \
	if strings "$$gov" | grep -q "set_controller"; then \
		echo "FAIL: governance.wasm exports test-only ABI 'set_controller'"; \
		echo "  The governance/testing feature leaked into the deployable build."; \
		exit 1; \
	fi; \
	echo "OK   governance.wasm exports no test-only ABI"
	@pa="$(DEPLOY_DIR)/price_aggregator.wasm"; \
	if [ ! -f "$$pa" ]; then echo "price-aggregator deploy WASM missing: $$pa"; exit 1; fi; \
	if strings "$$pa" | grep -Eq 'seed_oracle(_config)?'; then \
		echo "FAIL: price_aggregator.wasm exports test-only ABI 'seed_oracle' (or legacy seed_oracle_config)"; \
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
























MUTANTS_RUN_ARGS_UNSHARDED = $(MUTANTS_JOB_ARGS) \
	$(MUTANTS_JOBSERVER_ARGS) $(MUTANTS_TEST_TOOL_ARGS) \
	$(MUTANTS_FILTER) $(MUTANTS_EXTRA_ARGS)
MUTANTS_RUN_ARGS = $(MUTANTS_SHARD_ARGS) $(MUTANTS_RUN_ARGS_UNSHARDED)
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














define run_mutants_two_pass
	@count=$$(cargo mutants $(1) $(MUTANTS_FILTER) --list | wc -l); \
		[ "$$count" -gt 0 ] || { echo "No mutants matched scope: $(1)"; exit 1; }; \
		echo "Mutation scope: $$count mutants (two-pass)"
	@status=0; \
		$(MUTANTS_ENV) GITHUB_ACTIONS=false cargo mutants $(MUTANTS_RUN_MODE) $(1) $(2) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS_UNSHARDED) \
			|| status=$$?; \
		case $$status in 0|2|3) ;; *) exit $$status;; esac
	$(MUTANTS_ENV) cargo mutants $(MUTANTS_RUN_MODE) --iterate $(1) $(3) \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS)
endef







define run_mutants_three_pass
	@count=$$(cargo mutants $(1) $(MUTANTS_FILTER) --list | wc -l); \
		[ "$$count" -gt 0 ] || { echo "No mutants matched scope: $(1)"; exit 1; }; \
		echo "Mutation scope: $$count mutants (three-pass)"
	@status=0; \
		$(MUTANTS_ENV) GITHUB_ACTIONS=false cargo mutants $(MUTANTS_RUN_MODE) $(1) $(2) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS_UNSHARDED) \
			|| status=$$?; \
		case $$status in 0|2|3) ;; *) exit $$status;; esac
	@if [ -s mutants.out/missed.txt ] || [ -s mutants.out/timeout.txt ]; then \
		status=0; \
		$(MUTANTS_ENV) GITHUB_ACTIONS=false cargo mutants $(MUTANTS_RUN_MODE) --iterate $(1) $(3) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS_UNSHARDED) \
			|| status=$$?; \
		case $$status in 0|2|3) ;; *) exit $$status;; esac; \
	else \
		echo "Native-consumer pass skipped: package tests resolved every mutant"; \
	fi
	@if [ -s mutants.out/missed.txt ] || [ -s mutants.out/timeout.txt ]; then \
		$(MUTANTS_ENV) cargo mutants $(MUTANTS_RUN_MODE) --iterate $(1) $(4) \
			--minimum-test-timeout $(MUTANTS_TIMEOUT) $(MUTANTS_RUN_ARGS); \
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
		--exclude 'contracts/controller/src/views/**',\
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
	$(call run_mutants_two_pass,--package controller --file 'contracts/controller/src/views/**',\
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
	@echo "1/7 Checking Swap Aggregator..."
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
	@echo "2/7 Uploading pool WASM..."
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
	@echo "3/7 Uploading Controller WASM..."
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
	@echo "4/7 Deploying Governance..."
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
	@echo "5/7 Deploying Controller via governance..."
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
	@echo "6/7 Deploying Price Aggregator via governance..."
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
	@echo "7/7 Deploying central pool via governance timelock..."
	@POOL=$$(NETWORK=$(NETWORK) SIGNER=$(SIGNER) bash $(CONFIG_DIR)/script.sh deployPool $$(cat $(POOL_WASM_HASH_FILE)) | tail -n1 | tr -d '"'); \
	if [ -z "$$POOL" ]; then echo "deployPool returned no address"; exit 1; fi; \
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
	claimRevenueAll deployPool updateDelay \
	acceptAggregatorOwnership acceptOracleAdapterOwnership
POSITIONAL_MARKET_ACTIONS := createMarket updateMarketParams \
	configureMarketOracle \
	editOracleTolerance \
	getPrice getMarket getIndex \
	getOracle getReflector \
	getUtilisation getReserves getSupplied getBorrowed getDepositRate getBorrowRate \
	getRevenue getSyncData
POSITIONAL_ID_ACTIONS := addSpoke getSpoke createHub removeSpoke \
	executeOp cancelOp opState awaitOp transferGovOwnership disableTokenOracle \
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






usage:
	@echo "Stellar Lending Protocol"
	@echo ""
	@echo "  make help                          Full command reference"
	@echo ""
	@echo "Develop:"
	@echo "  make build | test | clippy | fmt | coverage"
	@echo ""
	@echo "Deploy & operate (network = testnet | mainnet):"
	@echo "  make <network> <action> [args]"
	@echo ""
	@echo "  make testnet setup                 Deploy + configure + unpause (full bootstrap)"
	@echo "  make testnet resume                Re-run config phases after a partial failure"
	@echo "  make testnet validateConfigs       Cross-check markets/spokes/networks JSON"
	@echo "  make testnet listOps               Governance ops + live state (pending/executed)"
	@echo "  make testnet info                  Deployed addresses + oracle wiring summary"
	@echo ""
	@echo "Ops: 'make help' lists every action; configs live under configs/."


help:
	@echo "Stellar Lending Protocol Makefile"
	@echo ""
	@echo "Build & Test:"
	@echo "  make build              Build all contracts (WASM)"
	@echo "  make optimize           Build + optimize WASM binaries"
	@echo "  make deploy-artifacts   Optimized WASM for mainnet ($(DEPLOY_DIR))"
	@echo "  make wasm-size-check    Build deploy artifacts + enforce size budget"
	@echo "  make integration-wasm   Deploy-sized WASM + mocks for testnet harness"
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
	@echo "  make fmt                Format code"
	@echo "  make clippy             Lint all targets with warnings denied"
	@echo "  make clean              Clean artifacts"
	@echo ""
	@echo "Deep verification (security-critical paths):"
	@echo "  make miri-all           Miri UB checks on pure-i128 math (common/pool/controller)"
	@echo "  make fuzz               libFuzzer math primitives (FUZZ_TIME=60)"
	@echo "  make fuzz-contract      libFuzzer contract-level flows (flow_e2e, pool_native, ...)"
	@echo "  make proptest           Contract properties (tuned defaults; override PROPTEST_CASES=N)"
	@echo "  make mutants            Full non-overlapping mutation suite (common/pool/governance/controller)"
	@echo "  make mutants-math       Focused local mutation run (also -rates and -pool-interest)"
	@echo "  make scout              Scout audit on host (scout-strict gates incomplete reports)"
	@echo ""
	@echo "Deployment (pattern: make <network> <action>, network = testnet | mainnet):"
	@echo "  make keygen                         Generate deployer key (testnet: friendbot-funded)"
	@echo "  make setup-testnet                  Same as 'make testnet setup'"
	@echo "  make testnet setup                  Full setup (deploy + config + markets/spokes + unpause)"
	@echo "  make testnet resume                 Re-run configure/markets/spokes/unpause (skips deploy)"
	@echo "  make testnet deploy                 Deploy all contracts (no market config)"
	@echo "  make testnet upgradeController      Upgrade controller WASM in-place (timelocked)"
	@echo "  make testnet upgradeGovernance      Upgrade governance WASM in-place (timelocked)"
	@echo "  make testnet upgradePool            Upload + upgrade central pool WASM (timelocked)"
	@echo "  make testnet upgradeAll             upgradePool + upgradeController + unpause path"
	@echo "  AGGREGATOR_CONTRACT=C... ACCUMULATOR_CONTRACT=G... make mainnet setup"
	@echo "    Aggregator = swap router (contract). Accumulator = revenue treasury (wallet or contract)."
	@echo "    ALLOW_MISSING_AGGREGATOR=1 / ALLOW_MISSING_ACCUMULATOR=1 to bootstrap without them (deliberate)."
	@echo "  AWAIT_MAX_WAIT_SECONDS=259200 make mainnet setup   Optional cap for ~48h mainnet timelock await"
	@echo "  DEPLOY_MIN_DELAY=1 make mainnet setup              Bootstrap with 1-ledger delay; raise after:"
	@echo "  make mainnet updateDelay 34560                     Timelocked min-delay increase (cannot shorten)"
	@echo "  make testnet deployFlashReceiver    Deploy flash-loan test receiver"
	@echo "  make testnet fundFlashReceiver      Fund flash receiver with FLASH_MARKET"
	@echo "  make testnet testFlashReceiver      Run flash receiver smoke cases"
	@echo "  make testnet deployAggregator       Deploy swap-router contract; writes networks.json aggregator"
	@echo "    AGGREGATOR_ADMIN=G...              Constructor admin (default: deploying signer)"
	@echo "    Then: make testnet setAggregator   Point the controller at it (timelocked)"
	@echo "  make testnet deployOracleAdapter    Deploy xoxno-oracle-adapter; writes networks.json xoxno_oracle_adapter"
	@echo "    ORACLE_ADAPTER_ADMIN=G...          Constructor admin (default: deploying signer)"
	@echo "    ORACLE_ADAPTER_SIGNERS='[\"G...\"]' Constructor bot-signer set (default: deploying signer alone)"
	@echo "    ORACLE_ADAPTER_THRESHOLD=N         N-of-M aggregation threshold (default: 1)"
	@echo "    Then: make testnet configureOracleFeeds  add_feed for every entry in oracle_feeds.json"
	@echo "  make testnet addOracleSigner <address>   Register a bot wallet's signer address (idempotent)"
	@echo ""
	@echo "  Aggregator + oracle adapter are standalone contracts (NOT governance-owned);"
	@echo "  every verb below is a direct owner-gated stellar contract invoke, no timelock:"
	@echo "    make testnet setAggregatorFee <bps>"
	@echo "    make testnet addAggregatorWhitelist <token>       / removeAggregatorWhitelist <token>"
	@echo "    make testnet addAggregatorReferral <owner> <bps>"
	@echo "    make testnet setAggregatorReferralFee <id> <bps>  / setAggregatorReferralActive <id> <bool>"
	@echo "    make testnet setAggregatorReferralOwner <id> <new_owner>"
	@echo "    make testnet claimAggregatorAdminFees <recipient> <token...>"
	@echo "    make testnet sweepAggregatorBalance <recipient> <token...>"
	@echo "    make testnet upgradeAggregator                    Build + upload + upgrade in place"
	@echo "    make testnet upgradeOracleAdapter                 Wasm only (build+upload+upgrade)"
	@echo "    SIGNER=ledger make mainnet upgradeOracleAdapterFull"
	@echo "      Full cutover: Wasm + configureOracleWindows (age/stale/skew from oracle_feeds.json)"
	@echo "      + reconfigureOracleFeeds (remove_feed then add_feed per feed) + verify getters"
	@echo "    make testnet reconfigureOracleFeeds               remove+add feeds only"
	@echo "    make testnet configureOracleWindows               age + stale + relative skew from JSON"
	@echo "    make testnet setOracleRelativeSkew <secs>         One-off skew setter"
	@echo "    make testnet verifyOracleAdapterWindows           Print live window getters"
	@echo "    make testnet finalizeOracleAdapterUpgrade         Windows + reconfigure (no Wasm)"
	@echo "  Ownership handoff (both are OZ Ownable, two-step transfer -> accept):"
	@echo "    make testnet transferAggregatorOwnership <new_owner> <live_until_ledger>"
	@echo "    SIGNER=ledger make testnet acceptAggregatorOwnership       Run as the NEW owner"
	@echo "    make testnet transferOracleAdapterOwnership <new_owner> <live_until_ledger>"
	@echo "    SIGNER=ledger make testnet acceptOracleAdapterOwnership    Run as the NEW owner"
	@echo "  make testnet info                   Show deployed contract IDs"
	@echo ""
	@echo "Config-driven operations (pattern: make <network> <action> [args]):"
	@echo ""
	@echo "  Validation & governance ops:"
	@echo "    make testnet validateConfigs       Cross-check markets/spokes/networks JSON (also runs pre-setup)"
	@echo "    make testnet listOps               All recorded governance ops with live state"
	@echo "    make testnet executeReady          Execute every recorded op that is Ready"
	@echo "    make testnet opState <op-id>       Unset | Waiting | Ready | Done"
	@echo "    make testnet awaitOp <op-id>       Poll until the op is Ready"
	@echo "    make testnet executeOp <op-id>     Execute one recorded, ready op"
	@echo "    make testnet cancelOp <op-id>      Cancel a pending op (CANCELLER role; single-veto,"
	@echo "                                       can't veto own removal; Recovery ops are non-vetoable)"
	@echo "    make testnet checkDelay            Live timelock delay vs configured target"
	@echo ""
	@echo "  Canceller-council recovery (owner-only, ~30d, non-vetoable):"
	@echo "    No config-driven verb: propose_canceller_reset/execute_canceller_reset take a"
	@echo "    Vec<Address>, which doesn't fit this dispatcher. These are GOVERNANCE entrypoints,"
	@echo "    so use invoke-id against the governance contract (invoke targets the controller):"
	@echo "    make invoke-id CONTRACT_ID=<gov> FN=propose_canceller_reset ARGS='--new_cancellers [\"G...\"] --salt <64-hex>'"
	@echo "    make invoke-id CONTRACT_ID=<gov> FN=execute_canceller_reset ARGS='--executor null --new_cancellers [\"G...\"] --salt <64-hex>'"
	@echo "    AUTO_EXECUTE=0 make testnet <verb> Schedule-only; execute later via executeOp/executeReady"
	@echo "    Re-applying a previously-executed setting is AUTOMATIC for direct verbs"
	@echo "    (fresh salt generation); setupAll*/resume converge and skip Done ops."
	@echo "    REAPPLY_ON_DONE=0 disables auto re-apply; SALT_NONCE=<n> forces a fresh id."
	@echo ""
	@echo "  Markets (writes):"
	@echo "    make testnet createMarket USDC"
	@echo "    make testnet updateMarketParams USDC                       Push max_utilization/rate model from JSON"
	@echo "    make testnet configureMarketOracle USDC   Auto-configures Scaled Ref quotes first"
	@echo "    make testnet configureReferenceOracle BTC set_oracle(PriceKey::Ref) from .references[]"
	@echo "    make testnet setupAllReferenceOracles     All Refs required by Scaled markets"
	@echo "    make testnet editOracleTolerance USDC 500"
	@echo "    make testnet updateIndexes USDC XLM"
	@echo "    make testnet setupAllMarkets       Refs then markets; does not deploy or unpause"
	@echo "    make testnet listMarkets"
	@echo "    make testnet listReferences        PriceKey::Ref oracles from markets.json"
	@echo "    make testnet listOracles           References + per-market oracle wiring from JSON"
	@echo ""
	@echo "  Hubs / Spokes (writes):"
	@echo "    make testnet listHubs"
	@echo "    make testnet createHub 1"
	@echo "    make testnet addSpoke 1"
	@echo "    make testnet addAssetToSpoke 1 USDC"
	@echo "    make testnet editAssetInSpoke 1 USDC"
	@echo "    make testnet removeAssetFromSpoke 1 USDC"
	@echo "    make testnet removeSpoke 1"
	@echo "    make testnet setupAllSpokes        Configure spokes only; does not deploy or unpause"
	@echo "    make testnet setupAll              Configure markets/spokes only; does not deploy or unpause"
	@echo "    make testnet listSpokes"
	@echo ""
	@echo "  Positions (writes):"
	@echo "    make testnet supply USDC 1000000000                  100 USDC at 7 dec, into account 0"
	@echo "    make testnet borrow USDC 100000000 <account_id>      Direct borrow (no swap)"
	@echo "    make testnet withdraw USDC 100000000 <account_id>    Withdraw collateral (0 = all)"
	@echo ""
	@echo "  Strategies (multiply / swap_debt / swap_collateral / repay_debt_with_collateral)"
	@echo "  require an AggregatorSwap JSON from the off-chain quote server. Invoke directly:"
	@echo "    make invoke FN=multiply ARGS='--caller G... --account_id 0 ... --swap @swap.json' NETWORK=testnet"
	@echo ""
	@echo "  Protocol control (writes):"
	@echo "    make testnet pause                              GUARDIAN-immediate (signer = caller)"
	@echo "    make testnet unpause                            Timelocked AdminOperation::Unpause"
	@echo "    make testnet setAggregator                      From networks.json or AGGREGATOR_CONTRACT"
	@echo "    make testnet setAccumulator                     Revenue treasury (required for claimRevenue)"
	@echo "    make testnet disableTokenOracle C...            Timelocked oracle circuit-breaker"
	@echo "    make testnet grantGovRole GAB...XYZ PROPOSER    Roles: PROPOSER|EXECUTOR|CANCELLER|ORACLE|GUARDIAN"
	@echo "    make testnet revokeGovRole GAB...XYZ PROPOSER"
	@echo "    make testnet setPositionLimits 10 10            Timelocked max supply/borrow positions"
	@echo "    make testnet setMinBorrowCollateralUsd 5000000000000000000"
	@echo "    make testnet setPositionManager GAB... true"
	@echo "    make testnet setSpokeLiquidationCurve 1 1020000000000000000 510000000000000000 10000"
	@echo "                                                     Timelocked target_hf/hf_for_max_bonus/bonus_factor_bps"
	@echo "    make testnet transferCtrlOwnership C... <live_until_ledger>"
	@echo "    make testnet transferGovOwnership G... <live_until_ledger>"
	@echo "    make testnet migrateController 2"
	@echo "    make testnet revokeBlendPool C..."
	@echo "    make testnet claimRevenue USDC XLM              Claim revenue one or more markets"
	@echo "    make testnet claimRevenueAll                    Claim revenue for every configured market"
	@echo "    make testnet whitelistBlendPools                Approve Blend pools from configs/$(NETWORK)/blend.json"
	@echo "    make testnet configureSpokeCurves               Apply spoke liquidation_curve overrides from configs/$(NETWORK)/spokes.json"
	@echo "    make testnet approveBlendPools                  Same as whitelistBlendPools"
	@echo ""
	@echo "  Quick views (reads, no signing cost):"
	@echo "    make testnet info                      Deployment addresses"
	@echo "    make testnet hasRole GAB... PROPOSER"
	@echo "    make testnet getPrice USDC             Spot / safe / aggregator prices"
	@echo "    make testnet getMarket USDC            Base spoke-0 listing"
	@echo "    make testnet getSpokeAsset 1 USDC      Live config for ANY spoke (not just base 0)"
	@echo "    make testnet getIndex USDC             Supply / borrow RAY index"
	@echo "    make testnet getAllMarkets"
	@echo "    make testnet getAllIndexes"
	@echo "    make testnet getSpoke 1"
	@echo "    make testnet getHealth 1"
	@echo "    make testnet getAccount 1"
	@echo "    make testnet accountExists 1"
	@echo "    make testnet getCollateralUsd 1"
	@echo "    make testnet getBorrowUsd 1"
	@echo "    make testnet getLtvUsd 1"
	@echo "    make testnet getLiqAvailable 1"
	@echo "    make testnet canLiquidate 1"
	@echo "    make testnet getCollateral 1 XLM"
	@echo "    make testnet getBorrow 1 USDC"
	@echo "    make testnet maxWithdraw 1 USDC        Largest withdraw currently executable"
	@echo "    make testnet maxSupply 1 USDC          Remaining supply-cap headroom"
	@echo "    make testnet maxBorrow 1 USDC          Largest borrow currently executable"
	@echo "    make testnet getLiquidationEstimate 1 USDC 100000000   Seize/repay/refund/bonus estimate"
	@echo "    make testnet getMinBorrowCollateralUsd"
	@echo "    make testnet isBlendPoolApproved C..."
	@echo ""
	@echo "  Pool views (hub-level utilization/reserves/rates; spokes share hub liquidity):"
	@echo "    make testnet getUtilisation USDC | getReserves USDC | getSupplied USDC | getBorrowed USDC"
	@echo "    make testnet getDepositRate USDC | getBorrowRate USDC | getRevenue USDC | getSyncData USDC"
	@echo "    make testnet getBulkIndexes"
	@echo ""
	@echo "  NOTE: no on-chain view exists for is_paused, get_hub, get_aggregator,"
	@echo "  get_accumulator, or get_position_limits (controller stores them without a"
	@echo "  getter). 'info'/'listHubs' show local config for those, not chain truth."
	@echo ""
	@echo "  Oracle probes (debug Oracle V2 wiring):"
	@echo "    make testnet getOracle USDC            Live price components for a market"
	@echo "    make testnet queryReflector CCYOZJ...MJRN63                    decimals + resolution"
	@echo "    make testnet queryReflectorPrice CCYOZJ... other USDC          lastprice"
	@echo "    make testnet queryReflectorTwap  CCYOZJ... other USDC 3        prices history"
	@echo "    make testnet queryReflectorPrice C...DEX... stellar CBIELTK... lastprice on Stellar DEX"
	@echo "    make testnet queryRedStone <feed_id> [adapter]                 RedStone feed price data"
	@echo ""
	@echo "Escape hatches for ad-hoc calls:"
	@echo "    make view FN=get_markets_detailed ARGS='--hub_assets [{\"hub_id\":1,\"asset\":\"C...\"}]' NETWORK=testnet"
	@echo "    make invoke FN=<controller_fn> ARGS='...' NETWORK=testnet"
	@echo "    make invoke-id CONTRACT_ID=C... FN=<fn> ARGS='...' NETWORK=testnet"
	@echo ""
	@echo "Ledger signing (any command):"
	@echo "    SIGNER=ledger make mainnet setupAll"

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
