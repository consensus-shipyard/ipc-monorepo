.PHONY: all build test lint license check-fmt check-clippy actor-bundle

BUILTIN_ACTORS_TAG    ?= v11.0.0
BUILTIN_ACTORS_BUNDLE := $(PWD)/builtin-actors/output/bundle.car

IPC_ACTORS_DIR        := $(PWD)/../contracts
IPC_ACTORS_OUT        := $(IPC_ACTORS_DIR)/out
IPC_ACTORS_CODE       := $(shell find $(IPC_ACTORS_DIR) -type f \( -name "*.sol" \))
IPC_ACTORS_GEN        := .contracts-gen

FENDERMINT_CODE       := $(shell find . -type f \( -name "*.rs" -o -name "Cargo.toml" \) | grep -v target)

CRATE   := $(shell cargo test -p 2>&1 | grep fendermint)
PACKAGE := $(patsubst %, --package %, $(CRATE))

# Override PROFILE env var to choose between `local | ci`
PROFILE ?= local

# Set to `--push` to push the multiarch image during the build.
# Leave on `--load` for local build, but it only works for a single platform.
BUILDX_STORE ?= --load
# Set according to what kind of `--platform` and `--cache` to use.
# Leave empty for local builds, then the platform matches the local one.
BUILDX_FLAGS ?=
# Set to the `<repo>/<image>:<tag>` label the image.
BUILDX_TAG   ?= fendermint:latest

all: test build diagrams

diagrams:
	make -C docs/diagrams diagrams

build: $(IPC_ACTORS_GEN) | protoc
	cargo build --release

install: $(IPC_ACTORS_GEN)
	cargo install --locked --path fendermint/app

# Using --release for testing because wasm can otherwise be slow.
test: $(BUILTIN_ACTORS_BUNDLE) $(IPC_ACTORS_GEN)
	FM_BUILTIN_ACTORS_BUNDLE=$(BUILTIN_ACTORS_BUNDLE) \
	FM_CONTRACTS_DIR=$(IPC_ACTORS_OUT) \
	cargo test --release $(PACKAGE)

e2e: docker-build | cargo-make
	cd fendermint/testing/smoke-test && cargo make --profile $(PROFILE)
	cd fendermint/testing/snapshot-test && cargo make --profile $(PROFILE)

clean:
	cargo clean
	rm $(BUILTIN_ACTORS_BUNDLE)

lint: \
	check-fmt \
	check-clippy

check-fmt:
	@# `nightly` is required to support ignore list in rustfmt.toml
	cargo +nightly fmt $(PACKAGE) --check

check-clippy: $(IPC_ACTORS_GEN)
	@# --all actually runs on the whole workspace but since fendermint depends on all, it's okay.
	@# We could have a separate top level job that does `fmt` and `clippy` check on the whole project.
	cargo clippy $(PACKAGE) --no-deps --tests -- -D clippy::all

docker-deps: $(BUILTIN_ACTORS_BUNDLE) $(IPC_ACTORS_GEN)
	rm -rf docker/.artifacts
	mkdir -p docker/.artifacts/contracts
	cp -r $(IPC_ACTORS_OUT)/* docker/.artifacts/contracts
	cp $(BUILTIN_ACTORS_BUNDLE) docker/.artifacts

# To use `buildx` locally to produce multiplatform images, one needs to run `docker buildx create --use`.
# After that it looks like even the regular docker build needs the `--load` parameter, which hopefully
# it can handle for anyone with `DOCKER_BUILDKIT=1`.
docker-build: docker-deps $(FENDERMINT_CODE)
	if [ "$(PROFILE)" = "ci" ]; then \
		cat docker/stripper.Dockerfile docker/builder.ci.Dockerfile docker/runner.Dockerfile > docker/Dockerfile ; \
		docker buildx build \
			$(BUILDX_STORE) \
			$(BUILDX_FLAGS) \
			-f docker/Dockerfile \
			-t $(BUILDX_TAG) $(PWD)/..; \
	else \
		cat docker/stripper.Dockerfile docker/builder.local.Dockerfile docker/runner.Dockerfile > docker/Dockerfile ; \
		DOCKER_BUILDKIT=1 \
		docker build \
			$(BUILDX_STORE) \
			-f docker/Dockerfile \
			-t fendermint:latest $(PWD)/..; \
	fi


# Build a bundle CAR; this is so we don't have to have a project reference,
# which means we are not tied to the release cycle of both FVM _and_ actors;
# so long as they work together.
actor-bundle: $(BUILTIN_ACTORS_BUNDLE)

compile-abi: $(IPC_ACTORS_GEN)

# Download a released builtin-actors bundle CAR file.
$(BUILTIN_ACTORS_BUNDLE):
	mkdir -p $(dir $@)
	curl -L -o $@ https://github.com/filecoin-project/builtin-actors/releases/download/$(BUILTIN_ACTORS_TAG)/builtin-actors-mainnet.car

# Regenerate the ABI artifacts if we don't have them already, or they changed.
$(IPC_ACTORS_GEN): $(IPC_ACTORS_CODE)
		cd $(IPC_ACTORS_DIR) && make compile-abi
		touch $@

.PHONY: protoc
protoc:
	@if [ -z "$(shell which protoc)" ]; then \
		echo "Please install the Protobuf Compiler. See https://grpc.io/docs/protoc-installation/"; \
		exit 1; \
	fi

.PHONY: cargo-make
cargo-make:
	@if [ -z "$(shell cargo --list | grep make)" ]; then \
		cargo install cargo-make; \
	fi
