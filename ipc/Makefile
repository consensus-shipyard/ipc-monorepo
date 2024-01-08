.PHONY: all build test lint license check-fmt check-clippy diagrams install-infra clean-infra

CRATE   := ipc-wallet ipc-provider ipc-api
PACKAGE := $(patsubst %, --package %, $(CRATE))

all: test build

build:
	cargo build --release -p ipc-cli && mkdir -p bin/ && cp target/release/ipc-cli ./bin/ipc-cli

test:
	cargo test --release $(PACKAGE)

# itest:
# 	cargo test -p itest --test checkpoint -- --nocapture

# e2e:
# 	cargo test --release -p ipc_e2e

clean:
	cargo clean

lint: \
	check-fmt \
	check-clippy

install-infra:
	./scripts/install_infra.sh

clean-infra:
	rm -rf ./bin/ipc-infra

check-fmt:
	@# `nightly` is required to support ignore list in rustfmt.toml
	cargo +nightly fmt $(PACKAGE) --check

check-clippy:
	cargo clippy $(PACKAGE) --tests --no-deps -- -D clippy::all

diagrams:
	$(MAKE) -C docs/diagrams

check-diagrams: diagrams
	if git diff --name-only docs/diagrams | grep .png; then \
		echo "There are uncommitted changes to the diagrams"; \
		exit 1; \
	fi
