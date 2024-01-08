# syntax=docker/dockerfile:1

# The goal of this step is to copy the `Cargo.toml` and `Cargo.lock` files _without_ the source code,
# so that we can run a step in `builder` that compiles the dependencies only. To do so we first
# copy the whole codebase then get rid of everything except the dependencies and do a build.
FROM --platform=$BUILDPLATFORM ubuntu:latest as stripper

WORKDIR /app

# Copy everything, even though we only need Cargo.* artifacts and Rust sources.
# COPY Cargo.toml Cargo.lock ./
COPY . .

# Delete anything other than cargo files: Rust sources, config files, Markdown, etc.
RUN find . -type f \! -name "Cargo.*" | xargs rm -rf

# Construct dummy sources. Add a print to help debug the case if we failed to properly replace the file.
RUN echo "fn main() { println!(\"I'm the dummy.\"); }" > fendermint/fendermint/app/src/main.rs && \
  for crate in $(find . -name "Cargo.toml" | xargs dirname | grep -v infra | grep -v node_modules | grep /); do \
  touch $crate/src/lib.rs; \
  done
