.PHONY: check fmt lint test doc package portability dependency-baseline performance fuzz mutation-smoke formal mutation audit dependencies check-release

check: fmt lint test doc

fmt:
	cargo fmt --all -- --check
	bash -n scripts/mutation-smoke.sh
	bash -n scripts/dependency-baseline.sh

lint:
	cargo clippy --all-targets --all-features --locked --offline -- -D warnings

test:
	cargo test --all-targets --locked --offline

doc:
	cargo test --doc --locked --offline
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --offline

package:
	cargo package --allow-dirty --locked --offline

portability:
	cargo check --target wasm32-wasip1 --locked --offline
	cargo check --target wasm32-wasip2 --locked --offline

dependency-baseline:
	bash scripts/dependency-baseline.sh

performance:
	test -n "$(CHRONOS_PERF_BUDGET_MS)"
	CHRONOS_PERF_BUDGET_MS="$(CHRONOS_PERF_BUDGET_MS)" cargo test --release --test performance_budget --locked --offline -- --ignored --nocapture --test-threads=1

fuzz:
	command -v cargo-fuzz
	cargo fuzz run checkpoint_decode -- -runs=100000

mutation-smoke:
	bash scripts/mutation-smoke.sh

# Optional named gates deliberately fail when their pinned tools are absent.
formal:
	command -v tlc
	tlc -config formal/LeaseProtocol.cfg formal/LeaseProtocol.tla
	tlc -config formal/CoordinationLifecycle.cfg formal/CoordinationLifecycle.tla

mutation:
	command -v cargo-mutants
	cargo mutants

audit:
	cargo audit

dependencies:
	cargo deny check

check-release: check package portability dependency-baseline mutation-smoke formal mutation audit dependencies
