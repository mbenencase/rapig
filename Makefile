.PHONY: build release


release:
	cargo build --release

test:
	cargo test

run:
	cargo run

check:
	cargo fmt
	cargo clippy
	cargo test
