.PHONY: build
build: 
	( ./scripts/release_cli.sh )

.PHONY: build-debug
build-debug:
	(cargo build && cp target/debug/cli auto-jlp)
.PHONY: fmt
fmt:
	( find -type f -name "*.rs" -not -path "*target*" -not -path "*vendor*" -exec rustfmt --edition 2021 {} \;)

lint:
	cargo +nightly clippy --fix -Z unstable-options --release --all --broken-code

.PHONY: do-lint
do-lint: lint