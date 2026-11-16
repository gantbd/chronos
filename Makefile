.PHONY: check check-public-source check-profile check-local-profile check-worker-profile qualify-native-workers qualify-local-runtime qualify-local-performance measure-local-latency check-all fmt lint test doc integration-check presentations-check qualify-gui package portability dependency-baseline boundaries performance fuzz mutation-smoke formal mutation audit dependencies check-release
.PHONY: check-source-bundle
.PHONY: check-vellum-capture

# Separate cross-project composition; not a core source-release prerequisite.
check-vellum-capture:
	cargo fmt --manifest-path integration/vellum-capture/Cargo.toml -- --check
	cargo test --manifest-path integration/vellum-capture/Cargo.toml --all-targets --locked --offline
	cargo clippy --manifest-path integration/vellum-capture/Cargo.toml --all-targets --locked --offline -- -D warnings

check-source-bundle:
	cargo test -p chronos-client --example source-bundle --locked --offline
	cargo clippy -p chronos-client --example source-bundle --locked --offline -- -D warnings

check: check-public-source fmt lint test doc dependency-baseline check-profile

check-public-source:
	bash scripts/check-public-source.sh

check-profile:
	cargo test -p chronos-client --test workspace_boundary --test dependency_boundary --test profile_baseline --locked --offline

# Native source qualification only: no UI, publication or product-SLO claim.
check-local-profile: check package portability mutation-smoke qualify-local-runtime qualify-local-performance

# Trusted native workers add no bearer transport or execution provider.
check-worker-profile: check-local-profile qualify-native-workers

qualify-native-workers:
	cargo test -p chronos-service --release --lib --test worker_behavior --test atomic_invocation_behavior --locked --offline
	cargo test -p chronos-sdk-embedded --release --example trusted_worker --locked --offline
	cargo run -p chronos-sdk-embedded --release --example trusted_worker --locked --offline

# Optimized synthetic local-process qualification, not installation/distribution.
qualify-local-runtime:
	cargo test -p chronos-standalone --release --test local_behavior --locked --offline

# Approved provisional reference-host gates; separate from ordinary correctness.
qualify-local-performance:
	cargo test -p chronos-standalone --release --test local_behavior local_candidate_ --locked --offline -- --ignored --nocapture --test-threads=1

# Explicit calibration only; deliberately excluded from pass/fail candidate gates.
measure-local-latency:
	cargo test -p chronos-standalone --release --test local_behavior local_request_latency_measurement --locked --offline -- --ignored --nocapture --test-threads=1

# Compatibility command; integration packages now live in the root workspace.
integration-check: check

check-all: check presentations-check

presentations-check:
	$(MAKE) -C ../chronos-tui check
	$(MAKE) -C ../chronos-gui-macos check

# Opt-in: creates one native window and a synthetic private service only.
qualify-gui:
	$(MAKE) -C ../chronos-gui-macos qualify-desktop

fmt:
	cargo fmt --all -- --check
	sh -n scripts/workbench.sh
	bash -n scripts/mutation-smoke.sh
	bash -n scripts/dependency-baseline.sh

lint:
	cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings

test:
	cargo test --workspace --all-targets --all-features --locked --offline

doc:
	cargo test --workspace --doc --locked --offline
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline

package:
	cargo package -p chronos-engine --allow-dirty --locked --offline

portability:
	cargo check -p chronos-engine --target wasm32-wasip1 --locked --offline
	cargo check -p chronos-engine --target wasm32-wasip2 --locked --offline

dependency-baseline:
	bash scripts/dependency-baseline.sh

boundaries: dependency-baseline check-profile

performance:
	test -n "$(CHRONOS_PERF_BUDGET_MS)"
	CHRONOS_PERF_BUDGET_MS="$(CHRONOS_PERF_BUDGET_MS)" cargo test -p chronos-engine --release --test performance_budget --locked --offline -- --ignored --nocapture --test-threads=1

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

check-release: check-all package portability dependency-baseline mutation-smoke formal mutation audit dependencies
