set shell := ["bash", "-euo", "pipefail", "-c"]

check: fmt clippy test docs

fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

docs:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

bench:
	cargo bench --workspace
